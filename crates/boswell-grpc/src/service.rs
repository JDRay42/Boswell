//! gRPC service implementation
//!
//! Implements the BosWellService trait generated from proto definitions.

use boswell_domain::traits::{ClaimQuery, ClaimStore};
use boswell_domain::{Claim, ClaimId};
use std::sync::{Arc, Mutex};
use tonic::{Request, Response, Status};

use crate::conversions::{
    claim_from_proto, claim_to_proto, confidence_from_proto, relationship_to_proto, tier_from_proto,
};
use crate::proto::bos_well_service_server::BosWellService;
use crate::proto::*;

/// Outcome of a server-side extraction pass, returned by a [`ServerExtractor`].
pub struct ExtractOutcome {
    /// Newly created claims, already persisted in the store.
    pub created: Vec<Claim>,
    /// Number of extracted claims that corroborated existing ones.
    pub corroborated_count: usize,
    /// Human-readable failure reasons for candidates that could not be stored.
    pub failures: Vec<String>,
}

/// A server-side text→claims extractor that the [`Extract`](BosWellService::extract)
/// RPC delegates to.
///
/// Implemented in `boswell-server` over the LLM-backed `boswell-extractor`,
/// sharing the same store as the gRPC service. It is kept as a trait object so
/// the service stays generic only over its store type `S` (the LLM provider type
/// does not leak into the service or server signatures).
#[tonic::async_trait]
pub trait ServerExtractor: Send + Sync {
    /// Extract claims from `text` into `namespace` at `tier`, tagging provenance
    /// with `source_id`. Returns the created claims and per-candidate outcomes.
    async fn extract(
        &self,
        text: String,
        namespace: String,
        tier: String,
        source_id: String,
    ) -> Result<ExtractOutcome, String>;
}

/// Implementation of the BosWellService
pub struct BosWellServiceImpl<S: ClaimStore> {
    store: Arc<Mutex<S>>,
    start_time: std::time::Instant,
    extractor: Option<Arc<dyn ServerExtractor>>,
}

impl<S: ClaimStore> BosWellServiceImpl<S> {
    /// Create a new service instance without a server-side extractor. The
    /// `Extract` RPC returns `FailedPrecondition` until one is attached with
    /// [`BosWellServiceImpl::with_extractor`].
    pub fn new(store: Arc<Mutex<S>>) -> Self {
        Self {
            store,
            start_time: std::time::Instant::now(),
            extractor: None,
        }
    }

    /// Attach a server-side extractor so the `Extract` RPC (and LLM-mode hook
    /// ingest) can turn text into claims.
    pub fn with_extractor(mut self, extractor: Arc<dyn ServerExtractor>) -> Self {
        self.extractor = Some(extractor);
        self
    }
}

#[tonic::async_trait]
impl<S> BosWellService for BosWellServiceImpl<S>
where
    // `Send` (not `Sync`) is sufficient: the store is only ever accessed through
    // `Arc<Mutex<S>>`, which is `Sync` whenever `S: Send`. Requiring `S: Sync`
    // would needlessly exclude stores like `SqliteStore` (rusqlite is `!Sync`).
    S: ClaimStore + Send + 'static,
    S::Error: std::fmt::Debug,
{
    async fn assert(
        &self,
        request: Request<AssertRequest>,
    ) -> Result<Response<AssertResponse>, Status> {
        let req = request.into_inner();

        // Validate authentication token (placeholder for now)
        if req.auth_token.is_empty() {
            return Err(Status::unauthenticated("Missing authentication token"));
        }

        // Convert proto types to domain types
        let confidence = confidence_from_proto(req.confidence)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let tier = if req.tier != 0 {
            tier_from_proto(
                Tier::try_from(req.tier).map_err(|_| Status::invalid_argument("Invalid tier"))?,
            )
            .map_err(|e| Status::invalid_argument(e.to_string()))?
        } else {
            "ephemeral".to_string() // Default tier
        };

        let created_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Create claim
        let claim = Claim {
            id: ClaimId::new(),
            namespace: req.namespace,
            subject: req.subject,
            predicate: req.predicate,
            object: req.object,
            source_type: Claim::SOURCE_ASSERTION.to_string(),
            confidence: (confidence.lower, confidence.upper),
            tier,
            created_at,
            stale_at: None,
        };

        // Assert claim to store
        let mut store = self.store.lock().unwrap();
        let result = store
            .assert_claim(claim.clone())
            .map_err(|e| Status::internal(format!("Failed to assert claim: {:?}", e)))?;

        Ok(Response::new(AssertResponse {
            claim_id: result.to_string(),
            is_duplicate: result == claim.id, // Simplified duplicate detection
            message: "Claim asserted successfully".to_string(),
        }))
    }

    async fn query(
        &self,
        request: Request<QueryRequest>,
    ) -> Result<Response<QueryResponse>, Status> {
        let req = request.into_inner();

        // Validate authentication token
        if req.auth_token.is_empty() {
            return Err(Status::unauthenticated("Missing authentication token"));
        }

        let filter = req
            .filter
            .ok_or_else(|| Status::invalid_argument("Missing filter"))?;

        // Build query
        let query = ClaimQuery {
            namespace: filter.namespace,
            tier: filter.tier.and_then(|t| {
                if t != 0 {
                    tier_from_proto(Tier::try_from(t).unwrap_or(Tier::Unspecified)).ok()
                } else {
                    None
                }
            }),
            source_type: filter.source_type.filter(|s| !s.trim().is_empty()),
            min_confidence: filter.min_confidence.filter(|&c| c > 0.0),
            semantic_text: None,
            limit: if req.limit > 0 {
                Some(req.limit as usize)
            } else {
                Some(100)
            },
        };

        // Query claims from store
        let store = self.store.lock().unwrap();
        let claims = store
            .query_claims(&query)
            .map_err(|e| Status::internal(format!("Query failed: {:?}", e)))?;

        // Apply additional filters (subject, predicate, object not in ClaimQuery yet)
        let filtered_claims: Vec<Claim> = claims
            .into_iter()
            .filter(|c| {
                if let Some(ref subject) = filter.subject {
                    if &c.subject != subject {
                        return false;
                    }
                }
                if let Some(ref predicate) = filter.predicate {
                    if &c.predicate != predicate {
                        return false;
                    }
                }
                if let Some(ref object) = filter.object {
                    if &c.object != object {
                        return false;
                    }
                }
                true
            })
            .collect();

        let total_count = filtered_claims.len() as i32;

        // Convert to proto
        let proto_claims = filtered_claims.into_iter().map(claim_to_proto).collect();

        Ok(Response::new(QueryResponse {
            claims: proto_claims,
            total_count,
            message: format!("Found {} claims", total_count),
        }))
    }

    async fn search(
        &self,
        request: Request<SearchRequest>,
    ) -> Result<Response<SearchResponse>, Status> {
        let req = request.into_inner();

        if req.auth_token.is_empty() {
            return Err(Status::unauthenticated("Missing authentication token"));
        }
        if req.query_text.trim().is_empty() {
            return Err(Status::invalid_argument("query_text must not be empty"));
        }

        let limit = if req.limit > 0 {
            req.limit as usize
        } else {
            10
        };
        let min_similarity = req.min_similarity.clamp(0.0, 1.0) as f32;

        let store = self.store.lock().unwrap();

        if !store.supports_semantic_search() {
            return Err(Status::failed_precondition(
                "Semantic search is not enabled on this instance",
            ));
        }

        // Fetch extra candidates when a namespace filter is applied, since the
        // post-filter may drop some of the top-k results.
        let fetch = if req.namespace.is_some() {
            limit * 4
        } else {
            limit
        };
        let hits = store
            .semantic_search(&req.query_text, fetch, min_similarity)
            .map_err(|e| Status::internal(format!("Search failed: {:?}", e)))?;

        let results: Vec<SearchResult> = hits
            .into_iter()
            .filter(|(claim, _)| match &req.namespace {
                Some(ns) => claim.namespace.starts_with(ns.as_str()),
                None => true,
            })
            .take(limit)
            .map(|(claim, similarity)| SearchResult {
                claim: Some(claim_to_proto(claim)),
                similarity: similarity as f64,
            })
            .collect();

        let total_count = results.len() as i32;

        Ok(Response::new(SearchResponse {
            results,
            total_count,
            message: format!("Found {} claims", total_count),
        }))
    }

    async fn learn(
        &self,
        request: Request<LearnRequest>,
    ) -> Result<Response<LearnResponse>, Status> {
        let req = request.into_inner();

        if req.auth_token.is_empty() {
            return Err(Status::unauthenticated("Missing authentication token"));
        }

        let mut inserted_count = 0;
        // Duplicates cannot be distinguished from other failures at the generic
        // ClaimStore layer (the error type is opaque), so they are reported under
        // error_count and this stays 0. Revisit if the trait gains a typed error.
        let duplicate_count = 0;
        let mut error_count = 0;
        let mut errors = Vec::new();

        let mut store = self.store.lock().unwrap();

        for proto_claim in req.claims {
            match claim_from_proto(proto_claim) {
                Ok(claim) => match store.assert_claim(claim.clone()) {
                    Ok(_) => inserted_count += 1,
                    Err(_) => {
                        error_count += 1;
                        errors.push(format!("Failed to insert claim {}", claim.id));
                    }
                },
                Err(e) => {
                    error_count += 1;
                    errors.push(format!("Invalid claim: {}", e));
                }
            }
        }

        Ok(Response::new(LearnResponse {
            inserted_count,
            duplicate_count,
            error_count,
            errors,
            message: format!("Inserted {} claims, {} errors", inserted_count, error_count),
        }))
    }

    async fn forget(
        &self,
        request: Request<ForgetRequest>,
    ) -> Result<Response<ForgetResponse>, Status> {
        let req = request.into_inner();

        if req.auth_token.is_empty() {
            return Err(Status::unauthenticated("Missing authentication token"));
        }

        let claim_id = ClaimId::from_string(&req.claim_id)
            .map_err(|e| Status::invalid_argument(format!("Invalid claim ID: {}", e)))?;

        // Real deletion: cascades to the claim's relationships, provenance, and
        // cached confidence (see `SqliteStore::delete_claim`).
        let mut store = self.store.lock().unwrap();
        match store.delete_claim(claim_id) {
            Ok(true) => Ok(Response::new(ForgetResponse {
                success: true,
                message: format!("Claim {} deleted", req.claim_id),
            })),
            Ok(false) => Ok(Response::new(ForgetResponse {
                success: false,
                message: "Claim not found".to_string(),
            })),
            Err(e) => Ok(Response::new(ForgetResponse {
                success: false,
                message: format!("Error deleting claim: {:?}", e),
            })),
        }
    }

    async fn get_claim(
        &self,
        request: Request<GetClaimRequest>,
    ) -> Result<Response<GetClaimResponse>, Status> {
        let req = request.into_inner();

        if req.auth_token.is_empty() {
            return Err(Status::unauthenticated("Missing authentication token"));
        }

        let claim_id = ClaimId::from_string(&req.claim_id)
            .map_err(|e| Status::invalid_argument(format!("Invalid claim ID: {}", e)))?;

        let store = self.store.lock().unwrap();
        match store
            .get_claim(claim_id)
            .map_err(|e| Status::internal(format!("Failed to get claim: {:?}", e)))?
        {
            Some(claim) => Ok(Response::new(GetClaimResponse {
                claim: Some(claim_to_proto(claim)),
                found: true,
            })),
            None => Ok(Response::new(GetClaimResponse {
                claim: None,
                found: false,
            })),
        }
    }

    async fn get_relationships(
        &self,
        request: Request<GetRelationshipsRequest>,
    ) -> Result<Response<GetRelationshipsResponse>, Status> {
        let req = request.into_inner();

        if req.auth_token.is_empty() {
            return Err(Status::unauthenticated("Missing authentication token"));
        }

        let claim_id = ClaimId::from_string(&req.claim_id)
            .map_err(|e| Status::invalid_argument(format!("Invalid claim ID: {}", e)))?;

        let store = self.store.lock().unwrap();
        let relationships = store
            .get_relationships(claim_id)
            .map_err(|e| Status::internal(format!("Failed to get relationships: {:?}", e)))?
            .into_iter()
            .map(relationship_to_proto)
            .collect();

        Ok(Response::new(GetRelationshipsResponse { relationships }))
    }

    async fn extract(
        &self,
        request: Request<ExtractRequest>,
    ) -> Result<Response<ExtractResponse>, Status> {
        let req = request.into_inner();

        if req.auth_token.is_empty() {
            return Err(Status::unauthenticated("Missing authentication token"));
        }

        let extractor = self.extractor.as_ref().ok_or_else(|| {
            Status::failed_precondition("Extraction is not enabled on this instance")
        })?;

        if req.text.trim().is_empty() {
            return Err(Status::invalid_argument("text must not be empty"));
        }
        if req.namespace.trim().is_empty() {
            return Err(Status::invalid_argument("namespace must not be empty"));
        }

        let tier = if req.tier.trim().is_empty() {
            "task".to_string()
        } else {
            req.tier
        };
        let source_id = if req.source_id.trim().is_empty() {
            "gateway:extract".to_string()
        } else {
            req.source_id
        };

        let outcome = extractor
            .extract(req.text, req.namespace, tier, source_id)
            .await
            .map_err(|e| Status::internal(format!("Extraction failed: {}", e)))?;

        let created_count = outcome.created.len() as i32;
        let corroborated_count = outcome.corroborated_count as i32;
        let failed_count = outcome.failures.len() as i32;
        let claims_created = outcome.created.into_iter().map(claim_to_proto).collect();

        Ok(Response::new(ExtractResponse {
            claims_created,
            created_count,
            corroborated_count,
            failed_count,
            failures: outcome.failures,
            message: format!("Extracted {} claims", created_count),
        }))
    }

    async fn health_check(
        &self,
        _request: Request<HealthCheckRequest>,
    ) -> Result<Response<HealthCheckResponse>, Status> {
        let store = self.store.lock().unwrap();
        let query = ClaimQuery::default();
        let claim_count = store
            .query_claims(&query)
            .map(|claims| claims.len() as i64)
            .unwrap_or(0);

        Ok(Response::new(HealthCheckResponse {
            status: health_check_response::Status::Healthy as i32,
            version: env!("CARGO_PKG_VERSION").to_string(),
            uptime_seconds: self.start_time.elapsed().as_secs() as i64,
            claim_count,
            message: "Service is healthy".to_string(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use boswell_domain::Relationship;

    // Mock store for testing
    struct MockStore;

    impl ClaimStore for MockStore {
        type Error = String;

        fn assert_claim(&mut self, claim: Claim) -> Result<ClaimId, Self::Error> {
            Ok(claim.id)
        }

        fn get_claim(&self, _id: ClaimId) -> Result<Option<Claim>, Self::Error> {
            Ok(Some(Claim {
                id: ClaimId::new(),
                namespace: "test".to_string(),
                subject: "Alice".to_string(),
                predicate: "knows".to_string(),
                object: "Bob".to_string(),
                source_type: "assertion".to_string(),
                confidence: (0.8, 0.95),
                tier: "task".to_string(),
                created_at: 1000000,
                stale_at: None,
            }))
        }

        fn query_claims(&self, _query: &ClaimQuery) -> Result<Vec<Claim>, Self::Error> {
            Ok(vec![Claim {
                id: ClaimId::new(),
                namespace: "test".to_string(),
                subject: "Alice".to_string(),
                predicate: "knows".to_string(),
                object: "Bob".to_string(),
                source_type: "assertion".to_string(),
                confidence: (0.8, 0.95),
                tier: "task".to_string(),
                created_at: 1000000,
                stale_at: None,
            }])
        }

        fn add_relationship(&mut self, _relationship: Relationship) -> Result<(), Self::Error> {
            Ok(())
        }

        fn get_relationships(&self, _id: ClaimId) -> Result<Vec<Relationship>, Self::Error> {
            Ok(vec![])
        }
    }

    // Mock store that supports semantic search, returning two canned hits in
    // different namespaces so namespace filtering can be exercised.
    struct SemanticMockStore;

    fn canned(namespace: &str, subject: &str) -> Claim {
        Claim {
            id: ClaimId::new(),
            namespace: namespace.to_string(),
            subject: subject.to_string(),
            predicate: "is_a".to_string(),
            object: "thing".to_string(),
            source_type: "assertion".to_string(),
            confidence: (0.8, 0.9),
            tier: "task".to_string(),
            created_at: 1,
            stale_at: None,
        }
    }

    impl ClaimStore for SemanticMockStore {
        type Error = String;
        fn assert_claim(&mut self, claim: Claim) -> Result<ClaimId, Self::Error> {
            Ok(claim.id)
        }
        fn get_claim(&self, _id: ClaimId) -> Result<Option<Claim>, Self::Error> {
            Ok(None)
        }
        fn query_claims(&self, _query: &ClaimQuery) -> Result<Vec<Claim>, Self::Error> {
            Ok(vec![])
        }
        fn add_relationship(&mut self, _r: Relationship) -> Result<(), Self::Error> {
            Ok(())
        }
        fn get_relationships(&self, _id: ClaimId) -> Result<Vec<Relationship>, Self::Error> {
            Ok(vec![])
        }
        fn supports_semantic_search(&self) -> bool {
            true
        }
        fn semantic_search(
            &self,
            _query_text: &str,
            _limit: usize,
            _min_similarity: f32,
        ) -> Result<Vec<(Claim, f32)>, Self::Error> {
            Ok(vec![
                (canned("lang", "rust"), 0.98),
                (canned("food", "banana"), 0.80),
            ])
        }
    }

    #[tokio::test]
    async fn test_search_requires_auth() {
        let service = BosWellServiceImpl::new(Arc::new(Mutex::new(SemanticMockStore)));
        let resp = service
            .search(Request::new(SearchRequest {
                query_text: "rust".to_string(),
                namespace: None,
                limit: 10,
                min_similarity: 0.5,
                auth_token: String::new(),
            }))
            .await;
        assert_eq!(resp.unwrap_err().code(), tonic::Code::Unauthenticated);
    }

    #[tokio::test]
    async fn test_search_unsupported_store() {
        let service = BosWellServiceImpl::new(Arc::new(Mutex::new(MockStore)));
        let resp = service
            .search(Request::new(SearchRequest {
                query_text: "rust".to_string(),
                namespace: None,
                limit: 10,
                min_similarity: 0.5,
                auth_token: "token".to_string(),
            }))
            .await;
        assert_eq!(resp.unwrap_err().code(), tonic::Code::FailedPrecondition);
    }

    #[tokio::test]
    async fn test_search_happy_path_and_namespace_filter() {
        let service = BosWellServiceImpl::new(Arc::new(Mutex::new(SemanticMockStore)));

        // No namespace filter → both hits returned, ordered by similarity.
        let all = service
            .search(Request::new(SearchRequest {
                query_text: "rust".to_string(),
                namespace: None,
                limit: 10,
                min_similarity: 0.5,
                auth_token: "token".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(all.results.len(), 2);
        assert!(all.results[0].similarity >= all.results[1].similarity);

        // Namespace filter keeps only the matching-prefix hit.
        let filtered = service
            .search(Request::new(SearchRequest {
                query_text: "rust".to_string(),
                namespace: Some("lang".to_string()),
                limit: 10,
                min_similarity: 0.5,
                auth_token: "token".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(filtered.results.len(), 1);
        assert_eq!(
            filtered.results[0].claim.as_ref().unwrap().namespace,
            "lang"
        );
    }

    #[tokio::test]
    async fn test_health_check() {
        let service = BosWellServiceImpl::new(Arc::new(Mutex::new(MockStore)));
        let request = Request::new(HealthCheckRequest {});

        let response = service.health_check(request).await.unwrap();
        let health = response.into_inner();

        assert_eq!(health.status, health_check_response::Status::Healthy as i32);
        // health_check counts claims via query_claims; MockStore returns exactly
        // one canned claim, so the count path is actually verified (not just >= 0).
        assert_eq!(health.claim_count, 1);
    }

    // ---- Tests exercising the new RPCs against a real in-memory store ----

    use boswell_store::SqliteStore;

    fn sqlite_service() -> BosWellServiceImpl<SqliteStore> {
        let store = SqliteStore::new(":memory:", false, 0).unwrap();
        BosWellServiceImpl::new(Arc::new(Mutex::new(store)))
    }

    async fn assert_one(service: &BosWellServiceImpl<SqliteStore>, subject: &str) -> String {
        let resp = service
            .assert(Request::new(AssertRequest {
                namespace: "test".to_string(),
                subject: subject.to_string(),
                predicate: "knows".to_string(),
                object: "Bob".to_string(),
                confidence: Some(ConfidenceInterval {
                    lower: 0.8,
                    upper: 0.9,
                }),
                tier: Tier::Task as i32,
                provenance: vec![],
                auth_token: "token".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();
        resp.claim_id
    }

    #[tokio::test]
    async fn test_get_claim_roundtrip_and_missing() {
        let service = sqlite_service();
        let id = assert_one(&service, "Alice").await;

        // Existing claim is found.
        let found = service
            .get_claim(Request::new(GetClaimRequest {
                claim_id: id.clone(),
                auth_token: "token".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(found.found);
        assert_eq!(found.claim.unwrap().subject, "Alice");

        // A random (valid) id that was never asserted is not found.
        let missing = service
            .get_claim(Request::new(GetClaimRequest {
                claim_id: ClaimId::new().to_string(),
                auth_token: "token".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(!missing.found);
        assert!(missing.claim.is_none());
    }

    #[tokio::test]
    async fn test_get_claim_requires_auth() {
        let service = sqlite_service();
        let err = service
            .get_claim(Request::new(GetClaimRequest {
                claim_id: ClaimId::new().to_string(),
                auth_token: String::new(),
            }))
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
    }

    #[tokio::test]
    async fn test_forget_deletes_claim() {
        let service = sqlite_service();
        let id = assert_one(&service, "Alice").await;

        let forget = service
            .forget(Request::new(ForgetRequest {
                claim_id: id.clone(),
                reason: String::new(),
                auth_token: "token".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(forget.success);

        // After deletion the claim is gone.
        let found = service
            .get_claim(Request::new(GetClaimRequest {
                claim_id: id.clone(),
                auth_token: "token".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(!found.found);

        // Forgetting again reports not-found rather than success.
        let again = service
            .forget(Request::new(ForgetRequest {
                claim_id: id,
                reason: String::new(),
                auth_token: "token".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(!again.success);
    }

    #[tokio::test]
    async fn test_get_relationships_after_add() {
        let store = SqliteStore::new(":memory:", false, 0).unwrap();
        let store = Arc::new(Mutex::new(store));
        let service = BosWellServiceImpl::new(Arc::clone(&store));

        let a = assert_one(&service, "Alice").await;
        let b = assert_one(&service, "Bob").await;
        let a_id = ClaimId::from_string(&a).unwrap();
        let b_id = ClaimId::from_string(&b).unwrap();

        {
            let mut guard = store.lock().unwrap();
            guard
                .add_relationship(Relationship {
                    from_claim: a_id,
                    to_claim: b_id,
                    relationship_type: boswell_domain::RelationshipType::Supports,
                    strength: 0.9,
                    created_at: 1000,
                })
                .unwrap();
        }

        let rels = service
            .get_relationships(Request::new(GetRelationshipsRequest {
                claim_id: a,
                auth_token: "token".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(rels.relationships.len(), 1);
        assert_eq!(
            rels.relationships[0].relationship_type,
            RelationshipType::Supports as i32
        );
    }

    #[tokio::test]
    async fn test_query_by_source_type() {
        let store = SqliteStore::new(":memory:", false, 0).unwrap();
        let store = Arc::new(Mutex::new(store));
        let service = BosWellServiceImpl::new(Arc::clone(&store));

        // One assertion (default source_type) and one extraction.
        assert_one(&service, "Alice").await;
        {
            let mut guard = store.lock().unwrap();
            guard
                .assert_claim(
                    Claim::new(
                        ClaimId::new(),
                        "test".into(),
                        "Carol".into(),
                        "knows".into(),
                        "Dave".into(),
                        (0.7, 0.8),
                        "task".into(),
                        1000,
                    )
                    .with_source_type("extraction"),
                )
                .unwrap();
        }

        let extraction = service
            .query(Request::new(QueryRequest {
                filter: Some(QueryFilter {
                    namespace: Some("test".to_string()),
                    source_type: Some("extraction".to_string()),
                    ..Default::default()
                }),
                mode: QueryMode::Fast as i32,
                limit: 100,
                auth_token: "token".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(extraction.claims.len(), 1);
        assert_eq!(extraction.claims[0].subject, "Carol");
    }

    #[tokio::test]
    async fn test_extract_disabled_without_extractor() {
        let service = sqlite_service();
        let err = service
            .extract(Request::new(ExtractRequest {
                text: "Alice works at Acme".to_string(),
                namespace: "test".to_string(),
                tier: "task".to_string(),
                source_id: String::new(),
                auth_token: "token".to_string(),
            }))
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
    }
}
