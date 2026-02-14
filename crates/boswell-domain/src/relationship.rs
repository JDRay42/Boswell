//! Relationship module (per ADR-002 - pairwise relationships only)

use super::ClaimId;

/// Type of relationship between claims
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RelationshipType {
    /// One claim supports another (increases confidence)
    Supports,
    
    /// One claim contradicts another
    Contradicts,
    
    /// One claim was derived from another (synthesis)
    DerivedFrom,
    
    /// One claim references another
    References,
    
    /// One claim supersedes another (newer version)
    Supersedes,
}

/// A pairwise relationship between two claims
/// 
/// Per ADR-002, we only model pairwise relationships.
/// Compound relationships are handled by the Synthesizer creating derived claims.
#[derive(Debug, Clone, PartialEq)]
pub struct Relationship {
    /// Source claim ID
    pub from_claim: ClaimId,
    
    /// Target claim ID
    pub to_claim: ClaimId,
    
    /// Type of relationship
    pub relationship_type: RelationshipType,
    
    /// Strength of relationship [0.0, 1.0]
    pub strength: f64,
    
    /// When this relationship was established
    pub created_at: u64,
}

impl Relationship {
    /// Create a new relationship
    pub fn new(
        from_claim: ClaimId,
        to_claim: ClaimId,
        relationship_type: RelationshipType,
        strength: f64,
        created_at: u64,
    ) -> Self {
        assert!((0.0..=1.0).contains(&strength), "Strength must be in [0, 1]");
        
        Self {
            from_claim,
            to_claim,
            relationship_type,
            strength,
            created_at,
        }
    }
}
