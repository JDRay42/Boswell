//! Trait definitions for external interactions
//!
//! These traits define the boundaries between domain logic and infrastructure.
//! Infrastructure implementations live in other crates.

use crate::{
    Claim, ClaimId, ExecutionReceipt, ExpandResult, Goal, GoalId, GoalQuery, OutcomeReport,
    Procedure, ProcedureId, ProcedureQuery, ProvenanceStamp, ReceiptReportOutcome, Relationship,
    StoredReceipt, TraversalContext,
};

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

    /// Count the claims [`ClaimStore::query_claims`] would return for `query`,
    /// without materializing them.
    ///
    /// The contract is exactly that equivalence, `query.limit` included: a count
    /// under a limit is the number of rows that would come back, not the number
    /// that match. The default implementation runs the query and takes its
    /// length, so a store that cannot count cheaply is still correct; stores
    /// with a backend that can count override it.
    fn count_claims(&self, query: &ClaimQuery) -> Result<u64, Self::Error> {
        Ok(self.query_claims(query)?.len() as u64)
    }

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

/// Trait for retrieving procedures and running the effectiveness-reporting
/// contract (design §3.3, §4.1).
///
/// Split from [`ClaimStore`] so the claim substrate stays independent of
/// procedural memory, but taken as a supertrait so implementors share one error
/// type and the transport layer can take a single store handle.
///
/// Every method has a default implementation that behaves as "this store holds
/// no procedures", so claim-only and mock stores compile unchanged. Stores that
/// do hold procedures override them and report
/// [`supports_procedures`](ProcedureStore::supports_procedures) as `true`.
pub trait ProcedureStore: ClaimStore {
    /// Whether this store can serve procedures and receipts.
    ///
    /// Defaults to `false`; the transport layer uses this to answer
    /// `Unimplemented` rather than silently returning empty results.
    fn supports_procedures(&self) -> bool {
        false
    }

    /// Fetch a procedure by id, or `None` if it does not exist.
    fn get_procedure(&self, _id: ProcedureId) -> Result<Option<Procedure>, Self::Error> {
        Ok(None)
    }

    /// Retrieve procedures matching `query`, ranked by effectiveness. `now` is
    /// Unix ms, used to decay effectiveness for ranking.
    fn query_procedures(
        &self,
        _query: &ProcedureQuery,
        _now: u64,
    ) -> Result<Vec<Procedure>, Self::Error> {
        Ok(Vec::new())
    }

    /// Record a newly issued execution receipt as `pending`.
    ///
    /// Retrieving a procedure for execution carries an obligation to report the
    /// outcome, so a store that hands out procedures must also issue receipts.
    fn issue_receipt(&mut self, _receipt: &ExecutionReceipt) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Fetch a receipt by id, or `None` if it does not exist.
    fn get_receipt(&self, _receipt_id: ProcedureId) -> Result<Option<StoredReceipt>, Self::Error> {
        Ok(None)
    }

    /// Apply an outcome report against a pending receipt as a gatekept,
    /// provenance-stamped write. Returns `None` if no such receipt exists.
    /// `now` is Unix ms.
    fn report_receipt(
        &mut self,
        _receipt_id: ProcedureId,
        _report: &OutcomeReport,
        _stamp: &ProvenanceStamp,
        _now: u64,
    ) -> Result<Option<ReceiptReportOutcome>, Self::Error> {
        Ok(None)
    }
}

/// Trait for retrieving goals and running the single-hop traversal surface
/// (design §3.2, §4.1).
///
/// Split from [`ClaimStore`] for the same reason [`ProcedureStore`] is, and
/// taken as a supertrait so implementors share one error type and the transport
/// can hold a single store handle.
///
/// Every method has a default implementation that behaves as "this store holds
/// no goals", so claim-only and mock stores compile unchanged. Stores that do
/// hold goals override them and report [`supports_goals`](GoalStore::supports_goals)
/// as `true`.
///
/// Traversal is **stateless and agent-driven** (§4): each call is one hop, the
/// agent holds the cursor, and the store keeps no descent state.
pub trait GoalStore: ClaimStore {
    /// Whether this store can serve goals and traversal.
    ///
    /// Defaults to `false`; the transport layer uses this to answer
    /// `Unimplemented` rather than silently returning an empty surface, which
    /// would be indistinguishable from a childless goal.
    fn supports_goals(&self) -> bool {
        false
    }

    /// Fetch a goal by id, or `None` if it does not exist.
    fn get_goal(&self, _id: GoalId) -> Result<Option<Goal>, Self::Error> {
        Ok(None)
    }

    /// Retrieve goals matching `query` — the entry hop into a decomposition.
    fn query_goals(&self, _query: &GoalQuery) -> Result<Vec<Goal>, Self::Error> {
        Ok(Vec::new())
    }

    /// Expand one goal into its deterministic candidate surface (§4.1): filter
    /// by edge-local preconditions, rank by effectiveness then context match,
    /// and return the survivors with their decision aids and factor readings.
    ///
    /// **Surface, not decide.** The weighting of one factor against another is
    /// never in the store. `now` is Unix ms.
    fn expand(
        &self,
        _goal_id: GoalId,
        _context: &TraversalContext,
        _now: u64,
    ) -> Result<ExpandResult, Self::Error> {
        Ok(ExpandResult::default())
    }
}

/// Query criteria for retrieving claims
#[derive(Debug, Clone, Default)]
pub struct ClaimQuery {
    /// Filter by namespace prefix
    pub namespace: Option<String>,

    /// Filter by exact subject
    pub subject: Option<String>,

    /// Filter by exact predicate
    pub predicate: Option<String>,

    /// Filter by exact object
    pub object: Option<String>,

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
