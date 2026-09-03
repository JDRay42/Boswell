//! The provenance-stamped write path (procedural memory, Phase 3).
//!
//! The one rule (design §5): **nothing writes directly to shared memory.** Every
//! write — a procedure, a goal edge, or an effectiveness/report update — carries a
//! [`ProvenanceStamp`], enters at the lowest tier its author is entitled to, and
//! climbs only by earning it. This module is the pure, dependency-light core of
//! that model: the trust-typed evidence and assurance ladders, an authority
//! scope, the stamp itself, the [`entry_tier`] formula, and [`CorroborationFacts`]
//! (the read-model the Gatekeeper weighs when deciding promotion).
//!
//! The *policy* (thresholds for climbing/falling) lives in the Gatekeeper; the
//! *facts* it weighs are computed by the store. Assurance values become
//! meaningful once the `IdentityProvider` port lands in Phase 4; here they are
//! carried and their ceiling is enforced, so plugging in a real provider later
//! needs no change to the ceiling formula. See
//! `docs/architecture/15-procedural-memory.md` §5 and §6.

use crate::Tier;

/// The trust-type of the evidence behind a write (design §5.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceType {
    /// Directly observed by the author.
    Observed,
    /// Inferred/synthesized from other memory.
    Inferred,
    /// Reported second-hand by another principal.
    Reported,
    /// Emitted by a tool the author invoked.
    ToolOutput,
}

impl EvidenceType {
    /// Get the evidence name as a stable string (for storage).
    pub fn as_str(&self) -> &'static str {
        match self {
            EvidenceType::Observed => "observed",
            EvidenceType::Inferred => "inferred",
            EvidenceType::Reported => "reported",
            EvidenceType::ToolOutput => "tool_output",
        }
    }

    /// Parse an evidence type from its string form.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "observed" => Some(EvidenceType::Observed),
            "inferred" => Some(EvidenceType::Inferred),
            "reported" => Some(EvidenceType::Reported),
            "tool_output" => Some(EvidenceType::ToolOutput),
            _ => None,
        }
    }

    /// The highest tier an entry may reach on this evidence *alone* (design §5.2):
    /// `tool_output` and `reported` cannot reach team tier without corroboration.
    pub fn tier_ceiling(&self) -> Tier {
        match self {
            EvidenceType::Observed => Tier::Permanent,
            EvidenceType::Inferred => Tier::Project,
            EvidenceType::Reported => Tier::Task,
            EvidenceType::ToolOutput => Tier::Task,
        }
    }

    /// Rank for picking the strongest evidence across stamps (higher = stronger).
    fn strength(&self) -> u8 {
        match self {
            EvidenceType::ToolOutput => 0,
            EvidenceType::Reported => 1,
            EvidenceType::Inferred => 2,
            EvidenceType::Observed => 3,
        }
    }

    /// The stronger of two evidence types (ties return `self`).
    pub fn stronger(self, other: Self) -> Self {
        if other.strength() > self.strength() {
            other
        } else {
            self
        }
    }
}

/// Identity assurance from the `IdentityProvider` (design §6). The tier ceiling is
/// a function of it: `permanent` requires `Attested`, `ephemeral` accepts
/// `Asserted`. Phase 4 supplies real values; Phase 3 carries and enforces them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Assurance {
    /// No identity backend; self-claimed at best.
    None,
    /// Self-asserted identity.
    Asserted,
    /// Verified by a provider (e.g. OIDC + mTLS).
    Verified,
    /// Cryptographically attested (e.g. SPIFFE/SPIRE SVID).
    Attested,
}

impl Assurance {
    /// Get the assurance name as a stable string (for storage).
    pub fn as_str(&self) -> &'static str {
        match self {
            Assurance::None => "none",
            Assurance::Asserted => "asserted",
            Assurance::Verified => "verified",
            Assurance::Attested => "attested",
        }
    }

    /// Parse an assurance from its string form.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "none" => Some(Assurance::None),
            "asserted" => Some(Assurance::Asserted),
            "verified" => Some(Assurance::Verified),
            "attested" => Some(Assurance::Attested),
            _ => None,
        }
    }

    /// The highest tier an entry may reach at this assurance (design §6).
    pub fn tier_ceiling(&self) -> Tier {
        match self {
            Assurance::None => Tier::Ephemeral,
            Assurance::Asserted => Tier::Task,
            Assurance::Verified => Tier::Project,
            Assurance::Attested => Tier::Permanent,
        }
    }

    /// Rank for picking the strongest assurance across stamps.
    fn strength(&self) -> u8 {
        match self {
            Assurance::None => 0,
            Assurance::Asserted => 1,
            Assurance::Verified => 2,
            Assurance::Attested => 3,
        }
    }

    /// The stronger of two assurances (ties return `self`).
    pub fn stronger(self, other: Self) -> Self {
        if other.strength() > self.strength() {
            other
        } else {
            self
        }
    }
}

/// An operation an authority may exercise over memory (design §6, §7.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    /// Read memory.
    Read,
    /// Write (advocate) memory.
    Write,
    /// Endorse a lower-authority write so it may climb a tier.
    Endorse,
    /// Curate: promote/demote/forget/GC, resolve contradictions.
    Curate,
}

impl Op {
    /// Get the op name as a stable string (for storage).
    pub fn as_str(&self) -> &'static str {
        match self {
            Op::Read => "read",
            Op::Write => "write",
            Op::Endorse => "endorse",
            Op::Curate => "curate",
        }
    }

    /// Parse an op from its string form.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "read" => Some(Op::Read),
            "write" => Some(Op::Write),
            "endorse" => Some(Op::Endorse),
            "curate" => Some(Op::Curate),
            _ => None,
        }
    }
}

/// What a principal is authorized to do (design §6): the namespaces it may act in,
/// the highest tier it may land an entry at, and the ops it may exercise.
///
/// Boswell **authorizes** (this type); the `IdentityProvider` only authenticates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Authority {
    /// Namespaces the principal may act in. An entry of `"*"` or `""` grants all.
    pub namespaces: Vec<String>,
    /// The highest tier this principal may land an entry at.
    pub max_tier: Tier,
    /// The operations this principal may exercise.
    pub ops: Vec<Op>,
}

impl Authority {
    /// Whether this authority permits acting on `target` namespace. A namespace
    /// entry of `""`/`"*"` is unrestricted; otherwise `target` must equal the
    /// entry or be a child of it (`"<entry>:..."`).
    pub fn allows_namespace(&self, target: &str) -> bool {
        self.namespaces.iter().any(|ns| {
            ns.is_empty() || ns == "*" || target == ns || target.starts_with(&format!("{}:", ns))
        })
    }

    /// Whether this authority may exercise `op`.
    pub fn can(&self, op: Op) -> bool {
        self.ops.contains(&op)
    }
}

/// The on-behalf-of delegation path, root first, e.g.
/// `human:jd -> agent:orch-7 -> sub:explore-3` (design §5.1).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DelegationChain(pub Vec<String>);

impl DelegationChain {
    /// The root principal (the ultimate on-behalf-of), if any.
    pub fn root(&self) -> Option<&str> {
        self.0.first().map(String::as_str)
    }

    /// The leaf principal (the immediate actor), if any.
    pub fn leaf(&self) -> Option<&str> {
        self.0.last().map(String::as_str)
    }
}

/// The provenance stamp attached to every write (design §5.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvenanceStamp {
    /// Stable agent identity, e.g. `agent:orch-7/sub:explore-3`.
    pub author: String,
    /// The on-behalf-of delegation path.
    pub delegation_chain: DelegationChain,
    /// What the author is authorized to do.
    pub authority: Authority,
    /// The trust-type of the evidence behind this write.
    pub evidence: EvidenceType,
    /// Identity assurance from the `IdentityProvider`.
    pub assurance: Assurance,
    /// Correlation: task id, if any.
    pub task_id: Option<String>,
    /// Correlation: session id, if any.
    pub session_id: Option<String>,
    /// When the write happened (Unix ms).
    pub timestamp: u64,
}

impl ProvenanceStamp {
    /// The ceiling this single stamp imposes: the min of the author's authority
    /// `max_tier`, the evidence ceiling, and the assurance ceiling.
    pub fn ceiling(&self) -> Tier {
        self.authority
            .max_tier
            .min(self.evidence.tier_ceiling())
            .min(self.assurance.tier_ceiling())
    }
}

/// The tier a stamped write may **enter** at (design §5.2):
/// `min(requested, author.max_tier, evidence_ceiling, assurance_ceiling)`.
///
/// A leaf subagent physically cannot land a project/team-tier entry.
pub fn entry_tier(requested: Tier, stamp: &ProvenanceStamp) -> Tier {
    requested.min(stamp.ceiling())
}

/// The provenance-derived facts the Gatekeeper weighs to decide a tier change
/// (design §5.2). The store computes the provenance fields; lifecycle fields
/// (`contradicted_by_higher_authority`, `failing`, `stale`) are set by the caller
/// (e.g. the Janitor sweep) from its own policy.
#[derive(Debug, Clone, PartialEq)]
pub struct CorroborationFacts {
    /// The entry's current tier.
    pub current_tier: Tier,
    /// Count of distinct authors who wrote or endorsed the entry.
    pub distinct_authors: usize,
    /// Highest authority tier among endorsers (holders of `Endorse`), if any.
    pub endorsed_max_tier: Option<Tier>,
    /// Highest authority `max_tier` among the entry's writers.
    pub author_max_tier: Tier,
    /// Strongest evidence type across the entry's stamps.
    pub best_evidence: EvidenceType,
    /// Strongest assurance across the entry's stamps.
    pub best_assurance: Assurance,
    /// Derived effectiveness (procedures; `0.0` for entries without it).
    pub effectiveness: f64,
    /// A higher-authority contradiction is on record.
    pub contradicted_by_higher_authority: bool,
    /// The entry is failing at its serving tier (repeated failure).
    pub failing: bool,
    /// The entry has decayed / gone stale.
    pub stale: bool,
}

impl CorroborationFacts {
    /// The effective tier ceiling for a *climb*: bounded by evidence and assurance
    /// (properties of the strongest stamp), and by authority — where a
    /// higher-authority endorsement can raise the authority bound above the
    /// original author's own `max_tier` (design §5.2, "climbs when a
    /// higher-authority parent endorses").
    pub fn climb_ceiling(&self) -> Tier {
        let authority_ceiling = match self.endorsed_max_tier {
            Some(endorsed) => self.author_max_tier.max(endorsed),
            None => self.author_max_tier,
        };
        self.best_evidence
            .tier_ceiling()
            .min(self.best_assurance.tier_ceiling())
            .min(authority_ceiling)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn authority(max_tier: Tier, ops: &[Op]) -> Authority {
        Authority {
            namespaces: vec!["person:jd".into()],
            max_tier,
            ops: ops.to_vec(),
        }
    }

    fn stamp(evidence: EvidenceType, assurance: Assurance, max_tier: Tier) -> ProvenanceStamp {
        ProvenanceStamp {
            author: "agent:worker".into(),
            delegation_chain: DelegationChain(vec!["human:jd".into(), "agent:worker".into()]),
            authority: authority(max_tier, &[Op::Write]),
            evidence,
            assurance,
            task_id: None,
            session_id: None,
            timestamp: 1,
        }
    }

    #[test]
    fn entry_tier_is_min_of_all_ceilings() {
        // Requests permanent but assurance (Asserted -> Task) caps it.
        let s = stamp(EvidenceType::Observed, Assurance::Asserted, Tier::Permanent);
        assert_eq!(entry_tier(Tier::Permanent, &s), Tier::Task);

        // Authority is the binding constraint here.
        let s = stamp(EvidenceType::Observed, Assurance::Attested, Tier::Ephemeral);
        assert_eq!(entry_tier(Tier::Permanent, &s), Tier::Ephemeral);

        // Evidence (tool_output -> Task) is the binding constraint.
        let s = stamp(
            EvidenceType::ToolOutput,
            Assurance::Attested,
            Tier::Permanent,
        );
        assert_eq!(entry_tier(Tier::Permanent, &s), Tier::Task);

        // Requested below every ceiling wins.
        let s = stamp(EvidenceType::Observed, Assurance::Attested, Tier::Permanent);
        assert_eq!(entry_tier(Tier::Ephemeral, &s), Tier::Ephemeral);
    }

    #[test]
    fn namespace_and_ops_authorization() {
        let a = authority(Tier::Task, &[Op::Read, Op::Write]);
        assert!(a.allows_namespace("person:jd"));
        assert!(a.allows_namespace("person:jd:sub"));
        assert!(!a.allows_namespace("person:kim"));
        assert!(a.can(Op::Write));
        assert!(!a.can(Op::Endorse));

        let wild = Authority {
            namespaces: vec!["*".into()],
            max_tier: Tier::Permanent,
            ops: vec![Op::Curate],
        };
        assert!(wild.allows_namespace("anything:here"));
    }

    #[test]
    fn climb_ceiling_uses_strongest_and_endorsement() {
        // Author capped at task, but a project-tier endorser raises the authority
        // bound; evidence/assurance both permit permanent -> ceiling = project.
        let facts = CorroborationFacts {
            current_tier: Tier::Task,
            distinct_authors: 2,
            endorsed_max_tier: Some(Tier::Project),
            author_max_tier: Tier::Task,
            best_evidence: EvidenceType::Observed,
            best_assurance: Assurance::Attested,
            effectiveness: 0.0,
            contradicted_by_higher_authority: false,
            failing: false,
            stale: false,
        };
        assert_eq!(facts.climb_ceiling(), Tier::Project);
    }

    #[test]
    fn climb_ceiling_bounded_by_weak_evidence() {
        let facts = CorroborationFacts {
            current_tier: Tier::Task,
            distinct_authors: 5,
            endorsed_max_tier: Some(Tier::Permanent),
            author_max_tier: Tier::Permanent,
            best_evidence: EvidenceType::ToolOutput, // caps at task
            best_assurance: Assurance::Attested,
            effectiveness: 0.99,
            contradicted_by_higher_authority: false,
            failing: false,
            stale: false,
        };
        assert_eq!(facts.climb_ceiling(), Tier::Task);
    }

    #[test]
    fn evidence_and_assurance_string_roundtrips() {
        for e in [
            EvidenceType::Observed,
            EvidenceType::Inferred,
            EvidenceType::Reported,
            EvidenceType::ToolOutput,
        ] {
            assert_eq!(EvidenceType::parse(e.as_str()), Some(e));
        }
        for a in [
            Assurance::None,
            Assurance::Asserted,
            Assurance::Verified,
            Assurance::Attested,
        ] {
            assert_eq!(Assurance::parse(a.as_str()), Some(a));
        }
        for o in [Op::Read, Op::Write, Op::Endorse, Op::Curate] {
            assert_eq!(Op::parse(o.as_str()), Some(o));
        }
    }

    #[test]
    fn delegation_chain_root_and_leaf() {
        let c = DelegationChain(vec![
            "human:jd".into(),
            "agent:orch".into(),
            "sub:explore".into(),
        ]);
        assert_eq!(c.root(), Some("human:jd"));
        assert_eq!(c.leaf(), Some("sub:explore"));
        assert_eq!(DelegationChain::default().root(), None);
    }
}
