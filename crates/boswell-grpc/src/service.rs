///! gRPC service implementation
///!
///! Implements the BosWellService trait generated from proto definitions.

use std::sync::{Arc, Mutex};
use tonic::{Request, Response, Status};
use boswell_domain::{Claim, ClaimId};
use boswell_domain::traits::{ClaimStore, ClaimQuery};

use crate::proto::bos_well_service_server::BosWellService;
use crate::proto::*;
use crate::conversions::{
    claim_from_proto, claim_to_proto, confidence_from_proto,
    tier_from_proto,
};

/// Implementation of the BosWellService
pub struct BosWellServiceImpl<S: ClaimStore> {
    store: Arc<Mutex<S>>,
    start_time: std::time::Instant,
}

impl<S: ClaimStore> BosWellServiceImpl<S> {
    /// Create a new service instance
    pub fn new(store: Arc<Mutex<S>>) -> Self {
        Self {
            store,
            start_time: std::time::Instant::now(),
        }
    }
}

#[tonic::async_trait]
impl<S> BosWellService for BosWellServiceImpl<S>
where
    S: ClaimStore + Send + Sync + 'static,
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
            tier_from_proto(Tier::try_from(req.tier)
                .map_err(|_| Status::invalid_argument("Invalid tier"))?)
                .map_err(|e| Status::invalid_argument(e.to_string()))?
        } else {
            "ephemeral".to_string()  // Default tier
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
            confidence: (confidence.lower, confidence.upper),
            tier,
            created_at,
            stale_at: None,
        };
        
        // Assert claim to store
        let mut store = self.store.lock().unwrap();
        let result = store.assert_claim(claim.clone())
            .map_err(|e| Status::internal(format!("Failed to assert claim: {:?}", e)))?;
        
        Ok(Response::new(AssertResponse {
            claim_id: result.to_string(),
            is_duplicate: result == claim.id,  // Simplified duplicate detection
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
        
        let filter = req.filter.ok_or_else(|| Status::invalid_argument("Missing filter"))?;
        
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
            min_confidence: filter.min_confidence.filter(|&c| c > 0.0),
            semantic_text: None,
            limit: if req.limit > 0 { Some(req.limit as usize) } else { Some(100) },
        };
        
        // Query claims from store
        let store = self.store.lock().unwrap();
        let claims = store.query_claims(&query)
            .map_err(|e| Status::internal(format!("Query failed: {:?}", e)))?;
        
        // Apply additional filters (subject, predicate, object not in ClaimQuery yet)
        let filtered_claims: Vec<Claim> = claims.into_iter()
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
        let proto_claims = filtered_claims.into_iter()
            .map(claim_to_proto)
            .collect();
        
        Ok(Response::new(QueryResponse {
            claims: proto_claims,
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
        let mut duplicate_count = 0;
        let mut error_count = 0;
        let mut errors = Vec::new();
        
        let mut store = self.store.lock().unwrap();
        
        for proto_claim in req.claims {
            match claim_from_proto(proto_claim) {
                Ok(claim) => {
                    match store.assert_claim(claim.clone()) {
                        Ok(_) => inserted_count += 1,
                        Err(_) => {
                            error_count += 1;
                            errors.push(format!("Failed to insert claim {}", claim.id));
                        }
                    }
                }
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
        
        // Check if claim exists
        let store = self.store.lock().unwrap();
        match store.get_claim(claim_id) {
            Ok(Some(_)) => {
                // TODO: Implement actual eviction marking in Phase 3
                Ok(Response::new(ForgetResponse {
                    success: true,
                    message: format!("Claim {} marked for eviction (stub)", req.claim_id),
                }))
            }
            Ok(None) => {
                Ok(Response::new(ForgetResponse {
                    success: false,
                    message: "Claim not found".to_string(),
                }))
            }
            Err(e) => {
                Ok(Response::new(ForgetResponse {
                    success: false,
                    message: format!("Error checking claim: {:?}", e),
                }))
            }
        }
    }

    async fn health_check(
        &self,
        _request: Request<HealthCheckRequest>,
    ) -> Result<Response<HealthCheckResponse>, Status> {
        let store = self.store.lock().unwrap();
        let query = ClaimQuery::default();
        let claim_count = store.query_claims(&query)
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
                confidence: (0.8, 0.95),
                tier: "task".to_string(),
                created_at: 1000000,
                stale_at: None,
            }))
        }
        
        fn query_claims(&self, _query: &ClaimQuery) -> Result<Vec<Claim>, Self::Error> {
            Ok(vec![
                Claim {
                    id: ClaimId::new(),
                    namespace: "test".to_string(),
                    subject: "Alice".to_string(),
                    predicate: "knows".to_string(),
                    object: "Bob".to_string(),
                    confidence: (0.8, 0.95),
                    tier: "task".to_string(),
                    created_at: 1000000,
                    stale_at: None,
                }
            ])
        }
        
        fn add_relationship(&mut self, _relationship: Relationship) -> Result<(), Self::Error> {
            Ok(())
        }
        
        fn get_relationships(&self, _id: ClaimId) -> Result<Vec<Relationship>, Self::Error> {
            Ok(vec![])
        }
    }
    
    #[tokio::test]
    async fn test_health_check() {
        let service = BosWellServiceImpl::new(Arc::new(Mutex::new(MockStore)));
        let request = Request::new(HealthCheckRequest {});
        
        let response = service.health_check(request).await.unwrap();
        let health = response.into_inner();
        
        assert_eq!(health.status, health_check_response::Status::Healthy as i32);
        assert!(health.claim_count >= 0);
    }
}
