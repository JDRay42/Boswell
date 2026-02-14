//! Trait definitions for external interactions
//!
//! These traits define the boundaries between domain logic and infrastructure.
//! Infrastructure implementations live in other crates.

use crate::{Claim, ClaimId, Relationship};

/// Trait for storing and retrieving claims
/// 
/// Implemented by the infrastructure layer (boswell-store)
pub trait ClaimStore {
    /// Error type for store operations
    type Error;

    /// Assert a new claim into the store
    fn assert_claim(&mut self, claim: Claim) -> Result<ClaimId, Self::Error>;

    /// Get a claim by ID
    fn get_claim(&self, id: ClaimId) -> Result<Option<Claim>, Self::Error>;

    /// Query claims matching criteria
    fn query_claims(&self, query: &ClaimQuery) -> Result<Vec<Claim>, Self::Error>;

    /// Add a relationship between claims
    fn add_relationship(&mut self, relationship: Relationship) -> Result<(), Self::Error>;

    /// Get relationships for a claim
    fn get_relationships(&self, id: ClaimId) -> Result<Vec<Relationship>, Self::Error>;
}

/// Query criteria for retrieving claims
#[derive(Debug, Clone, Default)]
pub struct ClaimQuery {
    /// Filter by namespace prefix
    pub namespace: Option<String>,
    
    /// Filter by tier
    pub tier: Option<String>,
    
    /// Filter by minimum confidence
    pub min_confidence: Option<f64>,
    
    /// Semantic search text (if supported)
    pub semantic_text: Option<String>,
    
    /// Maximum results to return
    pub limit: Option<usize>,
}

/// Trait for LLM provider operations
/// 
/// Implemented by the infrastructure layer (boswell-llm)
pub trait LlmProvider {
    /// Error type for LLM operations
    type Error;

    /// Generate text completion
    fn generate(&self, prompt: &str) -> Result<String, Self::Error>;

    /// Generate with structured output (if supported)
    fn generate_structured(&self, prompt: &str, schema: &str) -> Result<String, Self::Error>;
}

/// Trait for extracting claims from text
/// 
/// Implemented by the application layer (boswell-extractor)
pub trait Extractor {
    /// Error type for extraction operations
    type Error;

    /// Extract claims from unstructured text
    fn extract(&self, text: &str, namespace: &str) -> Result<Vec<Claim>, Self::Error>;
}
