//! Boswell Domain Layer
//!
//! This crate contains the core business logic and domain model for Boswell.
//! It has ZERO external dependencies (per ADR-004) and defines the fundamental
//! concepts, value objects, and trait interfaces that all other layers depend upon.
//!
//! ## Key Concepts
//!
//! - **Claim**: The fundamental unit - a statement with confidence, not a fact
//! - **Confidence Interval**: [lower, upper] bounds representing certainty
//! - **Provenance**: Source tracking for every claim
//! - **Relationships**: Pairwise connections between claims
//! - **Tiers**: Lifecycle stages (ephemeral → task → project → permanent)
//!
//! ## Architecture
//!
//! This crate follows Clean Architecture:
//! - No external crate dependencies
//! - Pure business logic only
//! - Infrastructure implementations live in other crates
//! - Trait definitions for all external interactions

#![warn(missing_docs)]
#![warn(clippy::all)]

pub mod claim;
pub mod confidence;
pub mod confidence_computation;
pub mod decay;
pub mod goal;
pub mod namespace;
pub mod procedure;
pub mod provenance;
pub mod relationship;
pub mod tier;
pub mod traits;

// Re-exports for convenience
pub use claim::{Claim, ClaimId};
pub use confidence::ConfidenceInterval;
pub use decay::{decay_factor, decayed_confidence, DecayConfig};
pub use goal::{
    ChildKind, ChildRef, EdgeRole, ExpandResult, ExpandedCandidate, FactorReading, Goal, GoalEdge,
    GoalId, TraversalContext,
};
pub use namespace::Namespace;
pub use procedure::{
    BodyFormat, ClaimMatch, ExecutionReceipt, Expect, FailureMode, Outcome, OutcomeReport,
    Parameter, Precondition, PreconditionCheck, Procedure, ProcedureId, ProcedureQuery,
    ProcedureSource, ReportEffect,
};
pub use provenance::ProvenanceEntry;
pub use relationship::{Relationship, RelationshipType};
pub use tier::Tier;
