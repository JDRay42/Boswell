//! The change log behind the feed (change feed design §3, ADR-024).
//!
//! The store's own write paths append here, inside a savepoint shared with the
//! change itself, so the log can neither miss a change nor record one that
//! rolled back. Callers never append: a call site that forgot would be a silent
//! hole in the feed.

use crate::{SqliteStore, StoreError};
use boswell_domain::{Change, ChangeEvent, EntityKind, FeedFilter, FeedPage, Tier};
use rusqlite::{params, OptionalExtension};
use std::time::{SystemTime, UNIX_EPOCH};

/// The longest summary an event keeps, in characters.
pub const SUMMARY_CHARS: usize = 160;

/// Rows scanned per feed read before giving up on filling a page. A narrow
/// filter over a busy log returns a short page and a cursor past what it
/// scanned, so the next read carries on rather than starting over.
const SCAN_LIMIT: usize = 5_000;

/// An event about to be appended; the store assigns `seq` and `at`.
pub(crate) struct NewEvent<'a> {
    pub namespace: &'a str,
    pub kind: EntityKind,
    pub entity_id: String,
    pub change: Change,
    pub tier_before: Option<Tier>,
    pub tier_after: Option<Tier>,
    pub subject: Option<&'a str>,
    pub summary: Option<String>,
    pub author: Option<&'a str>,
}

/// Cut `text` to [`SUMMARY_CHARS`], marking the cut.
pub(crate) fn summarize(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() <= SUMMARY_CHARS {
        text.to_string()
    } else {
        let kept: String = text.chars().take(SUMMARY_CHARS - 1).collect();
        format!("{}…", kept)
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl SqliteStore {
    /// Run `f` inside a savepoint, committing what it did only if it succeeds.
    ///
    /// A savepoint rather than a transaction because savepoints nest: a write
    /// path that calls another write path (collection re-parenting edges, a
    /// stamped write upserting a procedure) still commits or rolls back as one.
    pub(crate) fn atomically<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        self.conn.execute_batch("SAVEPOINT boswell_write")?;
        match f(self) {
            Ok(value) => {
                self.conn.execute_batch("RELEASE boswell_write")?;
                Ok(value)
            }
            Err(e) => {
                // Roll back what `f` did, then close the savepoint. If the
                // rollback itself fails the original error is still the one
                // worth reporting.
                let _ = self
                    .conn
                    .execute_batch("ROLLBACK TO boswell_write; RELEASE boswell_write");
                Err(e)
            }
        }
    }

    /// Append one event to the log.
    pub(crate) fn append_event(&self, event: NewEvent<'_>) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO change_events
                (at, namespace, kind, entity_id, change, tier_before, tier_after,
                 subject, summary, author)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                now_ms() as i64,
                event.namespace,
                event.kind.as_str(),
                event.entity_id,
                event.change.as_str(),
                event.tier_before.map(|t| t.as_str()),
                event.tier_after.map(|t| t.as_str()),
                event.subject,
                event.summary,
                event.author,
            ],
        )?;
        Ok(())
    }

    /// Record `author` on the `created` event of an object, if that event has
    /// none yet. Authorship reaches the store after the write that creates the
    /// object, so this is how the creating event learns who made it.
    pub(crate) fn attribute_created_event(
        &self,
        kind: EntityKind,
        entity_id: &str,
        author: &str,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE change_events SET author = ?1
             WHERE kind = ?2 AND entity_id = ?3 AND change = 'created' AND author IS NULL",
            params![author, kind.as_str(), entity_id],
        )?;
        Ok(())
    }

    /// Strip what a forgotten object said from every event about it.
    pub(crate) fn scrub_events(&self, kind: EntityKind, entity_id: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE change_events SET subject = NULL, summary = NULL
             WHERE kind = ?1 AND entity_id = ?2",
            params![kind.as_str(), entity_id],
        )?;
        Ok(())
    }

    /// Read the feed: events after `since` that pass `filter`, oldest first.
    ///
    /// `since: None` starts from the oldest retained event. `filter.namespace`
    /// is applied as namespace-and-children, never as a raw prefix.
    pub fn feed(&self, since: Option<u64>, filter: &FeedFilter) -> Result<FeedPage, StoreError> {
        let since = since.unwrap_or(0);
        let pruned_through = self.pruned_through()?;
        let limit = filter.limit.max(1);

        let mut stmt = self.conn.prepare(
            "SELECT seq, at, namespace, kind, entity_id, change, tier_before, tier_after,
                    subject, summary, author
             FROM change_events WHERE seq > ?1 ORDER BY seq LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![since as i64, SCAN_LIMIT as i64], |row| {
            Ok((
                row.get::<_, i64>(0)? as u64,
                row.get::<_, i64>(1)? as u64,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, Option<String>>(9)?,
                row.get::<_, Option<String>>(10)?,
            ))
        })?;

        let mut page = FeedPage {
            events: Vec::new(),
            next: since,
            gone: since < pruned_through,
        };
        for row in rows {
            let (
                seq,
                at,
                namespace,
                kind,
                entity_id,
                change,
                before,
                after,
                subject,
                summary,
                author,
            ) = row?;
            let event = ChangeEvent {
                seq,
                at,
                namespace,
                kind: EntityKind::parse(&kind).ok_or_else(|| {
                    StoreError::InvalidData(format!("unknown event kind '{}'", kind))
                })?,
                entity_id,
                change: Change::parse(&change).ok_or_else(|| {
                    StoreError::InvalidData(format!("unknown event change '{}'", change))
                })?,
                tier_before: before.as_deref().and_then(Tier::parse),
                tier_after: after.as_deref().and_then(Tier::parse),
                subject,
                summary,
                author,
            };
            page.next = seq;
            if filter.admits(&event) {
                page.events.push(event);
                if page.events.len() >= limit {
                    break;
                }
            }
        }
        Ok(page)
    }

    /// The newest event's cursor, or 0 for an empty log. A reader that wants
    /// only what happens from now on starts here.
    pub fn feed_head(&self) -> Result<u64, StoreError> {
        let head: Option<i64> =
            self.conn
                .query_row("SELECT MAX(seq) FROM change_events", [], |row| row.get(0))?;
        Ok(head.unwrap_or(0) as u64)
    }

    /// Drop events older than `before_ms` (Unix ms), returning how many went.
    ///
    /// Records the highest pruned `seq`, so a reader whose cursor predates it
    /// is told its page is incomplete rather than silently missing events.
    pub fn prune_change_events(&mut self, before_ms: u64) -> Result<usize, StoreError> {
        self.atomically(|s| {
            let highest: Option<i64> = s.conn.query_row(
                "SELECT MAX(seq) FROM change_events WHERE at < ?1",
                // SQLite integers are signed; a cutoff past i64::MAX means "all".
                params![before_ms.min(i64::MAX as u64) as i64],
                |row| row.get(0),
            )?;
            let Some(highest) = highest else {
                return Ok(0);
            };
            let removed = s.conn.execute(
                "DELETE FROM change_events WHERE seq <= ?1",
                params![highest],
            )?;
            s.conn.execute(
                "INSERT INTO change_feed_meta (key, value) VALUES ('pruned_through', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = MAX(value, excluded.value)",
                params![highest],
            )?;
            Ok(removed)
        })
    }

    fn pruned_through(&self) -> Result<u64, StoreError> {
        let value: Option<i64> = self
            .conn
            .query_row(
                "SELECT value FROM change_feed_meta WHERE key = 'pruned_through'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        Ok(value.unwrap_or(0) as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use boswell_domain::traits::ClaimStore;
    use boswell_domain::{
        BodyFormat, Claim, ClaimId, Goal, GoalId, GraphIntegrity, Procedure, ProcedureId,
        ProcedureSource, Relationship, RelationshipType,
    };

    fn store() -> SqliteStore {
        SqliteStore::new(":memory:", false, 0).unwrap()
    }

    fn claim(ns: &str, subject: &str) -> Claim {
        Claim::new(
            ClaimId::new(),
            ns.into(),
            subject.into(),
            "provides".into(),
            "ISO 8601".into(),
            (0.8, 0.9),
            "task".into(),
            100,
        )
    }

    fn all(store: &SqliteStore) -> Vec<ChangeEvent> {
        store
            .feed(
                None,
                &FeedFilter {
                    limit: 1_000,
                    ..FeedFilter::default()
                },
            )
            .unwrap()
            .events
    }

    fn procedure(supersedes: Option<ProcedureId>) -> Procedure {
        Procedure {
            id: ProcedureId::new(),
            namespace: "team".into(),
            name: "use-date-parser".into(),
            version: 1,
            supersedes,
            is_current: true,
            source: ProcedureSource::Authored,
            goal: "goal:team/parse-dates".into(),
            intent: "parse a timestamp".into(),
            tags: vec![],
            parameters: vec![],
            preconditions: vec![],
            required_tools: vec![],
            postconditions: vec![],
            est_duration_sec: None,
            usage_notes: String::new(),
            context_tags: vec![],
            body_format: BodyFormat::Prose,
            content_type: "text/plain".into(),
            body: "call it".into(),
            tier: Tier::Task,
            use_count: 0,
            success_count: 0,
            failure_count: 0,
            unknown_count: 0,
            last_used_at: None,
            created_at: 0,
            updated_at: 0,
            stale_at: None,
        }
    }

    #[test]
    fn asserting_a_claim_logs_its_creation_and_stamping_names_the_author() {
        let mut s = store();
        let id = s.assert_claim(claim("team", "DateParser")).unwrap();
        s.stamp_claim(id, "jd/sub:impl-2", 100).unwrap();
        s.stamp_claim(id, "auditor", 100).unwrap();

        let events = all(&s);
        assert_eq!(events.len(), 1);
        let e = &events[0];
        assert_eq!(e.change, Change::Created);
        assert_eq!(e.kind, EntityKind::Claim);
        assert_eq!(e.subject.as_deref(), Some("DateParser"));
        assert_eq!(e.summary.as_deref(), Some("DateParser provides ISO 8601"));
        assert_eq!(
            e.author.as_deref(),
            Some("jd/sub:impl-2"),
            "the first author made it"
        );
        assert_eq!(e.independence_root(), Some("jd"));
    }

    #[test]
    fn tier_moves_log_their_direction_and_a_no_op_logs_nothing() {
        let mut s = store();
        let id = s.assert_claim(claim("team", "X")).unwrap();
        s.update_claim_tier(id, "project").unwrap();
        s.update_claim_tier(id, "project").unwrap();
        s.update_claim_tier(id, "ephemeral").unwrap();

        let changes: Vec<_> = all(&s).iter().map(|e| e.change).collect();
        assert_eq!(
            changes,
            vec![Change::Created, Change::Climbed, Change::Fell]
        );
    }

    #[test]
    fn a_new_contradiction_logs_both_ends_once() {
        let mut s = store();
        let a = s.assert_claim(claim("team", "A")).unwrap();
        let b = s.assert_claim(claim("team", "B")).unwrap();
        let rel = Relationship::new(a, b, RelationshipType::Contradicts, 1.0, 100);
        s.add_relationship(rel.clone()).unwrap();
        s.add_relationship(rel).unwrap();
        s.add_relationship(Relationship::new(
            a,
            b,
            RelationshipType::Supports,
            1.0,
            100,
        ))
        .unwrap();

        let contradicted: Vec<_> = all(&s)
            .into_iter()
            .filter(|e| e.change == Change::Contradicted)
            .map(|e| e.subject.unwrap())
            .collect();
        assert_eq!(contradicted, vec!["A", "B"]);
    }

    /// Forget is a real delete, and the log must not keep what it removed.
    #[test]
    fn forgetting_strips_every_earlier_event_and_logs_the_forget() {
        let mut s = store();
        let id = s.assert_claim(claim("team", "Secret")).unwrap();
        s.update_claim_tier(id, "project").unwrap();
        s.delete_claim(id).unwrap();

        let events = all(&s);
        assert_eq!(events.len(), 3);
        assert!(events
            .iter()
            .all(|e| e.subject.is_none() && e.summary.is_none()));
        let forgotten = events.last().unwrap();
        assert_eq!(forgotten.change, Change::Forgotten);
        assert_eq!(forgotten.tier_before, Some(Tier::Project));
    }

    #[test]
    fn a_new_procedure_version_logs_the_old_one_superseded() {
        let mut s = store();
        let v1 = procedure(None);
        s.upsert_procedure(&v1).unwrap();
        let mut v2 = procedure(Some(v1.id));
        v2.version = 2;
        s.upsert_procedure(&v2).unwrap();
        // Re-saving with new counters is not news.
        v2.success_count = 5;
        s.upsert_procedure(&v2).unwrap();

        let log: Vec<_> = all(&s)
            .into_iter()
            .map(|e| (e.entity_id, e.change))
            .collect();
        assert_eq!(
            log,
            vec![
                (v1.id.to_string(), Change::Created),
                (v2.id.to_string(), Change::Created),
                (v1.id.to_string(), Change::Superseded),
            ]
        );
    }

    #[test]
    fn collecting_a_goal_logs_it_forgotten() {
        let mut s = store();
        let goal = Goal {
            id: GoalId::new(),
            namespace: "team".into(),
            name: "parse-dates".into(),
            intent: "parse dates".into(),
            definition_of_done: vec![],
            tier: Tier::Task,
            created_at: 0,
            updated_at: 0,
            stale_at: None,
        };
        s.upsert_goal(&goal).unwrap();
        s.collect_goal(goal.id, GraphIntegrity::CascadeAndOrphan, 0)
            .unwrap();

        let events = all(&s);
        assert_eq!(events[1].change, Change::Forgotten);
        assert!(events[0].summary.is_none(), "collection scrubs like Forget");
    }

    #[test]
    fn a_failed_write_logs_nothing() {
        let mut s = store();
        let c = claim("team", "X");
        s.assert_claim(c.clone()).unwrap();
        // Same id again: the insert is refused, and its event must go with it.
        assert!(s.assert_claim(c).is_err());
        assert_eq!(all(&s).len(), 1);
    }

    #[test]
    fn the_cursor_resumes_and_a_page_stops_at_its_limit() {
        let mut s = store();
        for i in 0..5 {
            s.assert_claim(claim("team", &format!("S{}", i))).unwrap();
        }
        let filter = FeedFilter {
            limit: 2,
            ..FeedFilter::default()
        };
        let first = s.feed(None, &filter).unwrap();
        assert_eq!(first.events.len(), 2);
        let second = s.feed(Some(first.next), &filter).unwrap();
        assert_eq!(second.events[0].subject.as_deref(), Some("S2"));
        assert_eq!(s.feed_head().unwrap(), 5);
    }

    /// A filtered-out event still advances the cursor, so a narrow reader does
    /// not rescan the same rows forever.
    #[test]
    fn filtered_events_still_advance_the_cursor() {
        let mut s = store();
        s.assert_claim(claim("other", "X")).unwrap();
        let page = s
            .feed(
                None,
                &FeedFilter {
                    namespace: Some("team".into()),
                    limit: 10,
                    ..FeedFilter::default()
                },
            )
            .unwrap();
        assert!(page.events.is_empty());
        assert_eq!(page.next, 1);
    }

    #[test]
    fn a_cursor_behind_the_pruned_window_is_told_so() {
        let mut s = store();
        s.assert_claim(claim("team", "Old")).unwrap();
        let removed = s.prune_change_events(u64::MAX).unwrap();
        assert_eq!(removed, 1);
        s.assert_claim(claim("team", "New")).unwrap();

        let filter = FeedFilter {
            limit: 10,
            ..FeedFilter::default()
        };
        let stale = s.feed(Some(0), &filter).unwrap();
        assert!(stale.gone);
        assert_eq!(stale.events[0].subject.as_deref(), Some("New"));
        assert!(!s.feed(Some(1), &filter).unwrap().gone);
    }
}
