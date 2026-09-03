//! SQLite persistence and retrieval for [`Procedure`] (procedural memory).
//!
//! This module adds procedure methods to [`SqliteStore`] without touching the
//! existing claim behavior. Scalar, filterable fields are stored as columns; the
//! collection-valued signature fields (tags, parameters, preconditions, ...) are
//! stored as JSON text. Domain types stay dependency-light (per ADR-004), so the
//! JSON shape is owned here by private serde DTOs rather than by the domain.
//!
//! Retrieval mirrors the `expand`/rank contract of
//! `docs/architecture/15-procedural-memory.md` §4.1 for the procedure-sibling
//! slice: filter by goal/intent (and by `is_current`), drop candidates whose hard
//! preconditions do not hold against the claim store, then rank the survivors by
//! derived effectiveness.

use crate::{SqliteStore, StoreError};
use boswell_domain::traits::{ClaimQuery, ClaimStore};
use boswell_domain::{
    BodyFormat, ClaimMatch, Expect, OutcomeReport, Parameter, Precondition, PreconditionCheck,
    Procedure, ProcedureId, ProcedureQuery, ProcedureSource, ReportEffect, Tier,
};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

// --- JSON DTOs for the collection-valued columns -------------------------------

#[derive(Serialize, Deserialize)]
struct ParameterDto {
    name: String,
    #[serde(rename = "type")]
    type_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    desc: Option<String>,
}

impl From<&Parameter> for ParameterDto {
    fn from(p: &Parameter) -> Self {
        Self {
            name: p.name.clone(),
            type_name: p.type_name.clone(),
            default: p.default.clone(),
            desc: p.desc.clone(),
        }
    }
}

impl From<ParameterDto> for Parameter {
    fn from(d: ParameterDto) -> Self {
        Self {
            name: d.name,
            type_name: d.type_name,
            default: d.default,
            desc: d.desc,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct ClaimMatchDto {
    subject: String,
    predicate: String,
    object: String,
}

#[derive(Serialize, Deserialize)]
struct PreconditionCheckDto {
    #[serde(rename = "match")]
    match_pattern: ClaimMatchDto,
    min_confidence: f64,
    expect: String,
}

#[derive(Serialize, Deserialize)]
struct PreconditionDto {
    kind: String,
    description: String,
    check: PreconditionCheckDto,
}

impl From<&Precondition> for PreconditionDto {
    fn from(p: &Precondition) -> Self {
        Self {
            kind: p.kind.clone(),
            description: p.description.clone(),
            check: PreconditionCheckDto {
                match_pattern: ClaimMatchDto {
                    subject: p.check.match_pattern.subject.clone(),
                    predicate: p.check.match_pattern.predicate.clone(),
                    object: p.check.match_pattern.object.clone(),
                },
                min_confidence: p.check.min_confidence,
                expect: p.check.expect.as_str().to_string(),
            },
        }
    }
}

impl PreconditionDto {
    fn into_domain(self) -> Result<Precondition, StoreError> {
        let expect = Expect::parse(&self.check.expect).ok_or_else(|| {
            StoreError::InvalidData(format!(
                "Unknown precondition expect: {}",
                self.check.expect
            ))
        })?;
        Ok(Precondition {
            kind: self.kind,
            description: self.description,
            check: PreconditionCheck {
                match_pattern: ClaimMatch {
                    subject: self.check.match_pattern.subject,
                    predicate: self.check.match_pattern.predicate,
                    object: self.check.match_pattern.object,
                },
                min_confidence: self.check.min_confidence,
                expect,
            },
        })
    }
}

// --- (de)serialization helpers -------------------------------------------------

fn to_json<T: Serialize>(value: &T) -> Result<String, StoreError> {
    serde_json::to_string(value)
        .map_err(|e| StoreError::InvalidData(format!("JSON serialize failed: {}", e)))
}

fn from_json<T: for<'de> Deserialize<'de>>(text: &str) -> Result<T, StoreError> {
    serde_json::from_str(text)
        .map_err(|e| StoreError::InvalidData(format!("JSON deserialize failed: {}", e)))
}

impl SqliteStore {
    /// Convert a `ProcedureId` to its 16-byte big-endian storage form.
    fn procedure_id_to_bytes(id: ProcedureId) -> Vec<u8> {
        id.value().to_be_bytes().to_vec()
    }

    /// Convert stored bytes back into a `ProcedureId`.
    fn bytes_to_procedure_id(bytes: &[u8]) -> Result<ProcedureId, StoreError> {
        if bytes.len() != 16 {
            return Err(StoreError::InvalidData(format!(
                "Expected 16 bytes for ProcedureId, got {}",
                bytes.len()
            )));
        }
        let mut arr = [0u8; 16];
        arr.copy_from_slice(bytes);
        Ok(ProcedureId::from_value(u128::from_be_bytes(arr)))
    }

    /// Insert a procedure, or replace it in place if one with the same id exists.
    ///
    /// This is an idempotent upsert keyed by `id`: refining a technique creates a
    /// *new* row (a new id that `supersedes` the old); this method persists a
    /// single version's current state, including its effectiveness counters.
    ///
    /// Returns the procedure's id.
    pub fn upsert_procedure(&mut self, procedure: &Procedure) -> Result<ProcedureId, StoreError> {
        let id_bytes = Self::procedure_id_to_bytes(procedure.id);
        let supersedes_bytes = procedure.supersedes.map(Self::procedure_id_to_bytes);

        let tags = to_json(&procedure.tags)?;
        let parameters = to_json(
            &procedure
                .parameters
                .iter()
                .map(ParameterDto::from)
                .collect::<Vec<_>>(),
        )?;
        let preconditions = to_json(
            &procedure
                .preconditions
                .iter()
                .map(PreconditionDto::from)
                .collect::<Vec<_>>(),
        )?;
        let required_tools = to_json(&procedure.required_tools)?;
        let postconditions = to_json(&procedure.postconditions)?;
        let context_tags = to_json(&procedure.context_tags)?;

        self.conn.execute(
            "INSERT INTO procedures (
                id, namespace, name, version, supersedes, is_current, source, goal,
                intent, tags, parameters, preconditions, required_tools, postconditions,
                est_duration_sec, usage_notes, context_tags,
                body_format, content_type, body,
                tier, use_count, success_count, failure_count,
                last_used_at, created_at, updated_at, stale_at
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
                ?9, ?10, ?11, ?12, ?13, ?14,
                ?15, ?16, ?17,
                ?18, ?19, ?20,
                ?21, ?22, ?23, ?24,
                ?25, ?26, ?27, ?28
             )
             ON CONFLICT(id) DO UPDATE SET
                namespace = excluded.namespace, name = excluded.name,
                version = excluded.version, supersedes = excluded.supersedes,
                is_current = excluded.is_current, source = excluded.source,
                goal = excluded.goal, intent = excluded.intent, tags = excluded.tags,
                parameters = excluded.parameters, preconditions = excluded.preconditions,
                required_tools = excluded.required_tools,
                postconditions = excluded.postconditions,
                est_duration_sec = excluded.est_duration_sec,
                usage_notes = excluded.usage_notes, context_tags = excluded.context_tags,
                body_format = excluded.body_format, content_type = excluded.content_type,
                body = excluded.body, tier = excluded.tier,
                use_count = excluded.use_count, success_count = excluded.success_count,
                failure_count = excluded.failure_count, last_used_at = excluded.last_used_at,
                created_at = excluded.created_at, updated_at = excluded.updated_at,
                stale_at = excluded.stale_at",
            params![
                &id_bytes,
                &procedure.namespace,
                &procedure.name,
                procedure.version,
                &supersedes_bytes,
                procedure.is_current as i64,
                procedure.source.as_str(),
                &procedure.goal,
                &procedure.intent,
                &tags,
                &parameters,
                &preconditions,
                &required_tools,
                &postconditions,
                procedure.est_duration_sec.map(|d| d as i64),
                &procedure.usage_notes,
                &context_tags,
                procedure.body_format.as_str(),
                &procedure.content_type,
                &procedure.body,
                procedure.tier.as_str(),
                procedure.use_count as i64,
                procedure.success_count as i64,
                procedure.failure_count as i64,
                procedure.last_used_at.map(|t| t as i64),
                procedure.created_at as i64,
                procedure.updated_at as i64,
                procedure.stale_at.map(|t| t as i64),
            ],
        )?;

        Ok(procedure.id)
    }

    /// Fetch a procedure by id, or `None` if it does not exist.
    pub fn get_procedure(&self, id: ProcedureId) -> Result<Option<Procedure>, StoreError> {
        let id_bytes = Self::procedure_id_to_bytes(id);
        self.conn
            .query_row(
                &format!("SELECT {} FROM procedures WHERE id = ?1", PROCEDURE_COLUMNS),
                params![&id_bytes],
                Self::row_to_procedure,
            )
            .optional()?
            .transpose()
    }

    /// Query procedures by goal/intent, filter by hard preconditions, and rank
    /// the survivors by derived effectiveness (design §4.1).
    ///
    /// `now` (Unix ms) is the reference time for the effectiveness recency term.
    /// Ranking is deterministic: effectiveness descending, then success_count
    /// descending, then id ascending as a final tie-break.
    ///
    /// Only current (`is_current`) procedures are returned unless the query sets
    /// `include_superseded`.
    pub fn query_procedures(
        &self,
        query: &ProcedureQuery,
        now: u64,
    ) -> Result<Vec<Procedure>, StoreError> {
        let mut sql = format!("SELECT {} FROM procedures WHERE 1=1", PROCEDURE_COLUMNS);
        let mut sql_params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if !query.include_superseded {
            sql.push_str(" AND is_current = 1");
        }
        if let Some(namespace) = &query.namespace {
            sql.push_str(" AND namespace LIKE ?");
            sql_params.push(Box::new(format!("{}%", namespace)));
        }
        if let Some(goal) = &query.goal {
            sql.push_str(" AND goal = ?");
            sql_params.push(Box::new(goal.clone()));
        }
        if let Some(intent) = &query.intent_contains {
            // TODO(procedural-memory): semantic intent match via the embedding
            // path (ADR-005). Phase 1 uses a case-insensitive substring match.
            sql.push_str(" AND intent LIKE ? ESCAPE '\\'");
            sql_params.push(Box::new(format!("%{}%", like_escape(intent))));
        }

        let mut stmt = self.conn.prepare(&sql)?;
        let param_refs: Vec<&dyn rusqlite::ToSql> = sql_params.iter().map(|p| p.as_ref()).collect();
        let mut candidates = stmt
            .query_map(&param_refs[..], Self::row_to_procedure)?
            .collect::<Result<Vec<Result<Procedure, StoreError>>, rusqlite::Error>>()?
            .into_iter()
            .collect::<Result<Vec<Procedure>, StoreError>>()?;

        // Filter by hard preconditions (resolved against the claim store).
        let mut survivors = Vec::with_capacity(candidates.len());
        for procedure in candidates.drain(..) {
            if self.preconditions_hold(&procedure.preconditions)? {
                survivors.push(procedure);
            }
        }

        // Rank deterministically by effectiveness, then success_count, then id.
        survivors.sort_by(|a, b| {
            b.effectiveness(now)
                .partial_cmp(&a.effectiveness(now))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.success_count.cmp(&a.success_count))
                .then_with(|| a.id.value().cmp(&b.id.value()))
        });

        if let Some(limit) = query.limit {
            survivors.truncate(limit);
        }
        Ok(survivors)
    }

    /// Whether every precondition in `preconditions` holds right now.
    ///
    /// Each precondition's `check` is resolved against the claim store: claims
    /// matching the `(subject, predicate, object)` triple whose lower-bound
    /// confidence is at least `min_confidence` are counted; `Exists` requires at
    /// least one, `Absent` requires none.
    fn preconditions_hold(&self, preconditions: &[Precondition]) -> Result<bool, StoreError> {
        for pc in preconditions {
            if !self.precondition_holds(pc)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Resolve a single precondition against the claim store.
    fn precondition_holds(&self, pc: &Precondition) -> Result<bool, StoreError> {
        // Push the confidence floor down to SQL; the (subject, predicate, object)
        // match is done in-process because ClaimQuery has no such fields today.
        // TODO(procedural-memory): push the triple match into SQL once ClaimQuery
        // grows subject/predicate/object filters (ADR-020 backlog).
        let cq = ClaimQuery {
            min_confidence: Some(pc.check.min_confidence),
            ..ClaimQuery::default()
        };
        let claims = self.query_claims(&cq)?;
        let m = &pc.check.match_pattern;
        let matched = claims
            .iter()
            .any(|c| c.subject == m.subject && c.predicate == m.predicate && c.object == m.object);
        Ok(match pc.check.expect {
            Expect::Exists => matched,
            Expect::Absent => !matched,
        })
    }

    /// Apply an outcome report to a stored procedure's effectiveness counters
    /// (design §3.3), persisting the result.
    ///
    /// Returns `None` if no procedure with `procedure_id` exists, otherwise the
    /// [`ReportEffect`] describing what changed. Attribution rules live in
    /// [`Procedure::apply_report`]; this method only loads, applies, and saves.
    /// `now` is a Unix-ms timestamp.
    pub fn apply_procedure_report(
        &mut self,
        procedure_id: ProcedureId,
        report: &OutcomeReport,
        now: u64,
    ) -> Result<Option<ReportEffect>, StoreError> {
        let Some(mut procedure) = self.get_procedure(procedure_id)? else {
            return Ok(None);
        };
        let effect = procedure.apply_report(report, now);
        self.upsert_procedure(&procedure)?;
        Ok(Some(effect))
    }

    /// Materialize a `Procedure` from a `procedures` row.
    ///
    /// The outer `rusqlite::Result` covers column access; the inner
    /// `Result<_, StoreError>` covers value decoding (ids, JSON, enums), keeping
    /// SQL errors and data-format errors distinct.
    #[allow(clippy::type_complexity)]
    fn row_to_procedure(
        row: &rusqlite::Row<'_>,
    ) -> rusqlite::Result<Result<Procedure, StoreError>> {
        // A closure so we can use `?` on StoreError without leaking it into the
        // rusqlite::Result the callback must return.
        let decode = |row: &rusqlite::Row<'_>| -> Result<Procedure, StoreError> {
            let id_bytes: Vec<u8> = row.get("id")?;
            let id = Self::bytes_to_procedure_id(&id_bytes)?;
            let supersedes_bytes: Option<Vec<u8>> = row.get("supersedes")?;
            let supersedes = supersedes_bytes
                .as_deref()
                .map(Self::bytes_to_procedure_id)
                .transpose()?;

            let source_str: String = row.get("source")?;
            let source = ProcedureSource::parse(&source_str).ok_or_else(|| {
                StoreError::InvalidData(format!("Unknown procedure source: {}", source_str))
            })?;
            let body_format_str: String = row.get("body_format")?;
            let body_format = BodyFormat::parse(&body_format_str).ok_or_else(|| {
                StoreError::InvalidData(format!("Unknown body format: {}", body_format_str))
            })?;
            let tier_str: String = row.get("tier")?;
            let tier = Tier::parse(&tier_str)
                .ok_or_else(|| StoreError::InvalidData(format!("Unknown tier: {}", tier_str)))?;

            let tags: Vec<String> = from_json(&row.get::<_, String>("tags")?)?;
            let parameters: Vec<ParameterDto> = from_json(&row.get::<_, String>("parameters")?)?;
            let preconditions_dto: Vec<PreconditionDto> =
                from_json(&row.get::<_, String>("preconditions")?)?;
            let required_tools: Vec<String> = from_json(&row.get::<_, String>("required_tools")?)?;
            let postconditions: Vec<String> = from_json(&row.get::<_, String>("postconditions")?)?;
            let context_tags: Vec<String> = from_json(&row.get::<_, String>("context_tags")?)?;

            let preconditions = preconditions_dto
                .into_iter()
                .map(PreconditionDto::into_domain)
                .collect::<Result<Vec<_>, _>>()?;

            let est_duration_sec: Option<i64> = row.get("est_duration_sec")?;
            let last_used_at: Option<i64> = row.get("last_used_at")?;
            let stale_at: Option<i64> = row.get("stale_at")?;

            Ok(Procedure {
                id,
                namespace: row.get("namespace")?,
                name: row.get("name")?,
                version: row.get("version")?,
                supersedes,
                is_current: row.get::<_, i64>("is_current")? != 0,
                source,
                goal: row.get("goal")?,
                intent: row.get("intent")?,
                tags,
                parameters: parameters.into_iter().map(Parameter::from).collect(),
                preconditions,
                required_tools,
                postconditions,
                est_duration_sec: est_duration_sec.map(|d| d as u64),
                usage_notes: row.get("usage_notes")?,
                context_tags,
                body_format,
                content_type: row.get("content_type")?,
                body: row.get("body")?,
                tier,
                use_count: row.get::<_, i64>("use_count")? as u64,
                success_count: row.get::<_, i64>("success_count")? as u64,
                failure_count: row.get::<_, i64>("failure_count")? as u64,
                last_used_at: last_used_at.map(|t| t as u64),
                created_at: row.get::<_, i64>("created_at")? as u64,
                updated_at: row.get::<_, i64>("updated_at")? as u64,
                stale_at: stale_at.map(|t| t as u64),
            })
        };
        Ok(decode(row))
    }
}

/// The full column list for `procedures`, selected in a fixed order and read by
/// name in [`SqliteStore::row_to_procedure`].
const PROCEDURE_COLUMNS: &str = "id, namespace, name, version, supersedes, is_current, source, \
     goal, intent, tags, parameters, preconditions, required_tools, postconditions, \
     est_duration_sec, usage_notes, context_tags, body_format, content_type, body, tier, \
     use_count, success_count, failure_count, last_used_at, created_at, updated_at, stale_at";

/// Escape `%`, `_`, and `\` for a `LIKE ... ESCAPE '\'` substring match so that
/// user-supplied intent text matches literally.
fn like_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        if matches!(ch, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use boswell_domain::{Claim, ClaimId, FailureMode, Outcome};

    /// Fixed reference time (Unix ms) used across the tests.
    const NOW: u64 = 1_700_000_000_000;

    fn store() -> SqliteStore {
        SqliteStore::new(":memory:", false, 0).unwrap()
    }

    /// Build a minimal prose procedure under `goal` with `name`.
    fn mk(goal: &str, name: &str) -> Procedure {
        Procedure {
            id: ProcedureId::new(),
            namespace: "person:jd".into(),
            name: name.into(),
            version: 1,
            supersedes: None,
            is_current: true,
            source: ProcedureSource::Authored,
            goal: goal.into(),
            intent: format!("intent for {}", name),
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
            body: format!("body of {}", name),
            tier: Tier::Project,
            use_count: 0,
            success_count: 0,
            failure_count: 0,
            last_used_at: None,
            created_at: NOW,
            updated_at: NOW,
            stale_at: None,
        }
    }

    fn precondition(subject: &str, predicate: &str, object: &str, expect: Expect) -> Precondition {
        Precondition {
            kind: "resource".into(),
            description: format!("{} {} {}", subject, predicate, object),
            check: PreconditionCheck {
                match_pattern: ClaimMatch {
                    subject: subject.into(),
                    predicate: predicate.into(),
                    object: object.into(),
                },
                min_confidence: 0.6,
                expect,
            },
        }
    }

    fn assert_claim(store: &mut SqliteStore, subject: &str, predicate: &str, object: &str) {
        let claim = Claim::new(
            ClaimId::new(),
            "person:jd".into(),
            subject.into(),
            predicate.into(),
            object.into(),
            (0.7, 0.8),
            "project".into(),
            NOW,
        );
        store.assert_claim(claim).unwrap();
    }

    #[test]
    fn upsert_get_roundtrip_preserves_rich_fields() {
        let mut store = store();
        let mut p = mk("goal:person:jd/cook-eggs", "omelette-classic");
        p.tags = vec!["breakfast".into(), "eggs".into()];
        p.parameters = vec![Parameter {
            name: "count".into(),
            type_name: "int".into(),
            default: Some("2".into()),
            desc: Some("how many eggs".into()),
        }];
        p.preconditions = vec![precondition(
            "person:jd",
            "attr:in-pantry",
            "ingredient:eggs",
            Expect::Exists,
        )];
        p.required_tools = vec!["pan".into()];
        p.postconditions = vec!["eggs are cooked".into()];
        p.est_duration_sec = Some(600);
        p.usage_notes = "French-style, buttery.".into();
        p.context_tags = vec!["mood:fancy".into()];
        p.success_count = 22;
        p.failure_count = 1;
        p.last_used_at = Some(NOW);

        let id = store.upsert_procedure(&p).unwrap();
        let got = store.get_procedure(id).unwrap().expect("procedure exists");
        assert_eq!(got, p);
    }

    #[test]
    fn upsert_is_idempotent_replace() {
        let mut store = store();
        let mut p = mk("g", "proc");
        store.upsert_procedure(&p).unwrap();
        p.success_count = 5;
        p.body = "revised body".into();
        store.upsert_procedure(&p).unwrap();

        let got = store.get_procedure(p.id).unwrap().unwrap();
        assert_eq!(got.success_count, 5);
        assert_eq!(got.body, "revised body");
        // Still a single row for this goal.
        let all = store
            .query_procedures(
                &ProcedureQuery {
                    goal: Some("g".into()),
                    ..Default::default()
                },
                NOW,
            )
            .unwrap();
        assert_eq!(all.len(), 1);
    }

    #[test]
    fn get_missing_returns_none() {
        let store = store();
        assert!(store.get_procedure(ProcedureId::new()).unwrap().is_none());
    }

    #[test]
    fn query_by_goal_ranks_by_effectiveness() {
        let mut store = store();
        let goal = "goal:person:jd/cook-eggs";

        let mut strong = mk(goal, "eggs-quick-scramble");
        strong.success_count = 9;
        strong.failure_count = 1; // 0.9
        strong.last_used_at = Some(NOW);

        let mut weak = mk(goal, "omelette-classic");
        weak.success_count = 1;
        weak.failure_count = 1; // 0.5
        weak.last_used_at = Some(NOW);

        // Insert weak first to prove ordering is by effectiveness, not insert order.
        store.upsert_procedure(&weak).unwrap();
        store.upsert_procedure(&strong).unwrap();

        let ranked = store
            .query_procedures(
                &ProcedureQuery {
                    goal: Some(goal.into()),
                    ..Default::default()
                },
                NOW,
            )
            .unwrap();
        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].name, "eggs-quick-scramble");
        assert_eq!(ranked[1].name, "omelette-classic");
    }

    #[test]
    fn query_filters_out_procedures_whose_preconditions_fail() {
        let mut store = store();
        let goal = "goal:person:jd/cook-eggs";

        // Only eggs are in the pantry.
        assert_claim(&mut store, "person:jd", "attr:in-pantry", "ingredient:eggs");

        let mut needs_eggs = mk(goal, "needs-eggs");
        needs_eggs.preconditions = vec![precondition(
            "person:jd",
            "attr:in-pantry",
            "ingredient:eggs",
            Expect::Exists,
        )];

        let mut needs_flour = mk(goal, "needs-flour");
        needs_flour.preconditions = vec![precondition(
            "person:jd",
            "attr:in-pantry",
            "ingredient:flour",
            Expect::Exists,
        )];

        // Passes only when eggs are absent (they are not) -> filtered out.
        let mut needs_no_eggs = mk(goal, "needs-no-eggs");
        needs_no_eggs.preconditions = vec![precondition(
            "person:jd",
            "attr:in-pantry",
            "ingredient:eggs",
            Expect::Absent,
        )];

        store.upsert_procedure(&needs_eggs).unwrap();
        store.upsert_procedure(&needs_flour).unwrap();
        store.upsert_procedure(&needs_no_eggs).unwrap();

        let surfaced = store
            .query_procedures(
                &ProcedureQuery {
                    goal: Some(goal.into()),
                    ..Default::default()
                },
                NOW,
            )
            .unwrap();
        let names: Vec<&str> = surfaced.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["needs-eggs"]);
    }

    #[test]
    fn superseded_versions_excluded_unless_requested() {
        let mut store = store();
        let goal = "goal:person:jd/cook-eggs";

        let mut v1 = mk(goal, "omelette");
        v1.version = 1;
        v1.is_current = false; // superseded head

        let mut v2 = mk(goal, "omelette");
        v2.version = 2;
        v2.supersedes = Some(v1.id);
        v2.is_current = true;

        store.upsert_procedure(&v1).unwrap();
        store.upsert_procedure(&v2).unwrap();

        // Default: current heads only.
        let current = store
            .query_procedures(
                &ProcedureQuery {
                    goal: Some(goal.into()),
                    ..Default::default()
                },
                NOW,
            )
            .unwrap();
        assert_eq!(current.len(), 1);
        assert_eq!(current[0].version, 2);
        assert_eq!(current[0].supersedes, Some(v1.id));

        // Opt in to history.
        let with_history = store
            .query_procedures(
                &ProcedureQuery {
                    goal: Some(goal.into()),
                    include_superseded: true,
                    ..Default::default()
                },
                NOW,
            )
            .unwrap();
        assert_eq!(with_history.len(), 2);
    }

    #[test]
    fn query_by_intent_substring_and_limit() {
        let mut store = store();
        let goal = "g";
        let mut a = mk(goal, "a");
        a.intent = "whisk the eggs gently".into();
        a.success_count = 1;
        a.last_used_at = Some(NOW);
        let mut b = mk(goal, "b");
        b.intent = "fold the omelette".into();
        b.success_count = 1;
        b.last_used_at = Some(NOW);
        store.upsert_procedure(&a).unwrap();
        store.upsert_procedure(&b).unwrap();

        let hits = store
            .query_procedures(
                &ProcedureQuery {
                    intent_contains: Some("eggs".into()),
                    ..Default::default()
                },
                NOW,
            )
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "a");

        let limited = store
            .query_procedures(
                &ProcedureQuery {
                    goal: Some(goal.into()),
                    limit: Some(1),
                    ..Default::default()
                },
                NOW,
            )
            .unwrap();
        assert_eq!(limited.len(), 1);
    }

    #[test]
    fn apply_report_persists_counter_changes() {
        let mut store = store();
        let p = mk("g", "proc");
        let id = store.upsert_procedure(&p).unwrap();

        // Success -> success_count + use_count.
        let effect = store
            .apply_procedure_report(id, &OutcomeReport::success(ProcedureId::new()), NOW + 10)
            .unwrap()
            .expect("procedure exists");
        assert!(effect.counted_as_success);
        let after = store.get_procedure(id).unwrap().unwrap();
        assert_eq!(after.success_count, 1);
        assert_eq!(after.use_count, 1);
        assert_eq!(after.last_used_at, Some(NOW + 10));

        // Bad result -> failure_count.
        store
            .apply_procedure_report(
                id,
                &OutcomeReport::failure(ProcedureId::new(), FailureMode::BadResult),
                NOW + 20,
            )
            .unwrap()
            .unwrap();
        let after = store.get_procedure(id).unwrap().unwrap();
        assert_eq!(after.failure_count, 1);
        assert_eq!(after.use_count, 2);

        // Executor error -> no procedure stat change.
        let effect = store
            .apply_procedure_report(
                id,
                &OutcomeReport::failure(ProcedureId::new(), FailureMode::ExecutorError),
                NOW + 30,
            )
            .unwrap()
            .unwrap();
        assert!(effect.attributed_to_executor);
        let after = store.get_procedure(id).unwrap().unwrap();
        assert_eq!(after.success_count, 1);
        assert_eq!(after.failure_count, 1);
        assert_eq!(after.use_count, 2);

        // Preconditions stale -> flag only, no counter change.
        let effect = store
            .apply_procedure_report(
                id,
                &OutcomeReport::failure(ProcedureId::new(), FailureMode::PreconditionsStale),
                NOW + 40,
            )
            .unwrap()
            .unwrap();
        assert!(effect.flagged_precondition_stale);
        let after = store.get_procedure(id).unwrap().unwrap();
        assert_eq!(after.failure_count, 1);
        assert_eq!(after.use_count, 2);
    }

    #[test]
    fn apply_report_to_missing_procedure_is_none() {
        let mut store = store();
        let outcome = store
            .apply_procedure_report(
                ProcedureId::new(),
                &OutcomeReport::success(ProcedureId::new()),
                NOW,
            )
            .unwrap();
        assert!(outcome.is_none());
    }

    #[test]
    fn abandoned_outcome_records_use_only() {
        let mut store = store();
        let id = store.upsert_procedure(&mk("g", "proc")).unwrap();
        let report = OutcomeReport {
            receipt_id: ProcedureId::new(),
            outcome: Outcome::Abandoned,
            failure_mode: None,
            executor_confidence: None,
            cost: None,
            notes: None,
        };
        store
            .apply_procedure_report(id, &report, NOW + 5)
            .unwrap()
            .unwrap();
        let after = store.get_procedure(id).unwrap().unwrap();
        assert_eq!(after.use_count, 1);
        assert_eq!(after.success_count, 0);
        assert_eq!(after.failure_count, 0);
    }
}
