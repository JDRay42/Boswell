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

    /// Search for claims semantically similar to `query_text`.
    ///
    /// Returns up to `limit` `(claim, similarity)` pairs whose similarity is at
    /// least `min_similarity` (cosine similarity in `[0.0, 1.0]`), ordered by
    /// similarity descending.
    ///
    /// The default implementation returns no results, so stores without a vector
    /// index (see [`ClaimStore::supports_semantic_search`]) degrade gracefully.
    fn semantic_search(
        &self,
        _query_text: &str,
        _limit: usize,
        _min_similarity: f32,
    ) -> Result<Vec<(Claim, f32)>, Self::Error> {
        Ok(Vec::new())
    }

    /// Whether this store can perform [`ClaimStore::semantic_search`].
    ///
    /// Defaults to `false`; stores backed by a vector index override this.
    fn supports_semantic_search(&self) -> bool {
        false
    }

    /// Delete a claim by id, returning `true` if a claim was removed.
    ///
    /// The default implementation is a no-op returning `false`, so read-only or
    /// mock stores compile unchanged; persistent stores override it. Deleting a
    /// claim should also remove its dependent rows (relationships, provenance,
    /// cached confidence).
    fn delete_claim(&mut self, _id: ClaimId) -> Result<bool, Self::Error> {
        Ok(false)
    }

    /// Move a claim to a new tier, returning `true` if a claim was updated.
    ///
    /// The default implementation is a no-op returning `false`; persistent stores
    /// override it. Changing a claim's tier changes its decay rate, so any cached
    /// effective confidence for the claim should be invalidated.
    fn update_claim_tier(&mut self, _id: ClaimId, _new_tier: &str) -> Result<bool, Self::Error> {
        Ok(false)
    }
}

/// Query criteria for retrieving claims
#[derive(Debug, Clone, Default)]
pub struct ClaimQuery {
    /// Filter by namespace prefix
    pub namespace: Option<String>,

    /// Filter by tier
    pub tier: Option<String>,

    /// Filter by exact `source_type` (e.g. `assertion`, `extraction`, `inference`, `import`)
    pub source_type: Option<String>,

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
