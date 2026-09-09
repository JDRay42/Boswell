//! Sybil-independence scenarios — design §8 #1, the empirical half of §8.1.
//!
//! §8.1 calls this problem *empirical, not design*: "stand up devAuth, script the
//! four identities through cooperative and adversarial (clone-swarm) scenarios,
//! and measure." The unit tests in `promotion.rs` already pin the gatekeeper's
//! *policy* against hand-written [`CorroborationFacts`]. What they cannot tell us
//! is whether the facts the store actually computes, from stamps actually minted
//! by an identity provider, carry the independence signal that policy assumes.
//!
//! So this file wires the real pieces together — `boswell-devauth`'s four sample
//! identities, the real `SqliteStore` write path, the real fact computation, the
//! real `PromotionGatekeeper` — and measures the end-to-end verdict. Every
//! assertion is a *measurement of the shipped system*, not a restatement of the
//! policy.
//!
//! Findings are written up in `docs/architecture/15-procedural-memory.md` §8.3.
//! Scenarios that measure a **defect** are named `finding_*` and assert today's
//! behaviour, so that closing the gap shows up as a diff here.

use boswell_devauth::{DevAuth, DevAuthConfig, DevIdentity};
use boswell_domain::{
    BodyFormat, CorroborationFacts, DelegationChain, EvidenceType, Procedure, ProcedureId,
    ProcedureSource, ProvenanceStamp, Tier,
};
use boswell_gatekeeper::{PromotionDecision, PromotionGatekeeper};
use boswell_store::SqliteStore;

const NOW: u64 = 1_700_000_000_000;

// --- harness -------------------------------------------------------------

/// A running Boswell: real store, real devAuth, real gatekeeper.
struct Bench {
    store: SqliteStore,
    auth: DevAuth,
    gatekeeper: PromotionGatekeeper,
}

impl Bench {
    fn new() -> Self {
        // Built from an explicit config rather than `from_env`: that reads
        // process-global state, and these tests run in parallel.
        let auth = DevAuth::new(&DevAuthConfig {
            allow_dev_auth: true,
            production: false,
            environment_declared: true,
        })
        .expect("devAuth opts in");
        Bench {
            store: SqliteStore::new(":memory:", false, 0).expect("in-memory store"),
            auth,
            gatekeeper: PromotionGatekeeper::default(),
        }
    }

    /// Mint a stamp as `identity`, acting on behalf of `root`.
    ///
    /// `author` overrides the principal id so one identity can fan out into many
    /// subagents — the shape `ProvenanceStamp::author` documents
    /// (`agent:orch-7/sub:explore-3`). That is how a clone swarm is modelled: the
    /// clones authenticate as one identity but stamp distinct author strings.
    fn stamp(
        &self,
        identity: DevIdentity,
        root: &str,
        author: Option<&str>,
        evidence: EvidenceType,
        session: &str,
    ) -> ProvenanceStamp {
        let leaf = author
            .unwrap_or_else(|| identity.principal_id())
            .to_string();
        self.stamp_with_chain(
            identity,
            DelegationChain(vec![root.to_string(), leaf.clone()]),
            &leaf,
            evidence,
            session,
        )
    }

    /// As [`Bench::stamp`], but the caller supplies the whole delegation chain —
    /// including a self-rooted or empty one, which is what an adversary who
    /// simply declines to declare its provenance would send.
    fn stamp_with_chain(
        &self,
        identity: DevIdentity,
        chain: DelegationChain,
        author: &str,
        evidence: EvidenceType,
        session: &str,
    ) -> ProvenanceStamp {
        let mut stamp = self.auth.stamp(identity, chain, evidence, NOW);
        stamp.author = author.to_string();
        stamp.session_id = Some(session.to_string());
        stamp
    }

    /// Write `procedure` as the stamp's author, requesting `tier`. Returns the
    /// tier it actually entered at.
    fn write(&mut self, procedure: &Procedure, tier: Tier, stamp: &ProvenanceStamp) -> Tier {
        self.store
            .write_procedure_stamped(procedure, tier, stamp)
            .expect("authorized write")
            .entry_tier
    }

    /// The gatekeeper's verdict on `procedure`, over the facts the store computes.
    fn verdict(&self, id: ProcedureId) -> (PromotionDecision, CorroborationFacts) {
        let facts = self
            .store
            .corroboration_facts_for_procedure(id, NOW)
            .expect("facts query")
            .expect("procedure exists");
        (self.gatekeeper.evaluate(&facts), facts)
    }
}

/// A procedure in `namespace`, entering at the bottom.
fn procedure(namespace: &str, name: &str) -> Procedure {
    Procedure {
        id: ProcedureId::new(),
        namespace: namespace.into(),
        name: name.into(),
        version: 1,
        supersedes: None,
        is_current: true,
        source: ProcedureSource::Authored,
        goal: format!("goal:{}/{}", namespace, name),
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
        unknown_count: 0,
        last_used_at: None,
        created_at: NOW,
        updated_at: NOW,
        stale_at: None,
    }
}

// --- cooperative scenarios ----------------------------------------------

/// A lone worker's write lands at task tier and stays there. One writer under
/// one root corroborates nothing.
#[test]
fn worker_writes_task_tier_and_holds() {
    let mut bench = Bench::new();
    let proc = procedure("agent:worker", "omelette");
    let stamp = bench.stamp(
        DevIdentity::StandardWorker,
        "human:jd",
        None,
        EvidenceType::Observed,
        "s1",
    );

    // Asks for permanent; the worker's authority caps entry at task.
    assert_eq!(bench.write(&proc, Tier::Permanent, &stamp), Tier::Task);

    let (decision, facts) = bench.verdict(proc.id);
    assert_eq!(facts.distinct_delegation_roots, 1);
    assert_eq!(decision, PromotionDecision::Hold);
}

/// Two workers under genuinely distinct delegation roots corroborate — and the
/// climb is *still* refused, because corroboration raises confidence, not
/// authority: the worker's own `max_tier` is the ceiling.
#[test]
fn independent_roots_corroborate_but_authority_still_bounds_the_climb() {
    let mut bench = Bench::new();
    let proc = procedure("agent:worker", "omelette");
    for (root, author, session) in [
        ("human:alice", "agent:worker/sub:1", "s1"),
        ("human:bob", "agent:worker/sub:2", "s2"),
    ] {
        let stamp = bench.stamp(
            DevIdentity::StandardWorker,
            root,
            Some(author),
            EvidenceType::Observed,
            session,
        );
        bench.write(&proc, Tier::Task, &stamp);
    }

    let (decision, facts) = bench.verdict(proc.id);
    assert_eq!(facts.distinct_delegation_roots, 2, "diverse provenance");
    assert_eq!(facts.distinct_authors, 2);
    assert_eq!(facts.author_max_tier, Tier::Task);
    assert_eq!(
        facts.climb_ceiling(),
        Tier::Task,
        "authority is the binding constraint"
    );
    assert_eq!(decision, PromotionDecision::Hold);
}

/// The leader's own namespace, two independent roots: this is the cooperative
/// case that *does* climb, and the only shape in devAuth's roster that reaches
/// project tier without an endorsement.
#[test]
fn two_independent_leader_roots_climb_to_project() {
    let mut bench = Bench::new();
    let proc = procedure("project:alpha", "release");
    for (root, author, session) in [
        ("human:alice", "project:lead/sub:1", "s1"),
        ("human:bob", "project:lead/sub:2", "s2"),
    ] {
        let stamp = bench.stamp(
            DevIdentity::ProjectLeader,
            root,
            Some(author),
            EvidenceType::Observed,
            session,
        );
        bench.write(&proc, Tier::Task, &stamp);
    }

    let (decision, facts) = bench.verdict(proc.id);
    assert_eq!(facts.distinct_delegation_roots, 2);
    assert_eq!(decision, PromotionDecision::Climb(Tier::Project));
}

// --- adversarial scenarios ----------------------------------------------

/// The interloper is the red-team identity: whatever it asks for, its writes
/// stay on the floor. devAuth coerces its evidence to `tool_output` and its
/// authority tops out at ephemeral.
#[test]
fn interloper_cannot_climb_off_the_floor() {
    let mut bench = Bench::new();
    let proc = procedure("agent:interloper", "poison");
    let stamp = bench.stamp(
        DevIdentity::UntrustedInterloper,
        "human:mallory",
        None,
        // Asks for the strongest evidence there is; devAuth downgrades it.
        EvidenceType::Observed,
        "s1",
    );
    assert_eq!(stamp.evidence, EvidenceType::ToolOutput, "evidence coerced");

    assert_eq!(bench.write(&proc, Tier::Permanent, &stamp), Tier::Ephemeral);
    assert_eq!(bench.verdict(proc.id).0, PromotionDecision::Hold);
}

/// The clone swarm, played honestly: nine subagents under one puppeteer. Nine
/// distinct author identities, one delegation root — raw author count would
/// promote; provenance diversity does not. **This is the defense working.**
#[test]
fn honest_clone_swarm_manufactures_no_corroboration() {
    let mut bench = Bench::new();
    let proc = procedure("project:alpha", "release");
    for i in 0..9 {
        let stamp = bench.stamp(
            DevIdentity::ProjectLeader,
            "human:mallory",
            Some(&format!("project:lead/sub:{i}")),
            EvidenceType::Observed,
            &format!("s{i}"),
        );
        bench.write(&proc, Tier::Task, &stamp);
    }

    let (decision, facts) = bench.verdict(proc.id);
    assert_eq!(facts.distinct_authors, 9);
    assert_eq!(
        facts.distinct_delegation_roots, 1,
        "clones sharing a root count once"
    );
    assert_eq!(decision, PromotionDecision::Hold);
}

// --- findings ------------------------------------------------------------

/// The swarm from above, playing dishonestly: instead of declaring the puppeteer
/// as its root, each clone roots the chain at itself — "I answer to no one."
///
/// This used to work (§8.3, finding 1): the root was read straight off the chain,
/// so nine self-rooted clones became nine "distinct delegation roots" and the
/// entry climbed. The independence unit is now normalized to the authenticated
/// principal, so all nine collapse back onto `project:lead`, and the verdict
/// matches the honest swarm's. Declining to declare provenance buys nothing.
#[test]
fn self_rooted_clone_swarm_manufactures_no_corroboration() {
    let mut bench = Bench::new();
    let proc = procedure("project:alpha", "release");
    for i in 0..9 {
        let author = format!("project:lead/sub:{i}");
        let stamp = bench.stamp_with_chain(
            DevIdentity::ProjectLeader,
            // Self-rooted: "I answer to no one."
            DelegationChain(vec![author.clone()]),
            &author,
            EvidenceType::Observed,
            &format!("s{i}"),
        );
        bench.write(&proc, Tier::Task, &stamp);
    }

    let (decision, facts) = bench.verdict(proc.id);
    assert_eq!(facts.distinct_authors, 9);
    assert_eq!(
        facts.distinct_delegation_roots, 1,
        "nine subagents of one credential are one principal"
    );
    assert_eq!(decision, PromotionDecision::Hold);
}

/// The same swarm with no delegation chain at all. An empty chain has no root, so
/// the author is the fallback — and the author is normalized the same way, so
/// sending nothing is no better than lying.
#[test]
fn unchained_clone_swarm_manufactures_no_corroboration() {
    let mut bench = Bench::new();
    let proc = procedure("project:alpha", "release");
    for i in 0..9 {
        let author = format!("project:lead/sub:{i}");
        let stamp = bench.stamp_with_chain(
            DevIdentity::ProjectLeader,
            DelegationChain(vec![]),
            &author,
            EvidenceType::Observed,
            &format!("s{i}"),
        );
        bench.write(&proc, Tier::Task, &stamp);
    }

    let (decision, facts) = bench.verdict(proc.id);
    assert_eq!(facts.distinct_delegation_roots, 1);
    assert_eq!(decision, PromotionDecision::Hold);
}

/// The other half of the rule: normalization must not punish honest principals
/// who write directly. Two distinct principals, each its own root and neither
/// carrying a subagent path, still corroborate — the collapse applies to
/// subagents of one credential, not to short chains.
#[test]
fn self_rooted_but_genuinely_distinct_principals_still_corroborate() {
    let mut bench = Bench::new();
    let proc = procedure("project:alpha", "release");
    for principal in ["human:alice", "human:bob"] {
        let stamp = bench.stamp_with_chain(
            DevIdentity::ProjectLeader,
            DelegationChain(vec![principal.to_string()]),
            principal,
            EvidenceType::Observed,
            principal,
        );
        bench.write(&proc, Tier::Task, &stamp);
    }

    let (decision, facts) = bench.verdict(proc.id);
    assert_eq!(facts.distinct_delegation_roots, 2);
    assert_eq!(decision, PromotionDecision::Climb(Tier::Project));
}

/// **Finding 2 — the project leader cannot endorse the worker it leads.**
///
/// devAuth's module doc says "the worker writes task-tier, the project-leader can
/// endorse into project tier". Measured, that never happens: the leader's
/// authority covers `project*` only, and the worker writes into `agent:worker`,
/// so `endorse_procedure` refuses on namespace. The endorsement half of the trust
/// gradient is unreachable with the shipped roster.
#[test]
fn finding_project_leader_cannot_endorse_a_workers_entry() {
    let mut bench = Bench::new();
    let proc = procedure("agent:worker", "omelette");
    let write = bench.stamp(
        DevIdentity::StandardWorker,
        "human:jd",
        None,
        EvidenceType::Observed,
        "s1",
    );
    bench.write(&proc, Tier::Task, &write);

    let endorsement = bench.stamp(
        DevIdentity::ProjectLeader,
        "human:alice",
        None,
        EvidenceType::Observed,
        "s2",
    );
    let err = bench
        .store
        .endorse_procedure(proc.id, &endorsement)
        .expect_err("the leader's authority does not reach agent:worker");
    assert!(
        format!("{err}").contains("outside the endorser's authority"),
        "refused on namespace, not on the endorse op: {err}"
    );
}

/// **Finding 3 — top tier is unreachable in devAuth.**
///
/// A permanent-tier climb requires an endorsement whose `max_tier` is permanent
/// (§5.2) *and* a cross-authority endorser (§8.1). The only identity holding
/// `Op::Endorse` is the project leader, whose `max_tier` is project; the memory
/// manager reaches permanent but holds `Curate`, not `Endorse`. So no combination
/// of the four sample identities can promote anything to permanent, and the
/// top-tier rule the gatekeeper enforces has never been exercised end to end.
#[test]
fn finding_no_devauth_identity_can_promote_to_permanent() {
    let mut bench = Bench::new();
    let proc = procedure("project:alpha", "release");

    // Get it as high as devAuth allows: two independent leader roots -> project.
    for (root, author) in [
        ("human:alice", "project:lead/sub:1"),
        ("human:bob", "project:lead/sub:2"),
    ] {
        let stamp = bench.stamp(
            DevIdentity::ProjectLeader,
            root,
            Some(author),
            EvidenceType::Observed,
            author,
        );
        bench.write(&proc, Tier::Project, &stamp);
    }
    // And endorse it from a third, unrelated root.
    let endorsement = bench.stamp(
        DevIdentity::ProjectLeader,
        "human:carol",
        Some("project:lead/endorser"),
        EvidenceType::Observed,
        "s3",
    );
    assert!(bench
        .store
        .endorse_procedure(proc.id, &endorsement)
        .expect("the leader may endorse in its own namespace"));

    let (decision, facts) = bench.verdict(proc.id);
    assert!(
        facts.cross_authority_endorsement,
        "a distinct endorsing root"
    );
    assert_eq!(
        facts.endorsed_max_tier,
        Some(Tier::Project),
        "the only endorser in the roster tops out at project"
    );
    assert_eq!(facts.climb_ceiling(), Tier::Project);
    assert_eq!(
        decision,
        PromotionDecision::Hold,
        "permanent is unreachable, however much corroboration is piled on"
    );

    // The memory manager, which *does* reach permanent, cannot endorse at all.
    let curator = bench.stamp(
        DevIdentity::MemoryManager,
        "human:dave",
        None,
        EvidenceType::Observed,
        "s4",
    );
    let err = bench
        .store
        .endorse_procedure(proc.id, &curator)
        .expect_err("the memory manager holds Curate, not Endorse");
    assert!(format!("{err}").contains("lacks the endorse op"), "{err}");
}

/// **Finding 4 — two of the three diversity axes are computed but never weighed.**
///
/// §8.1's proxy is "distinct delegation-chain roots, distinct sessions spread
/// over time, distinct evidence types". The store computes all three;
/// `PromotionConfig` reads only the first. A single burst — two roots, one
/// session, one evidence type — promotes exactly as readily as corroboration
/// accumulated across sessions from varied evidence.
#[test]
fn finding_session_and_evidence_diversity_do_not_affect_the_verdict() {
    let mut bench = Bench::new();
    let proc = procedure("project:alpha", "release");
    for (root, author) in [
        ("human:alice", "project:lead/sub:1"),
        ("human:bob", "project:lead/sub:2"),
    ] {
        let stamp = bench.stamp(
            DevIdentity::ProjectLeader,
            root,
            Some(author),
            EvidenceType::Observed, // one evidence type
            "one-burst",            // one session
        );
        bench.write(&proc, Tier::Task, &stamp);
    }

    let (decision, facts) = bench.verdict(proc.id);
    assert_eq!(facts.distinct_sessions, 1);
    assert_eq!(facts.distinct_evidence_types, 1);
    assert_eq!(
        decision,
        PromotionDecision::Climb(Tier::Project),
        "neither axis is consulted"
    );
}
