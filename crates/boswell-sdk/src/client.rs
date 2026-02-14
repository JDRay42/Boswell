//! Boswell client implementation.

use crate::error::SdkError;
use crate::session::establish_session;
use boswell_domain::{Claim, ClaimId, Tier};
use boswell_grpc::proto::{
    bos_well_service_client::BosWellServiceClient, AssertRequest, AssertResponse, ConfidenceInterval,
    ForgetRequest, ForgetResponse, LearnRequest, LearnResponse, QueryFilter as GrpcQueryFilter,
    QueryMode as GrpcQueryMode, QueryRequest, QueryResponse, Tier as GrpcTier,
};
use tonic::transport::Channel;

/// Query filter for claim queries
#[derive(Debug, Default, Clone)]
pub struct QueryFilter {
    /// Namespace filter
    pub namespace: Option<String>,
    /// Subject filter
    pub subject: Option<String>,
    /// Predicate filter
    pub predicate: Option<String>,
    /// Object filter
    pub object: Option<String>,
    /// Minimum confidence threshold
    pub min_confidence: Option<f64>,
    /// Tier filter
    pub tier: Option<Tier>,
}

/// Boswell SDK client
pub struct BoswellClient {
    router_endpoint: String,
    session_token: Option<String>,
    instance_endpoint: Option<String>,
    grpc_client: Option<BosWellServiceClient<Channel>>,
}

impl BoswellClient {
    /// Create a new Boswell client
    pub fn new(router_endpoint: &str) -> Self {
        Self {
            router_endpoint: router_endpoint.to_string(),
            session_token: None,
            instance_endpoint: None,
            grpc_client: None,
        }
    }

    /// Establish session with Router and connect to gRPC instance
    pub fn connect(&mut self) -> Result<(), SdkError> {
        // Establish session with Router
        let session_response = establish_session(&self.router_endpoint)?;

        self.session_token = Some(session_response.token);

        // Pick the first healthy instance
        let instance = session_response
            .instances
            .iter()
            .find(|i| i.health == "healthy")
            .or_else(|| session_response.instances.first())
            .ok_or(SdkError::NoInstancesAvailable)?;

        self.instance_endpoint = Some(instance.endpoint.clone());

        // Connect to gRPC instance
        self.connect_grpc(&instance.endpoint)?;

        Ok(())
    }

    /// Connect to gRPC instance
    fn connect_grpc(&mut self, endpoint: &str) -> Result<(), SdkError> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| SdkError::ConnectionError(format!("Failed to create runtime: {}", e)))?;

        let channel = runtime
            .block_on(async { Channel::from_shared(endpoint.to_string()) })
            .map_err(|e| SdkError::ConnectionError(format!("Invalid endpoint: {}", e)))?
            .connect_lazy();

        self.grpc_client = Some(BosWellServiceClient::new(channel));

        Ok(())
    }

    /// Assert a claim
    pub fn assert(
        &mut self,
        namespace: &str,
        subject: &str,
        predicate: &str,
        object: &str,
        confidence: Option<f64>,
        tier: Option<Tier>,
    ) -> Result<ClaimId, SdkError> {
        let client = self.grpc_client.as_mut().ok_or(SdkError::NotConnected)?;
        let token = self.session_token.as_ref().ok_or(SdkError::NotConnected)?;

        let confidence_interval = confidence.map(|c| ConfidenceInterval {
            lower: c,
            upper: c,
        });

        let tier_i32 = tier
            .map(|t| grpc_tier_from_domain_tier(t))
            .unwrap_or(GrpcTier::Unspecified as i32);

        let request = AssertRequest {
            namespace: namespace.to_string(),
            subject: subject.to_string(),
            predicate: predicate.to_string(),
            object: object.to_string(),
            confidence: confidence_interval,
            tier: tier_i32,
            provenance: vec![],
            auth_token: token.clone(),
        };

        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| SdkError::ConnectionError(format!("Failed to create runtime: {}", e)))?;

        let response: AssertResponse = runtime
            .block_on(async { client.assert(request).await })
            .map(|r| r.into_inner())
            .map_err(|e| {
                // Check if it's an auth error
                if matches!(e.code(), tonic::Code::Unauthenticated) {
                    SdkError::AuthError("Session expired or invalid".to_string())
                } else {
                    SdkError::from(e)
                }
            })?;

        ClaimId::from_string(&response.claim_id)
            .map_err(|e| SdkError::GrpcError(format!("Invalid claim ID: {}", e)))
    }

    /// Query claims
    pub fn query(&mut self, filter: QueryFilter) -> Result<Vec<Claim>, SdkError> {
        let client = self.grpc_client.as_mut().ok_or(SdkError::NotConnected)?;
        let token = self.session_token.as_ref().ok_or(SdkError::NotConnected)?;

        let grpc_filter = GrpcQueryFilter {
            namespace: filter.namespace,
            subject: filter.subject,
            predicate: filter.predicate,
            object: filter.object,
            min_confidence: filter.min_confidence,
            tier: filter.tier.map(grpc_tier_from_domain_tier),
        };

        let request = QueryRequest {
            filter: Some(grpc_filter),
            mode: GrpcQueryMode::Fast as i32,
            limit: 100,
            auth_token: token.clone(),
        };

        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| SdkError::ConnectionError(format!("Failed to create runtime: {}", e)))?;

        let response: QueryResponse = runtime
            .block_on(async { client.query(request).await })
            .map(|r| r.into_inner())
            .map_err(SdkError::from)?;

        // Convert gRPC claims to domain claims
        let claims: Result<Vec<Claim>, _> = response
            .claims
            .into_iter()
            .map(|c| grpc_claim_to_domain(&c))
            .collect();

        claims.map_err(|e| SdkError::GrpcError(format!("Failed to convert claim: {}", e)))
    }

    /// Learn multiple claims in batch
    pub fn learn(&mut self, claims: Vec<Claim>) -> Result<LearnResponse, SdkError> {
        let client = self.grpc_client.as_mut().ok_or(SdkError::NotConnected)?;
        let token = self.session_token.as_ref().ok_or(SdkError::NotConnected)?;

        let grpc_claims: Vec<_> = claims.into_iter().map(domain_claim_to_grpc).collect();

        let request = LearnRequest {
            claims: grpc_claims,
            skip_duplicates: false,
            auth_token: token.clone(),
        };

        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| SdkError::ConnectionError(format!("Failed to create runtime: {}", e)))?;

        let response: LearnResponse = runtime
            .block_on(async { client.learn(request).await })
            .map(|r| r.into_inner())
            .map_err(SdkError::from)?;

        Ok(response)
    }

    /// Forget (evict) claims
    pub fn forget(&mut self, claim_ids: Vec<ClaimId>) -> Result<bool, SdkError> {
        let client = self.grpc_client.as_mut().ok_or(SdkError::NotConnected)?;
        let token = self.session_token.as_ref().ok_or(SdkError::NotConnected)?;

        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| SdkError::ConnectionError(format!("Failed to create runtime: {}", e)))?;

        // Execute forget operations sequentially
        for claim_id in claim_ids {
            let request = ForgetRequest {
                claim_id: claim_id.to_string(),
                reason: String::new(),
                auth_token: token.clone(),
            };

            let response: ForgetResponse = runtime
                .block_on(async { client.forget(request).await })
                .map(|r| r.into_inner())
                .map_err(SdkError::from)?;

            if !response.success {
                return Ok(false);
            }
        }

        Ok(true)
    }
}

// Helper functions for type conversion

fn grpc_tier_from_domain_tier(tier: Tier) -> i32 {
    match tier {
        Tier::Ephemeral => GrpcTier::Ephemeral as i32,
        Tier::Task => GrpcTier::Task as i32,
        Tier::Project => GrpcTier::Project as i32,
        Tier::Permanent => GrpcTier::Permanent as i32,
    }
}

fn domain_tier_from_grpc(tier: i32) -> Result<String, String> {
    match GrpcTier::try_from(tier) {
        Ok(GrpcTier::Ephemeral) => Ok("ephemeral".to_string()),
        Ok(GrpcTier::Task) => Ok("task".to_string()),
        Ok(GrpcTier::Project) => Ok("project".to_string()),
        Ok(GrpcTier::Permanent) => Ok("permanent".to_string()),
        _ => Err("Invalid tier".to_string()),
    }
}

fn grpc_claim_to_domain(claim: &boswell_grpc::proto::Claim) -> Result<Claim, String> {
    let claim_id =
        ClaimId::from_string(&claim.id).map_err(|e| format!("Invalid claim ID: {}", e))?;

    let confidence = claim
        .confidence
        .as_ref()
        .map(|c| (c.lower, c.upper))
        .ok_or("Missing confidence interval")?;

    // Validate confidence bounds
    if confidence.0 < 0.0
        || confidence.0 > 1.0
        || confidence.1 < 0.0
        || confidence.1 > 1.0
        || confidence.0 > confidence.1
    {
        return Err("Invalid confidence bounds".to_string());
    }

    let tier = domain_tier_from_grpc(claim.tier)?;

    let created_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    Ok(Claim {
        id: claim_id,
        namespace: claim.namespace.clone(),
        subject: claim.subject.clone(),
        predicate: claim.predicate.clone(),
        object: claim.object.clone(),
        confidence,
        tier,
        created_at,
        stale_at: None,
    })
}

fn domain_claim_to_grpc(claim: Claim) -> boswell_grpc::proto::Claim {
    // Convert tier string to proto Tier  
    let tier = Tier::parse(&claim.tier)
        .map(grpc_tier_from_domain_tier)
        .unwrap_or(GrpcTier::Unspecified as i32);

    boswell_grpc::proto::Claim {
        id: claim.id.to_string(),
        namespace: claim.namespace,
        subject: claim.subject,
        predicate: claim.predicate,
        object: claim.object,
        confidence: Some(ConfidenceInterval {
            lower: claim.confidence.0,
            upper: claim.confidence.1,
        }),
        tier,
    }
}


