//! Claim module - the fundamental unit of Boswell's memory system

use std::fmt;

/// Unique identifier for a claim based on ULID (per ADR-011)
/// 
/// ULIDs provide:
/// - Chronological sortability for temporal queries
/// - 128-bit uniqueness
/// - Lexicographic ordering that matches creation time
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClaimId(u128);

impl ClaimId {
    /// Create a new ClaimId from a ULID value
    pub fn new(value: u128) -> Self {
        Self(value)
    }

    /// Get the raw ULID value
    pub fn value(&self) -> u128 {
        self.0
    }
}

impl fmt::Display for ClaimId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // TODO: Format as proper ULID string
        write!(f, "{:032x}", self.0)
    }
}

/// A claim - the fundamental unit of knowledge in Boswell
///
/// Per ADR-001, everything is a claim with confidence, not a fact.
/// Claims are immutable once created; updates create new claims.
#[derive(Debug, Clone, PartialEq)]
pub struct Claim {
    /// Unique identifier
    pub id: ClaimId,
    
    /// Namespace for organization (per ADR-006)
    pub namespace: String,
    
    /// Subject of the claim
    pub subject: String,
    
    /// Predicate/relationship
    pub predicate: String,
    
    /// Object of the claim
    pub object: String,
    
    /// Confidence interval [lower, upper] (per ADR-003)
    pub confidence: (f64, f64),
    
    /// Current tier (ephemeral, task, project, permanent)
    pub tier: String,
    
    /// When this claim was created (timestamp)
    pub created_at: u64,
    
    /// When this claim should be considered stale
    pub stale_at: Option<u64>,
}

impl Claim {
    /// Create a new claim
    pub fn new(
        id: ClaimId,
        namespace: String,
        subject: String,
        predicate: String,
        object: String,
        confidence: (f64, f64),
        tier: String,
        created_at: u64,
    ) -> Self {
        Self {
            id,
            namespace,
            subject,
            predicate,
            object,
            confidence,
            tier,
            created_at,
            stale_at: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_claim_id_ordering() {
        let id1 = ClaimId::new(1000);
        let id2 = ClaimId::new(2000);
        
        assert!(id1 < id2);
        assert!(id2 > id1);
    }
}
