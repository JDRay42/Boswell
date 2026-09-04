//! The provenance-stamped write path for procedures and goal edges (procedural
//! memory, Phase 3).
//!
//! Every write is authorized against its author's [`Authority`], stamped into the
//! append-only `provenance_stamps` ledger, and — for procedures — clamped to the
//! [`entry_tier`] its author is entitled to (design §5). The ledger lets
//! [`SqliteStore::corroboration_facts_for_procedure`] tell the Gatekeeper how many
//! distinct authors backed an entry, whether a higher-authority principal endorsed
//! it, and the strongest evidence/assurance behind it — the inputs to a tier
//! climb.
//!
//! Outcome reports are themselves gatekept writes (design §3.3): a low-assurance
//! executor's failure report cannot, on its own, tank a team-tier procedure —
//! [`SqliteStore::apply_procedure_report_stamped`] quarantines it pending
//! corroboration. This is the anti-poisoning lever for reports.

use crate::procedure_store::{from_json, to_json};
use crate::{SqliteStore, StoreError};
use boswell_domain::{
    entry_tier, Assurance, Authority, ChildRef, CorroborationFacts, DelegationChain, EvidenceType,
    GoalEdge, Op, OutcomeReport, Procedure, ProcedureId, ProvenanceStamp, ReportEffect, Tier,
};
use rusqlite::params;

/// Which kind of write a stamp records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StampKind {
    /// The originating (advocating) write of an entry.
    Write,
    /// A higher-authority endorsement of an existing entry.
    Endorse,
    /// An outcome report against a procedure.
    Report,
}

impl StampKind {
    /// Stable storage string.
    pub fn as_str(&self) -> &'static str {
        match self {
            StampKind::Write => "write",
            StampKind::Endorse => "endorse",
            StampKind::Report => "report",
        }
    }

    /// Parse from the storage string.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "write" => Some(StampKind::Write),
            "endorse" => Some(StampKind::Endorse),
            "report" => Some(StampKind::Report),
            _ => None,
        }
    }
}

/// A stamp as read back from the ledger, with its kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredStamp {
    /// What kind of write this stamp recorded.
    pub kind: StampKind,
    /// The stamp itself.
    pub stamp: ProvenanceStamp,
}

/// The result of a provenance-stamped procedure write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StampedWriteOutcome {
    /// The tier the entry actually entered at.
    pub entry_tier: Tier,
    /// Whether the requested tier was clamped down by a ceiling.
    pub clamped: bool,
}

/// The result of a gatekept outcome report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StampedReportOutcome {
    /// The counter changes applied, if the report was applied.
    pub effect: Option<ReportEffect>,
    /// Whether a negative report was quarantined (recorded but not applied)
    /// because the reporter's assurance is too low for the procedure's tier.
    pub quarantined: bool,
}

impl SqliteStore {
    /// Write a procedure through the provenance-stamped path (design §5).
    ///
    /// Authorizes the write against the author's [`Authority`] (namespace + the
    /// `Write` op), clamps the tier to [`entry_tier`], upserts the procedure at
    /// that tier, and appends a `write` stamp. Returns the entered tier and whether
    /// it was clamped. Fails with [`StoreError::Unauthorized`] if the author may
    /// not write this namespace.
    pub fn write_procedure_stamped(
        &mut self,
        procedure: &Procedure,
        requested_tier: Tier,
        stamp: &ProvenanceStamp,
    ) -> Result<StampedWriteOutcome, StoreError> {
        if !stamp.authority.can(Op::Write) {
            return Err(StoreError::Unauthorized(
                "author lacks the write op".to_string(),
            ));
        }
        if !stamp.authority.allows_namespace(&procedure.namespace) {
            return Err(StoreError::Unauthorized(format!(
                "namespace '{}' is outside the author's authority",
                procedure.namespace
            )));
        }

        let entered = entry_tier(requested_tier, stamp);
        let clamped = entered.rank() != requested_tier.rank();

        let mut to_write = procedure.clone();
        to_write.tier = entered;
        self.upsert_procedure(&to_write)?;

        self.append_stamp(
            "procedure",
            &procedure.id.to_string(),
            StampKind::Write,
            stamp,
        )?;

        Ok(StampedWriteOutcome {
            entry_tier: entered,
            clamped,
        })
    }

    /// Record a higher-authority endorsement of a procedure (design §5.2).
    ///
    /// Requires the `Endorse` op and namespace authority. Appends an `endorse`
    /// stamp; the next Gatekeeper evaluation can then let the entry climb. Returns
    /// `false` if no such procedure exists.
    pub fn endorse_procedure(
        &mut self,
        procedure_id: ProcedureId,
        stamp: &ProvenanceStamp,
    ) -> Result<bool, StoreError> {
        let Some(procedure) = self.get_procedure(procedure_id)? else {
            return Ok(false);
        };
        if !stamp.authority.can(Op::Endorse) {
            return Err(StoreError::Unauthorized(
                "endorser lacks the endorse op".to_string(),
            ));
        }
        if !stamp.authority.allows_namespace(&procedure.namespace) {
            return Err(StoreError::Unauthorized(format!(
                "namespace '{}' is outside the endorser's authority",
                procedure.namespace
            )));
        }
        self.append_stamp(
            "procedure",
            &procedure_id.to_string(),
            StampKind::Endorse,
            stamp,
        )?;
        Ok(true)
    }

    /// Add a goal edge through the provenance-stamped path (design §5.1).
    ///
    /// Authorizes against the parent goal's namespace + the `Write` op, adds the
    /// edge (including the Phase 2 cycle guard), and appends a `write` stamp keyed
    /// to the edge. Fails with [`StoreError::NotFound`] if the parent goal is
    /// missing, or [`StoreError::Unauthorized`] if the author may not write it.
    pub fn add_goal_edge_stamped(
        &mut self,
        edge: &GoalEdge,
        stamp: &ProvenanceStamp,
        now: u64,
    ) -> Result<(), StoreError> {
        let parent = self
            .get_goal(edge.parent)?
            .ok_or_else(|| StoreError::NotFound(format!("goal {}", edge.parent)))?;
        if !stamp.authority.can(Op::Write) {
            return Err(StoreError::Unauthorized(
                "author lacks the write op".to_string(),
            ));
        }
        if !stamp.authority.allows_namespace(&parent.namespace) {
            return Err(StoreError::Unauthorized(format!(
                "namespace '{}' is outside the author's authority",
                parent.namespace
            )));
        }
        self.add_goal_edge(edge, now)?;
        self.append_stamp("goal_edge", &goal_edge_key(edge), StampKind::Write, stamp)?;
        Ok(())
    }

    /// Apply an outcome report as a gatekept, provenance-stamped write (design
    /// §3.3).
    ///
    /// The report stamp is always recorded. Success and executor/precondition
    /// outcomes apply as in Phase 1. A *negative* report (`bad_result` /
    /// `step_failed`) against a **team-tier** (`project`/`permanent`) procedure is
    /// applied only if the reporter's assurance is high enough for that tier;
    /// otherwise it is **quarantined** — recorded, but not allowed to move the
    /// counters on its own. Returns `None` if the procedure does not exist.
    pub fn apply_procedure_report_stamped(
        &mut self,
        procedure_id: ProcedureId,
        report: &OutcomeReport,
        stamp: &ProvenanceStamp,
        now: u64,
    ) -> Result<Option<StampedReportOutcome>, StoreError> {
        let Some(mut procedure) = self.get_procedure(procedure_id)? else {
            return Ok(None);
        };

        // The report is itself a write; record its provenance regardless.
        self.append_stamp(
            "procedure",
            &procedure_id.to_string(),
            StampKind::Report,
            stamp,
        )?;

        // A negative report against a team-tier procedure needs a trusted reporter.
        let team_tier = procedure.tier.rank() >= Tier::Project.rank();
        let reporter_trusted = stamp.assurance.tier_ceiling().rank() >= procedure.tier.rank();
        if report.is_negative() && team_tier && !reporter_trusted {
            return Ok(Some(StampedReportOutcome {
                effect: None,
                quarantined: true,
            }));
        }

        let effect = procedure.apply_report(report, now);
        self.upsert_procedure(&procedure)?;
        Ok(Some(StampedReportOutcome {
            effect: Some(effect),
            quarantined: false,
        }))
    }

    /// Compute the [`CorroborationFacts`] the Gatekeeper weighs for a procedure.
    ///
    /// Populates the provenance-derived fields from the stamp ledger and the
    /// derived effectiveness; the lifecycle fields (`contradicted_by_higher_authority`,
    /// `failing`, `stale`) are left `false` for the caller (e.g. the Janitor) to
    /// set from its own policy. `now` (Unix ms) is the effectiveness reference.
    /// Returns `None` if the procedure does not exist.
    pub fn corroboration_facts_for_procedure(
        &self,
        procedure_id: ProcedureId,
        now: u64,
    ) -> Result<Option<CorroborationFacts>, StoreError> {
        let Some(procedure) = self.get_procedure(procedure_id)? else {
            return Ok(None);
        };
        let stamps = self.get_provenance_stamps("procedure", &procedure_id.to_string())?;

        let mut distinct: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut endorsed_max_tier: Option<Tier> = None;
        let mut author_max_tier: Option<Tier> = None;
        let mut best_evidence = EvidenceType::ToolOutput;
        let mut best_assurance = Assurance::None;

        for stored in &stamps {
            best_evidence = best_evidence.stronger(stored.stamp.evidence);
            best_assurance = best_assurance.stronger(stored.stamp.assurance);
            match stored.kind {
                StampKind::Write => {
                    distinct.insert(stored.stamp.author.clone());
                    author_max_tier = Some(match author_max_tier {
                        Some(t) => t.max(stored.stamp.authority.max_tier),
                        None => stored.stamp.authority.max_tier,
                    });
                }
                StampKind::Endorse => {
                    distinct.insert(stored.stamp.author.clone());
                    if stored.stamp.authority.can(Op::Endorse) {
                        endorsed_max_tier = Some(match endorsed_max_tier {
                            Some(t) => t.max(stored.stamp.authority.max_tier),
                            None => stored.stamp.authority.max_tier,
                        });
                    }
                }
                StampKind::Report => {}
            }
        }

        Ok(Some(CorroborationFacts {
            current_tier: procedure.tier,
            distinct_authors: distinct.len(),
            endorsed_max_tier,
            author_max_tier: author_max_tier.unwrap_or(procedure.tier),
            best_evidence,
            best_assurance,
            effectiveness: procedure.effectiveness(now),
            contradicted_by_higher_authority: false,
            failing: false,
            stale: false,
        }))
    }

    /// Read the provenance stamps for a procedure, oldest first.
    pub fn procedure_provenance(
        &self,
        procedure_id: ProcedureId,
    ) -> Result<Vec<StoredStamp>, StoreError> {
        self.get_provenance_stamps("procedure", &procedure_id.to_string())
    }

    /// Read the provenance stamps for a goal edge, oldest first.
    pub fn goal_edge_provenance(&self, edge: &GoalEdge) -> Result<Vec<StoredStamp>, StoreError> {
        self.get_provenance_stamps("goal_edge", &goal_edge_key(edge))
    }

    /// Append one stamp to the ledger.
    fn append_stamp(
        &self,
        entity_kind: &str,
        entity_id: &str,
        kind: StampKind,
        stamp: &ProvenanceStamp,
    ) -> Result<(), StoreError> {
        let delegation = to_json(&stamp.delegation_chain.0)?;
        let namespaces = to_json(&stamp.authority.namespaces)?;
        let ops = to_json(
            &stamp
                .authority
                .ops
                .iter()
                .map(|o| o.as_str().to_string())
                .collect::<Vec<_>>(),
        )?;
        self.conn.execute(
            "INSERT INTO provenance_stamps (
                entity_kind, entity_id, stamp_kind, author, delegation_chain,
                authority_namespaces, authority_max_tier, authority_ops,
                evidence, assurance, task_id, session_id, timestamp, dev_provider
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                entity_kind,
                entity_id,
                kind.as_str(),
                &stamp.author,
                &delegation,
                &namespaces,
                stamp.authority.max_tier.as_str(),
                &ops,
                stamp.evidence.as_str(),
                stamp.assurance.as_str(),
                &stamp.task_id,
                &stamp.session_id,
                stamp.timestamp as i64,
                stamp.dev_provider as i64,
            ],
        )?;
        Ok(())
    }

    /// Read stamps for an entity, oldest first (ascending ledger id).
    fn get_provenance_stamps(
        &self,
        entity_kind: &str,
        entity_id: &str,
    ) -> Result<Vec<StoredStamp>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT stamp_kind, author, delegation_chain, authority_namespaces, \
             authority_max_tier, authority_ops, evidence, assurance, task_id, session_id, \
             timestamp, dev_provider FROM provenance_stamps WHERE entity_kind = ?1 AND entity_id = ?2 \
             ORDER BY id",
        )?;
        let rows = stmt
            .query_map(params![entity_kind, entity_id], Self::row_to_stamp)?
            .collect::<Result<Vec<Result<StoredStamp, StoreError>>, rusqlite::Error>>()?;
        rows.into_iter().collect()
    }

    fn row_to_stamp(row: &rusqlite::Row<'_>) -> rusqlite::Result<Result<StoredStamp, StoreError>> {
        let decode = |row: &rusqlite::Row<'_>| -> Result<StoredStamp, StoreError> {
            let kind_str: String = row.get("stamp_kind")?;
            let kind = StampKind::parse(&kind_str).ok_or_else(|| {
                StoreError::InvalidData(format!("Unknown stamp kind: {}", kind_str))
            })?;
            let delegation: Vec<String> = from_json(&row.get::<_, String>("delegation_chain")?)?;
            let namespaces: Vec<String> =
                from_json(&row.get::<_, String>("authority_namespaces")?)?;
            let op_strs: Vec<String> = from_json(&row.get::<_, String>("authority_ops")?)?;
            let ops = op_strs
                .iter()
                .map(|s| {
                    Op::parse(s)
                        .ok_or_else(|| StoreError::InvalidData(format!("Unknown op: {}", s)))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let max_tier_str: String = row.get("authority_max_tier")?;
            let max_tier = Tier::parse(&max_tier_str).ok_or_else(|| {
                StoreError::InvalidData(format!("Unknown tier: {}", max_tier_str))
            })?;
            let evidence_str: String = row.get("evidence")?;
            let evidence = EvidenceType::parse(&evidence_str).ok_or_else(|| {
                StoreError::InvalidData(format!("Unknown evidence: {}", evidence_str))
            })?;
            let assurance_str: String = row.get("assurance")?;
            let assurance = Assurance::parse(&assurance_str).ok_or_else(|| {
                StoreError::InvalidData(format!("Unknown assurance: {}", assurance_str))
            })?;

            Ok(StoredStamp {
                kind,
                stamp: ProvenanceStamp {
                    author: row.get("author")?,
                    delegation_chain: DelegationChain(delegation),
                    authority: Authority {
                        namespaces,
                        max_tier,
                        ops,
                    },
                    evidence,
                    assurance,
                    task_id: row.get("task_id")?,
                    session_id: row.get("session_id")?,
                    timestamp: row.get::<_, i64>("timestamp")? as u64,
                    dev_provider: row.get::<_, i64>("dev_provider")? != 0,
                },
            })
        };
        Ok(decode(row))
    }
}

/// The canonical ledger key for a goal edge: `parent|childkind|childid|role`.
fn goal_edge_key(edge: &GoalEdge) -> String {
    let (kind, child) = match edge.child {
        ChildRef::Goal(id) => ("goal", id.to_string()),
        ChildRef::Procedure(id) => ("procedure", id.to_string()),
    };
    format!("{}|{}|{}|{}", edge.parent, kind, child, edge.role.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use boswell_domain::{BodyFormat, EdgeRole, FailureMode, Goal, GoalId, ProcedureSource};

    const NOW: u64 = 1_700_000_000_000;

    fn store() -> SqliteStore {
        SqliteStore::new(":memory:", false, 0).unwrap()
    }

    fn procedure(name: &str) -> Procedure {
        Procedure {
            id: ProcedureId::new(),
            namespace: "person:jd".into(),
            name: name.into(),
            version: 1,
            supersedes: None,
            is_current: true,
            source: ProcedureSource::Authored,
            goal: "goal:person:jd/cook-eggs".into(),
            intent: name.into(),
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
            body: "body".into(),
            tier: Tier::Ephemeral,
            use_count: 0,
            success_count: 0,
            failure_count: 0,
            last_used_at: None,
            created_at: NOW,
            updated_at: NOW,
            stale_at: None,
        }
    }

    fn stamp(
        author: &str,
        max_tier: Tier,
        ops: &[Op],
        evidence: EvidenceType,
        assurance: Assurance,
    ) -> ProvenanceStamp {
        ProvenanceStamp {
            author: author.into(),
            delegation_chain: DelegationChain(vec!["human:jd".into(), author.into()]),
            authority: Authority {
                namespaces: vec!["person:jd".into()],
                max_tier,
                ops: ops.to_vec(),
            },
            evidence,
            assurance,
            task_id: Some("t1".into()),
            session_id: None,
            timestamp: NOW,
            dev_provider: false,
        }
    }

    #[test]
    fn stamped_write_clamps_tier_and_records_stamp() {
        let mut store = store();
        let proc = procedure("omelette");
        // Requests permanent, but Asserted assurance caps entry at task.
        let s = stamp(
            "agent:worker",
            Tier::Permanent,
            &[Op::Write],
            EvidenceType::Observed,
            Assurance::Asserted,
        );
        let outcome = store
            .write_procedure_stamped(&proc, Tier::Permanent, &s)
            .unwrap();
        assert_eq!(outcome.entry_tier, Tier::Task);
        assert!(outcome.clamped);
        assert_eq!(
            store.get_procedure(proc.id).unwrap().unwrap().tier,
            Tier::Task
        );

        let stamps = store.procedure_provenance(proc.id).unwrap();
        assert_eq!(stamps.len(), 1);
        assert_eq!(stamps[0].kind, StampKind::Write);
        assert_eq!(stamps[0].stamp.author, "agent:worker");
    }

    #[test]
    fn dev_provider_taint_roundtrips() {
        let mut store = store();
        let proc = procedure("p");
        let mut s = stamp(
            "agent:interloper",
            Tier::Ephemeral,
            &[Op::Write],
            EvidenceType::ToolOutput,
            Assurance::Asserted,
        );
        s.dev_provider = true;
        store
            .write_procedure_stamped(&proc, Tier::Ephemeral, &s)
            .unwrap();
        let stamps = store.procedure_provenance(proc.id).unwrap();
        assert_eq!(stamps.len(), 1);
        assert!(stamps[0].stamp.dev_provider);
    }

    #[test]
    fn stamped_write_rejects_unauthorized() {
        let mut store = store();
        let proc = procedure("p");
        // No write op.
        let s = stamp(
            "agent:reader",
            Tier::Task,
            &[Op::Read],
            EvidenceType::Observed,
            Assurance::Attested,
        );
        assert!(matches!(
            store.write_procedure_stamped(&proc, Tier::Task, &s),
            Err(StoreError::Unauthorized(_))
        ));

        // Write op, but wrong namespace.
        let mut s2 = stamp(
            "agent:worker",
            Tier::Task,
            &[Op::Write],
            EvidenceType::Observed,
            Assurance::Attested,
        );
        s2.authority.namespaces = vec!["person:kim".into()];
        assert!(matches!(
            store.write_procedure_stamped(&proc, Tier::Task, &s2),
            Err(StoreError::Unauthorized(_))
        ));
    }

    #[test]
    fn corroboration_facts_count_authors_and_endorsement() {
        let mut store = store();
        let proc = procedure("p");
        store
            .write_procedure_stamped(
                &proc,
                Tier::Task,
                &stamp(
                    "agent:a",
                    Tier::Task,
                    &[Op::Write],
                    EvidenceType::Reported,
                    Assurance::Asserted,
                ),
            )
            .unwrap();
        // A second distinct author writes the same procedure.
        store
            .write_procedure_stamped(
                &proc,
                Tier::Task,
                &stamp(
                    "agent:b",
                    Tier::Project,
                    &[Op::Write],
                    EvidenceType::Observed,
                    Assurance::Verified,
                ),
            )
            .unwrap();
        // A project-tier endorser endorses it.
        store
            .endorse_procedure(
                proc.id,
                &stamp(
                    "project:lead",
                    Tier::Project,
                    &[Op::Endorse],
                    EvidenceType::Observed,
                    Assurance::Attested,
                ),
            )
            .unwrap();

        let facts = store
            .corroboration_facts_for_procedure(proc.id, NOW)
            .unwrap()
            .unwrap();
        assert_eq!(facts.distinct_authors, 3);
        assert_eq!(facts.endorsed_max_tier, Some(Tier::Project));
        assert_eq!(facts.author_max_tier, Tier::Project);
        assert_eq!(facts.best_evidence, EvidenceType::Observed);
        assert_eq!(facts.best_assurance, Assurance::Attested);
    }

    #[test]
    fn endorse_requires_endorse_op_and_existing_procedure() {
        let mut store = store();
        let proc = procedure("p");
        store
            .write_procedure_stamped(
                &proc,
                Tier::Task,
                &stamp(
                    "agent:a",
                    Tier::Task,
                    &[Op::Write],
                    EvidenceType::Observed,
                    Assurance::Verified,
                ),
            )
            .unwrap();

        // Lacks the endorse op.
        assert!(matches!(
            store.endorse_procedure(
                proc.id,
                &stamp(
                    "agent:a",
                    Tier::Task,
                    &[Op::Write],
                    EvidenceType::Observed,
                    Assurance::Verified,
                ),
            ),
            Err(StoreError::Unauthorized(_))
        ));

        // No such procedure.
        assert!(!store
            .endorse_procedure(
                ProcedureId::new(),
                &stamp(
                    "project:lead",
                    Tier::Project,
                    &[Op::Endorse],
                    EvidenceType::Observed,
                    Assurance::Attested,
                ),
            )
            .unwrap());
    }

    #[test]
    fn negative_report_on_team_tier_by_untrusted_reporter_is_quarantined() {
        let mut store = store();
        let mut proc = procedure("p");
        proc.tier = Tier::Project; // team tier
        store.upsert_procedure(&proc).unwrap();

        // Asserted reporter (ceiling task) cannot demote a project-tier procedure.
        let weak = stamp(
            "agent:interloper",
            Tier::Ephemeral,
            &[Op::Write],
            EvidenceType::ToolOutput,
            Assurance::Asserted,
        );
        let outcome = store
            .apply_procedure_report_stamped(
                proc.id,
                &OutcomeReport::failure(ProcedureId::new(), FailureMode::BadResult),
                &weak,
                NOW,
            )
            .unwrap()
            .unwrap();
        assert!(outcome.quarantined);
        assert!(outcome.effect.is_none());
        assert_eq!(
            store.get_procedure(proc.id).unwrap().unwrap().failure_count,
            0
        );
        // But the report stamp was still recorded.
        assert_eq!(store.procedure_provenance(proc.id).unwrap().len(), 1);
    }

    #[test]
    fn negative_report_on_team_tier_by_trusted_reporter_applies() {
        let mut store = store();
        let mut proc = procedure("p");
        proc.tier = Tier::Project;
        store.upsert_procedure(&proc).unwrap();

        let trusted = stamp(
            "project:lead",
            Tier::Project,
            &[Op::Write],
            EvidenceType::Observed,
            Assurance::Verified, // ceiling project
        );
        let outcome = store
            .apply_procedure_report_stamped(
                proc.id,
                &OutcomeReport::failure(ProcedureId::new(), FailureMode::BadResult),
                &trusted,
                NOW,
            )
            .unwrap()
            .unwrap();
        assert!(!outcome.quarantined);
        assert!(outcome.effect.unwrap().counted_as_failure);
        assert_eq!(
            store.get_procedure(proc.id).unwrap().unwrap().failure_count,
            1
        );
    }

    #[test]
    fn success_report_always_applies_even_on_team_tier() {
        let mut store = store();
        let mut proc = procedure("p");
        proc.tier = Tier::Permanent;
        store.upsert_procedure(&proc).unwrap();

        let weak = stamp(
            "agent:interloper",
            Tier::Ephemeral,
            &[Op::Write],
            EvidenceType::ToolOutput,
            Assurance::None,
        );
        let outcome = store
            .apply_procedure_report_stamped(
                proc.id,
                &OutcomeReport::success(ProcedureId::new()),
                &weak,
                NOW,
            )
            .unwrap()
            .unwrap();
        assert!(!outcome.quarantined);
        assert_eq!(
            store.get_procedure(proc.id).unwrap().unwrap().success_count,
            1
        );
    }

    #[test]
    fn negative_report_on_task_tier_applies_regardless_of_reporter() {
        let mut store = store();
        let proc = procedure("p"); // ephemeral by default (not team tier)
        store.upsert_procedure(&proc).unwrap();
        let weak = stamp(
            "agent:interloper",
            Tier::Ephemeral,
            &[Op::Write],
            EvidenceType::ToolOutput,
            Assurance::None,
        );
        let outcome = store
            .apply_procedure_report_stamped(
                proc.id,
                &OutcomeReport::failure(ProcedureId::new(), FailureMode::StepFailed("x".into())),
                &weak,
                NOW,
            )
            .unwrap()
            .unwrap();
        assert!(!outcome.quarantined);
        assert_eq!(
            store.get_procedure(proc.id).unwrap().unwrap().failure_count,
            1
        );
    }

    #[test]
    fn stamped_goal_edge_authorizes_and_records() {
        let mut store = store();
        let goal = Goal {
            id: GoalId::new(),
            namespace: "person:jd".into(),
            name: "cook-eggs".into(),
            intent: "cook eggs".into(),
            definition_of_done: vec![],
            tier: Tier::Project,
            created_at: NOW,
            updated_at: NOW,
            stale_at: None,
        };
        store.upsert_goal(&goal).unwrap();

        let edge = GoalEdge {
            parent: goal.id,
            child: ChildRef::Procedure(ProcedureId::new()),
            role: EdgeRole::Accomplish,
            preconditions: vec![],
            context_tags: vec![],
            usage_notes: String::new(),
            cached_effectiveness: 0.5,
        };
        let s = stamp(
            "agent:worker",
            Tier::Task,
            &[Op::Write],
            EvidenceType::Observed,
            Assurance::Verified,
        );
        store.add_goal_edge_stamped(&edge, &s, NOW).unwrap();
        assert_eq!(store.get_goal_edges(goal.id).unwrap().len(), 1);
        let prov = store.goal_edge_provenance(&edge).unwrap();
        assert_eq!(prov.len(), 1);
        assert_eq!(prov[0].stamp.author, "agent:worker");

        // Missing parent goal -> NotFound.
        let orphan = GoalEdge {
            parent: GoalId::new(),
            ..edge.clone()
        };
        assert!(matches!(
            store.add_goal_edge_stamped(&orphan, &s, NOW),
            Err(StoreError::NotFound(_))
        ));
    }
}
