//! Goal module - a navigational decomposition node (procedural memory, Phase 2).
//!
//! A [`Goal`] is a first-class, **skinny** navigation node: it says *how a
//! high-level goal decomposes* toward executable procedures, but carries no
//! executable body itself. Goals form a **DAG** (not a tree) — one goal such as
//! `cook-eggs` can be reachable under both `prepare-breakfast` and
//! `quick-dinner`.
//!
//! Decomposition lives on **edges** ([`GoalEdge`]), not inside the goal row.
//! Each edge points at a child that is *either* a sub-goal *or* a
//! [`crate::Procedure`], and carries the edge-local signals needed to filter and
//! rank a hop **without fetching the child row**: preconditions, `context_tags`,
//! `usage_notes`, a `role`, and a cached effectiveness. See
//! `docs/architecture/15-procedural-memory.md` §3.2 and §4.
//!
//! Retrieval is stateless, agent-driven recursive descent (§4): the agent holds
//! the cursor and calls [`expand`](crate::goal) one level at a time. The store's
//! job is to **surface** a deterministic candidate set, never to decide.

use crate::{Precondition, ProcedureId, Tier};
use std::fmt;

/// Unique identifier for a goal, based on UUIDv7 (per ADR-011).
///
/// Mirrors [`crate::ClaimId`] / [`crate::ProcedureId`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GoalId(u128);

impl GoalId {
    /// Generate a new UUIDv7-based `GoalId`.
    pub fn new() -> Self {
        Self(uuid::Uuid::now_v7().as_u128())
    }

    /// Create a `GoalId` from a raw `u128` value (for storage deserialization).
    pub fn from_value(value: u128) -> Self {
        Self(value)
    }

    /// Parse a `GoalId` from a UUIDv7 string.
    pub fn from_string(s: &str) -> Result<Self, String> {
        uuid::Uuid::parse_str(s)
            .map(|u| Self(u.as_u128()))
            .map_err(|e| format!("Invalid UUIDv7 string: {}", e))
    }

    /// Get the raw `u128` value.
    pub fn value(&self) -> u128 {
        self.0
    }

    /// Get the timestamp component of the UUIDv7 (milliseconds since Unix epoch).
    pub fn timestamp(&self) -> u64 {
        (self.0 >> 80) as u64
    }
}

impl Default for GoalId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for GoalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", uuid::Uuid::from_u128(self.0))
    }
}

/// A skinny navigational decomposition node.
///
/// Deliberately holds no children and no executable body — traversal touches
/// these rows, so they stay cheap to walk. Children live in [`GoalEdge`] rows
/// keyed by parent.
#[derive(Debug, Clone, PartialEq)]
pub struct Goal {
    /// Unique identifier.
    pub id: GoalId,
    /// Namespace for organization (per ADR-006).
    pub namespace: String,
    /// Human-readable name, e.g. `"prepare-breakfast"`.
    pub name: String,
    /// Embedded intent text — the semantic match key for the top-level entry hop.
    pub intent: String,
    /// Postconditions that define the target ("definition of done"), prose in
    /// Phase 2.
    pub definition_of_done: Vec<String>,
    /// Current tier (reuses the claim lifecycle tiers).
    pub tier: Tier,
    /// When this goal was created (Unix ms).
    pub created_at: u64,
    /// When this goal was last updated (Unix ms).
    pub updated_at: u64,
    /// When this goal should be considered stale (Unix ms), if set.
    pub stale_at: Option<u64>,
}

/// The kind of entity a [`GoalEdge`] points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChildKind {
    /// The child is another [`Goal`] (a sub-goal — descend further).
    Goal,
    /// The child is a [`crate::Procedure`] (an executable leaf).
    Procedure,
}

impl ChildKind {
    /// Get the kind name as a stable string (for storage).
    pub fn as_str(&self) -> &'static str {
        match self {
            ChildKind::Goal => "goal",
            ChildKind::Procedure => "procedure",
        }
    }

    /// Parse a kind from its string form.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "goal" => Some(ChildKind::Goal),
            "procedure" => Some(ChildKind::Procedure),
            _ => None,
        }
    }
}

/// A typed reference to an edge's child: either a sub-goal or a procedure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChildRef {
    /// A sub-goal to descend into.
    Goal(GoalId),
    /// A procedure to (eventually) execute.
    Procedure(ProcedureId),
}

impl ChildRef {
    /// The child's kind.
    pub fn kind(&self) -> ChildKind {
        match self {
            ChildRef::Goal(_) => ChildKind::Goal,
            ChildRef::Procedure(_) => ChildKind::Procedure,
        }
    }

    /// The child's raw `u128` id value (used as a deterministic tie-break).
    pub fn id_value(&self) -> u128 {
        match self {
            ChildRef::Goal(id) => id.value(),
            ChildRef::Procedure(id) => id.value(),
        }
    }

    /// Reconstruct a `ChildRef` from a stored kind + raw id value.
    pub fn from_parts(kind: ChildKind, id_value: u128) -> Self {
        match kind {
            ChildKind::Goal => ChildRef::Goal(GoalId::from_value(id_value)),
            ChildKind::Procedure => ChildRef::Procedure(ProcedureId::from_value(id_value)),
        }
    }
}

/// The role a child edge plays under its parent goal (design §3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeRole {
    /// A way to make progress on the parent goal.
    Accomplish,
    /// A procedure that helps *choose* among the `Accomplish` candidates. A
    /// decision-aid is not a new type — it is a [`crate::Procedure`] surfaced
    /// alongside the candidates it ranks.
    Decide,
}

impl EdgeRole {
    /// Get the role name as a stable string (for storage).
    pub fn as_str(&self) -> &'static str {
        match self {
            EdgeRole::Accomplish => "accomplish",
            EdgeRole::Decide => "decide",
        }
    }

    /// Parse a role from its string form.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "accomplish" => Some(EdgeRole::Accomplish),
            "decide" => Some(EdgeRole::Decide),
            _ => None,
        }
    }
}

/// A self-describing child edge under a parent goal.
///
/// Carries everything a hop needs to filter and rank the child **without
/// fetching the child row** (§4.2): edge-local `preconditions` (contextual
/// gating for *this* placement), `context_tags`, `usage_notes`, the `role`, and
/// a `cached_effectiveness` maintained out-of-band (e.g. by a Janitor sweep or
/// [`recompute`](crate::goal)).
#[derive(Debug, Clone, PartialEq)]
pub struct GoalEdge {
    /// The parent goal this edge hangs under.
    pub parent: GoalId,
    /// The child (a sub-goal or a procedure).
    pub child: ChildRef,
    /// The child's role under this parent.
    pub role: EdgeRole,
    /// Edge-local preconditions gating this placement (resolved against claims).
    pub preconditions: Vec<Precondition>,
    /// Edge-contextual selection tags, e.g. `"time:quick"`, `"ldl:low"`.
    pub context_tags: Vec<String>,
    /// Prose usage notes specific to choosing this child here.
    pub usage_notes: String,
    /// Cached effectiveness for ranking (0.0..=1.0), maintained out-of-band so a
    /// hop need not fetch the child row.
    pub cached_effectiveness: f64,
}

/// The context reference an agent passes to `expand` (design §4.1).
///
/// Phase 2 models the situational `context_tags` used to rank candidates; the
/// weighting of one factor against another is never in the store — it lives with
/// the agent (or an agent-run `decide` procedure).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TraversalContext {
    /// The tags describing the current situation, e.g. `["time:quick"]`.
    pub context_tags: Vec<String>,
}

/// A raw factor reading surfaced by `expand`: a claim consulted while resolving
/// the candidates' preconditions, returned so the agent can see *why* (§4.1).
#[derive(Debug, Clone, PartialEq)]
pub struct FactorReading {
    /// Claim subject.
    pub subject: String,
    /// Claim predicate.
    pub predicate: String,
    /// Claim object.
    pub object: String,
    /// Claim confidence interval `[lower, upper]`.
    pub confidence: (f64, f64),
}

/// One ranked entry in an [`ExpandResult`].
#[derive(Debug, Clone, PartialEq)]
pub struct ExpandedCandidate {
    /// The child this edge points at.
    pub child: ChildRef,
    /// The child's role under the parent.
    pub role: EdgeRole,
    /// The edge's context tags.
    pub context_tags: Vec<String>,
    /// The edge's usage notes.
    pub usage_notes: String,
    /// The effectiveness used for ranking.
    pub effectiveness: f64,
    /// How many of the edge's `context_tags` matched the traversal context.
    pub context_match: usize,
}

/// The deterministic candidate surface returned by a single `expand` hop (§4.1).
///
/// **Surface, not decide.** Candidates are precondition-filtered and ranked by
/// effectiveness then context match; the agent (or a `decide` procedure) applies
/// the actual weighting.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExpandResult {
    /// Ranked `Accomplish` children whose edge preconditions currently hold.
    pub candidates: Vec<ExpandedCandidate>,
    /// `Decide` children (procedures that help choose) whose preconditions hold.
    pub decision_aids: Vec<ExpandedCandidate>,
    /// Raw claim readings consulted while resolving the surfaced edges.
    pub factor_readings: Vec<FactorReading>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_ref_roundtrips_through_parts() {
        let g = ChildRef::Goal(GoalId::from_value(7));
        let p = ChildRef::Procedure(ProcedureId::from_value(9));
        assert_eq!(g.kind(), ChildKind::Goal);
        assert_eq!(p.kind(), ChildKind::Procedure);
        assert_eq!(ChildRef::from_parts(ChildKind::Goal, 7), g);
        assert_eq!(ChildRef::from_parts(ChildKind::Procedure, 9), p);
        assert_eq!(g.id_value(), 7);
        assert_eq!(p.id_value(), 9);
    }

    #[test]
    fn enum_string_roundtrips() {
        for k in [ChildKind::Goal, ChildKind::Procedure] {
            assert_eq!(ChildKind::parse(k.as_str()), Some(k));
        }
        for r in [EdgeRole::Accomplish, EdgeRole::Decide] {
            assert_eq!(EdgeRole::parse(r.as_str()), Some(r));
        }
        assert!(ChildKind::parse("nope").is_none());
        assert!(EdgeRole::parse("nope").is_none());
    }
}
