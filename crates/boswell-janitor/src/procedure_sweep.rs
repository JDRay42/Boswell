//! Provenance-aware procedure tier management (procedural memory, Phase 3).
//!
//! This is the background promotion/demotion pass the design calls for (§5.2,
//! §8.1 #4): a Janitor sweep that, for each stored procedure, gathers its
//! [`CorroborationFacts`] from the provenance ledger, layers the Janitor's own
//! lifecycle policy (failing / stale) on top, and asks the
//! [`PromotionGatekeeper`] whether the procedure should climb, hold, or fall a
//! tier — then applies the verdict. The Gatekeeper decides; the Janitor applies.
//!
//! The sweep is the general case, and it costs an entry up to one sweep interval
//! of lag before a just-earned promotion takes effect (§8 #4). §8.1's answer for
//! the case where that lag is least acceptable is
//! [`Janitor::endorse_and_promote`]:
//! an authority endorsement records its stamp and re-evaluates that
//! one entry synchronously, so the climb it earns lands with the endorsement
//! rather than at the next tick. It reaches the same verdict the sweep would;
//! the only difference is when.
//!
//! Unlike the claim sweep, this operates on the concrete [`SqliteStore`] because
//! procedures live outside the `ClaimStore` trait.

use crate::{Janitor, JanitorError};
use boswell_domain::{CorroborationFacts, Procedure, ProcedureId, ProvenanceStamp, Tier};
use boswell_gatekeeper::{PromotionDecision, PromotionGatekeeper};
use boswell_store::{SqliteStore, StoreError};
use std::time::{SystemTime, UNIX_EPOCH};

/// A procedure needs at least this many trials before a losing win/loss record
/// counts as "failing" (avoids demoting on one-off noise).
const FAILING_MIN_TRIALS: u64 = 3;

/// Map a store failure onto a Janitor error, keeping an authority refusal
/// distinguishable from a broken store — the fast-track hands both back to a
/// caller that has to answer differently for each.
fn map_store_error(e: StoreError) -> JanitorError {
    match e {
        StoreError::Unauthorized(msg) => JanitorError::Unauthorized(msg),
        other => JanitorError::Store(other.to_string()),
    }
}

/// Current time in milliseconds since the Unix epoch (procedures timestamp in ms).
fn current_timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

impl Janitor {
    /// Sweep procedures, climbing or falling each one's tier per the promotion
    /// Gatekeeper (design §5.2). Returns the number of procedures whose tier
    /// changed. Honors `dry_run` (logs intended changes without applying them).
    ///
    /// Operates on the concrete [`SqliteStore`] because procedures are not part of
    /// the `ClaimStore` trait; the server's sweep loop calls this alongside the
    /// claim sweep.
    pub fn sweep_procedures(&mut self, store: &mut SqliteStore) -> Result<usize, JanitorError> {
        let gatekeeper = PromotionGatekeeper::default();
        let now = current_timestamp_ms();
        let stale_after_ms = self.project_stale_days() * 86_400_000;

        let procedures = store.list_procedures(false).map_err(map_store_error)?;

        let mut changed = 0;
        for procedure in procedures {
            if self
                .evaluate_procedure(store, &procedure, &gatekeeper, now, stale_after_ms)?
                .is_some()
            {
                changed += 1;
            }
        }
        Ok(changed)
    }

    /// Record an authority endorsement of a procedure and evaluate that one
    /// procedure immediately — §8.1's synchronous fast-track for the case where
    /// waiting a sweep interval is least acceptable (design §8 #4).
    ///
    /// Returns the tier the procedure moved to, or `None` if it did not move —
    /// which includes an endorsement that is valid but not yet enough to climb,
    /// an unknown `procedure_id`, and a dry-run Janitor.
    ///
    /// The verdict is the sweep's verdict: the same [`CorroborationFacts`], the
    /// same lifecycle policy, the same [`PromotionGatekeeper`]. Falls still take
    /// priority over climbs, so endorsing a failing procedure demotes it rather
    /// than promoting it — the fast-track changes *when* the entry is judged, never
    /// *how*.
    ///
    /// `dry_run` governs the Janitor's own mutation, the tier change, and not the
    /// endorsement: the stamp is the caller's write and is recorded either way.
    ///
    /// # Errors
    ///
    /// [`JanitorError::Unauthorized`] if the endorser lacks the `Endorse` op or
    /// the procedure's namespace is outside its authority; the endorsement is not
    /// recorded and no tier changes.
    pub fn endorse_and_promote(
        &mut self,
        store: &mut SqliteStore,
        procedure_id: ProcedureId,
        stamp: &ProvenanceStamp,
    ) -> Result<Option<Tier>, JanitorError> {
        let existed = store
            .endorse_procedure(procedure_id, stamp)
            .map_err(map_store_error)?;
        if !existed {
            return Ok(None);
        }
        self.promote_procedure_now(store, procedure_id)
    }

    /// Evaluate a single procedure now and apply the Gatekeeper's verdict, rather
    /// than waiting for the next sweep. Returns the tier it moved to, or `None` if
    /// it held, was suppressed by `dry_run`, or does not exist.
    ///
    /// [`Janitor::endorse_and_promote`] is the intended entry point; this is here
    /// for a caller that has already recorded whatever changed the facts.
    pub fn promote_procedure_now(
        &mut self,
        store: &mut SqliteStore,
        procedure_id: ProcedureId,
    ) -> Result<Option<Tier>, JanitorError> {
        let Some(procedure) = store.get_procedure(procedure_id).map_err(map_store_error)? else {
            return Ok(None);
        };
        let now = current_timestamp_ms();
        let stale_after_ms = self.project_stale_days() * 86_400_000;
        self.evaluate_procedure(
            store,
            &procedure,
            &PromotionGatekeeper::default(),
            now,
            stale_after_ms,
        )
    }

    /// Gather one procedure's facts, ask the Gatekeeper, and apply the verdict.
    /// Returns the tier it moved to, or `None` if it held or `dry_run` suppressed
    /// the move. The single place a procedure's tier changes, so the sweep and the
    /// fast-track cannot drift apart.
    fn evaluate_procedure(
        &mut self,
        store: &mut SqliteStore,
        procedure: &Procedure,
        gatekeeper: &PromotionGatekeeper,
        now: u64,
        stale_after_ms: u64,
    ) -> Result<Option<Tier>, JanitorError> {
        let Some(mut facts) = store
            .corroboration_facts_for_procedure(procedure.id, now)
            .map_err(map_store_error)?
        else {
            return Ok(None);
        };
        self.apply_procedure_lifecycle_policy(procedure, &mut facts, now, stale_after_ms);

        let target = match gatekeeper.evaluate(&facts) {
            PromotionDecision::Climb(t) | PromotionDecision::Fall(t) => t,
            PromotionDecision::Hold => return Ok(None),
        };

        if self.dry_run() {
            tracing::info!(
                "DRY RUN: would move procedure {} from {:?} to {:?}",
                procedure.id,
                procedure.tier,
                target
            );
            return Ok(None);
        }

        let updated = store
            .set_procedure_tier(procedure.id, target, now)
            .map_err(map_store_error)?;
        if !updated {
            return Ok(None);
        }
        if target.rank() > procedure.tier.rank() {
            self.record_procedure_promotion(procedure.tier);
        } else {
            self.record_procedure_demotion(procedure.tier);
        }
        tracing::info!(
            "Procedure {} moved from {:?} to {:?}",
            procedure.id,
            procedure.tier,
            target
        );
        Ok(Some(target))
    }

    /// Expire overdue, unreported execution receipts ("silence is not success",
    /// design §3.3), returning the number expired. Each expiry records an
    /// `unknown` against its procedure's effectiveness. Honors `dry_run` (skips
    /// the mutation and logs instead).
    pub fn sweep_receipts(&mut self, store: &mut SqliteStore) -> Result<usize, JanitorError> {
        if self.dry_run() {
            let pending = store
                .count_receipts(boswell_domain::ReceiptStatus::Pending)
                .map_err(|e| JanitorError::Store(e.to_string()))?;
            tracing::info!(
                "DRY RUN: would expire overdue receipts among {} pending",
                pending
            );
            return Ok(0);
        }
        let now = current_timestamp_ms();
        store
            .expire_receipts(now)
            .map_err(|e| JanitorError::Store(e.to_string()))
    }

    /// Set the Janitor-owned lifecycle fields on `facts`: a procedure is *failing*
    /// when its losses outweigh its wins over enough trials, and *stale* when its
    /// `stale_at` has passed or it has been idle past the project staleness window.
    fn apply_procedure_lifecycle_policy(
        &self,
        procedure: &Procedure,
        facts: &mut CorroborationFacts,
        now: u64,
        stale_after_ms: u64,
    ) {
        facts.failing = procedure.use_count >= FAILING_MIN_TRIALS
            && procedure.failure_count > procedure.success_count;

        let idle_since = procedure.last_used_at.unwrap_or(procedure.created_at);
        let idle_stale = now.saturating_sub(idle_since) >= stale_after_ms;
        let explicitly_stale = procedure.stale_at.is_some_and(|t| now >= t);
        facts.stale = idle_stale || explicitly_stale;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::JanitorConfig;
    use boswell_domain::{
        Assurance, Authority, DelegationChain, EvidenceType, Op, ProcedureSource, ProvenanceStamp,
        Tier,
    };

    fn now_ms() -> u64 {
        current_timestamp_ms()
    }

    fn procedure(name: &str, tier: Tier, now: u64) -> Procedure {
        Procedure {
            id: boswell_domain::ProcedureId::new(),
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
            body_format: boswell_domain::BodyFormat::Prose,
            content_type: "text/plain".into(),
            body: "b".into(),
            tier,
            use_count: 0,
            success_count: 0,
            failure_count: 0,
            unknown_count: 0,
            last_used_at: Some(now),
            created_at: now,
            updated_at: now,
            stale_at: None,
        }
    }

    fn stamp(author: &str, max_tier: Tier, ops: &[Op]) -> ProvenanceStamp {
        ProvenanceStamp {
            author: author.into(),
            delegation_chain: DelegationChain(vec![author.into()]),
            authority: Authority {
                namespaces: vec!["person:jd".into()],
                max_tier,
                ops: ops.to_vec(),
            },
            evidence: EvidenceType::Observed,
            assurance: Assurance::Attested,
            task_id: None,
            session_id: None,
            timestamp: 1,
            dev_provider: false,
        }
    }

    #[test]
    fn sweep_promotes_corroborated_procedure() {
        let mut store = SqliteStore::new(":memory:", false, 0).unwrap();
        let now = now_ms();
        let proc = procedure("p", Tier::Task, now);
        // Two distinct authors write it (corroboration) at attested/observed.
        store
            .write_procedure_stamped(
                &proc,
                Tier::Task,
                &stamp("agent:a", Tier::Project, &[Op::Write]),
            )
            .unwrap();
        store
            .write_procedure_stamped(
                &proc,
                Tier::Task,
                &stamp("agent:b", Tier::Project, &[Op::Write]),
            )
            .unwrap();

        let mut janitor = Janitor::default_config();
        let changed = janitor.sweep_procedures(&mut store).unwrap();
        assert_eq!(changed, 1);
        assert_eq!(
            store.get_procedure(proc.id).unwrap().unwrap().tier,
            Tier::Project
        );
    }

    #[test]
    fn sweep_demotes_failing_procedure() {
        let mut store = SqliteStore::new(":memory:", false, 0).unwrap();
        let now = now_ms();
        let mut proc = procedure("p", Tier::Project, now);
        proc.use_count = 5;
        proc.failure_count = 4;
        proc.success_count = 1; // failing
        store
            .write_procedure_stamped(
                &proc,
                Tier::Project,
                &stamp("agent:a", Tier::Project, &[Op::Write]),
            )
            .unwrap();

        let mut janitor = Janitor::default_config();
        let changed = janitor.sweep_procedures(&mut store).unwrap();
        assert_eq!(changed, 1);
        assert_eq!(
            store.get_procedure(proc.id).unwrap().unwrap().tier,
            Tier::Task
        );
    }

    #[test]
    fn sweep_holds_uncorroborated_healthy_procedure() {
        let mut store = SqliteStore::new(":memory:", false, 0).unwrap();
        let now = now_ms();
        let proc = procedure("p", Tier::Task, now);
        // Single author, no effectiveness, not failing/stale -> hold.
        store
            .write_procedure_stamped(
                &proc,
                Tier::Task,
                &stamp("agent:a", Tier::Project, &[Op::Write]),
            )
            .unwrap();

        let mut janitor = Janitor::default_config();
        let changed = janitor.sweep_procedures(&mut store).unwrap();
        assert_eq!(changed, 0);
        assert_eq!(
            store.get_procedure(proc.id).unwrap().unwrap().tier,
            Tier::Task
        );
    }

    #[test]
    fn sweep_receipts_expires_overdue() {
        use boswell_domain::{ExecutionReceipt, ReceiptStatus};
        let mut store = SqliteStore::new(":memory:", false, 0).unwrap();
        let now = now_ms();
        let proc = procedure("p", Tier::Task, now);
        store.upsert_procedure(&proc).unwrap();
        let overdue = ExecutionReceipt::issue(&proc, "agent:worker", now - 5000, 1000);
        store.issue_receipt(&overdue).unwrap();

        let mut janitor = Janitor::default_config();
        let expired = janitor.sweep_receipts(&mut store).unwrap();
        assert_eq!(expired, 1);
        assert_eq!(
            store.get_procedure(proc.id).unwrap().unwrap().unknown_count,
            1
        );
        assert_eq!(store.count_receipts(ReceiptStatus::Expired).unwrap(), 1);
    }

    /// The claim the fast-track exists to make: an authority endorsement lands its
    /// climb with the endorsement, not at the next sweep tick (design §8 #4). The
    /// procedure has one author and no effectiveness, so the sweep on its own would
    /// hold it — and the sweep that follows finds nothing left to do.
    #[test]
    fn an_endorsement_promotes_without_waiting_for_a_sweep() {
        let mut store = SqliteStore::new(":memory:", false, 0).unwrap();
        let now = now_ms();
        let proc = procedure("p", Tier::Task, now);
        store
            .write_procedure_stamped(
                &proc,
                Tier::Task,
                &stamp("agent:a", Tier::Task, &[Op::Write]),
            )
            .unwrap();

        let mut janitor = Janitor::default_config();
        assert_eq!(
            janitor.sweep_procedures(&mut store).unwrap(),
            0,
            "one author, no effectiveness: the sweep holds it"
        );

        let moved = janitor
            .endorse_and_promote(
                &mut store,
                proc.id,
                &stamp("agent:lead", Tier::Project, &[Op::Endorse]),
            )
            .unwrap();

        assert_eq!(moved, Some(Tier::Project));
        assert_eq!(
            store.get_procedure(proc.id).unwrap().unwrap().tier,
            Tier::Project
        );
        assert_eq!(
            janitor.metrics().total_promoted(),
            1,
            "a fast-tracked climb counts in the same counter as a swept one"
        );
        assert_eq!(
            janitor.sweep_procedures(&mut store).unwrap(),
            0,
            "the sweep has nothing left to apply"
        );
    }

    /// The fast-track changes when an entry is judged, never how. Falls take
    /// priority in the Gatekeeper, so endorsing a failing procedure demotes it.
    #[test]
    fn endorsing_a_failing_procedure_demotes_it() {
        let mut store = SqliteStore::new(":memory:", false, 0).unwrap();
        let now = now_ms();
        let mut proc = procedure("p", Tier::Project, now);
        proc.use_count = 5;
        proc.failure_count = 4;
        proc.success_count = 1;
        store
            .write_procedure_stamped(
                &proc,
                Tier::Project,
                &stamp("agent:a", Tier::Project, &[Op::Write]),
            )
            .unwrap();

        let mut janitor = Janitor::default_config();
        let moved = janitor
            .endorse_and_promote(
                &mut store,
                proc.id,
                &stamp("agent:lead", Tier::Permanent, &[Op::Endorse]),
            )
            .unwrap();

        assert_eq!(moved, Some(Tier::Task));
        assert_eq!(
            store.get_procedure(proc.id).unwrap().unwrap().tier,
            Tier::Task
        );
    }

    /// An endorser without the `Endorse` op is refused, and the refusal leaves both
    /// the ledger and the tier untouched — the fast-track is not a way around the
    /// authority check.
    #[test]
    fn a_refused_endorsement_changes_nothing() {
        let mut store = SqliteStore::new(":memory:", false, 0).unwrap();
        let now = now_ms();
        let proc = procedure("p", Tier::Task, now);
        store
            .write_procedure_stamped(
                &proc,
                Tier::Task,
                &stamp("agent:a", Tier::Task, &[Op::Write]),
            )
            .unwrap();

        let mut janitor = Janitor::default_config();
        let err = janitor
            .endorse_and_promote(
                &mut store,
                proc.id,
                &stamp("agent:nobody", Tier::Permanent, &[Op::Write]),
            )
            .unwrap_err();

        assert!(
            matches!(err, JanitorError::Unauthorized(_)),
            "expected an authority refusal, got {err:?}"
        );
        assert_eq!(
            store.get_procedure(proc.id).unwrap().unwrap().tier,
            Tier::Task
        );
        assert_eq!(
            janitor.sweep_procedures(&mut store).unwrap(),
            0,
            "no endorse stamp was recorded, so the sweep still holds it"
        );
    }

    /// `dry_run` governs the Janitor's own mutation. The endorsement is the
    /// caller's write and is recorded either way, so the climb it earns lands on
    /// the next non-dry sweep.
    #[test]
    fn dry_run_records_the_endorsement_but_holds_the_tier() {
        let mut store = SqliteStore::new(":memory:", false, 0).unwrap();
        let now = now_ms();
        let proc = procedure("p", Tier::Task, now);
        store
            .write_procedure_stamped(
                &proc,
                Tier::Task,
                &stamp("agent:a", Tier::Task, &[Op::Write]),
            )
            .unwrap();

        let mut dry = Janitor::new(JanitorConfig {
            dry_run: true,
            ..JanitorConfig::default()
        });
        let moved = dry
            .endorse_and_promote(
                &mut store,
                proc.id,
                &stamp("agent:lead", Tier::Project, &[Op::Endorse]),
            )
            .unwrap();

        assert_eq!(moved, None);
        assert_eq!(
            store.get_procedure(proc.id).unwrap().unwrap().tier,
            Tier::Task
        );

        let mut wet = Janitor::default_config();
        assert_eq!(
            wet.sweep_procedures(&mut store).unwrap(),
            1,
            "the endorse stamp survived the dry run"
        );
        assert_eq!(
            store.get_procedure(proc.id).unwrap().unwrap().tier,
            Tier::Project
        );
    }

    #[test]
    fn endorsing_an_unknown_procedure_reports_no_change() {
        let mut store = SqliteStore::new(":memory:", false, 0).unwrap();
        let mut janitor = Janitor::default_config();
        let moved = janitor
            .endorse_and_promote(
                &mut store,
                boswell_domain::ProcedureId::new(),
                &stamp("agent:lead", Tier::Project, &[Op::Endorse]),
            )
            .unwrap();
        assert_eq!(moved, None);
    }

    #[test]
    fn dry_run_makes_no_changes() {
        let mut store = SqliteStore::new(":memory:", false, 0).unwrap();
        let now = now_ms();
        let proc = procedure("p", Tier::Task, now);
        store
            .write_procedure_stamped(
                &proc,
                Tier::Task,
                &stamp("agent:a", Tier::Project, &[Op::Write]),
            )
            .unwrap();
        store
            .write_procedure_stamped(
                &proc,
                Tier::Task,
                &stamp("agent:b", Tier::Project, &[Op::Write]),
            )
            .unwrap();

        let mut janitor = Janitor::new(JanitorConfig {
            dry_run: true,
            ..JanitorConfig::default()
        });
        let changed = janitor.sweep_procedures(&mut store).unwrap();
        assert_eq!(changed, 0);
        assert_eq!(
            store.get_procedure(proc.id).unwrap().unwrap().tier,
            Tier::Task
        );
    }
}
