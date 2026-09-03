//! Boswell client implementation.

use crate::error::SdkError;
use crate::session::establish_session;
use boswell_domain::{Claim, ClaimId, Relationship, Tier};
use boswell_grpc::conversions::relationship_from_proto;
use boswell_grpc::proto::{
    bos_well_service_client::BosWellServiceClient, health_check_response, AssertRequest,
    AssertResponse, ConfidenceInterval, ExtractRequest, ExtractResponse, ForgetRequest,
    ForgetResponse, GetClaimRequest, GetClaimResponse, GetRelationshipsRequest,
    GetRelationshipsResponse, HealthCheckRequest, HealthCheckResponse, LearnRequest, LearnResponse,
    QueryFilter as GrpcQueryFilter, QueryMode as GrpcQueryMode, QueryRequest, QueryResponse,
    SearchRequest, SearchResponse, Tier as GrpcTier,
};
use tonic::transport::Channel;

/// Instance health as reported by the `HealthCheck` RPC.
#[derive(Debug, Clone)]
pub struct HealthStatus {
    /// Health level: `healthy`, `degraded`, `unhealthy`, or `unspecified`.
    pub status: String,
    /// Instance software version.
    pub version: String,
    /// Seconds the instance has been running.
    pub uptime_seconds: i64,
    /// Number of claims currently stored.
    pub claim_count: i64,
    /// Optional human-readable message.
    pub message: String,
}

/// Result of a server-side extraction (`Extract` RPC), as seen by the SDK.
#[derive(Debug, Clone)]
pub struct ExtractResult {
    /// Claims newly created by the extraction (already persisted server-side).
    pub claims_created: Vec<Claim>,
    /// Number of claims created.
    pub created_count: usize,
    /// Number of extracted claims that corroborated existing claims.
    pub corroborated_count: usize,
    /// Number of candidates that failed validation or storage.
    pub failed_count: usize,
    /// Human-readable failure reasons.
    pub failures: Vec<String>,
}

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
    /// Source-type filter (e.g. `assertion`, `extraction`, `inference`, `import`)
    pub source_type: Option<String>,
}

/// Boswell SDK client
pub struct BoswellClient {
    router_endpoint: String,
    session_token: Option<String>,
    instance_endpoint: Option<String>,
    grpc_client: Option<BosWellServiceClient<Channel>>,
    http_client: reqwest::Client,
}

impl BoswellClient {
    /// Create a new Boswell client
    pub fn new(router_endpoint: &str) -> Self {
        Self {
            router_endpoint: router_endpoint.to_string(),
            session_token: None,
            instance_endpoint: None,
            grpc_client: None,
            http_client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .pool_max_idle_per_host(10)
                .build()
                .expect("Failed to build HTTP client"),
        }
    }

    /// Establish session with Router and connect to gRPC instance
    pub async fn connect(&mut self) -> Result<(), SdkError> {
        // Establish session with Router
        let session_response = establish_session(&self.http_client, &self.router_endpoint).await?;

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
        self.connect_grpc(&instance.endpoint).await?;

        Ok(())
    }

    /// Connect to gRPC instance
    async fn connect_grpc(&mut self, endpoint: &str) -> Result<(), SdkError> {
        let channel = Channel::from_shared(endpoint.to_string())
            .map_err(|e| SdkError::ConnectionError(format!("Invalid endpoint: {}", e)))?
            .connect_lazy();

        self.grpc_client = Some(BosWellServiceClient::new(channel));

        Ok(())
    }

    /// Reconnect after auth failure
    async fn reconnect(&mut self) -> Result<(), SdkError> {
        self.connect().await
    }

    /// Assert a claim
    pub async fn assert(
        &mut self,
        namespace: &str,
        subject: &str,
        predicate: &str,
        object: &str,
        confidence: Option<f64>,
        tier: Option<Tier>,
    ) -> Result<ClaimId, SdkError> {
        let mut retried = false;

        loop {
            let client = self.grpc_client.as_mut().ok_or(SdkError::NotConnected)?;
            let token = self.session_token.as_ref().ok_or(SdkError::NotConnected)?;

            let confidence_interval = confidence.map(|c| ConfidenceInterval { lower: c, upper: c });

            let tier_i32 = tier
                .map(grpc_tier_from_domain_tier)
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

            match client.assert(request).await {
                Ok(r) => {
                    let assert_response: AssertResponse = r.into_inner();
                    return ClaimId::from_string(&assert_response.claim_id)
                        .map_err(|e| SdkError::GrpcError(format!("Invalid claim ID: {}", e)));
                }
                Err(e) if matches!(e.code(), tonic::Code::Unauthenticated) && !retried => {
                    // Session expired - try to reconnect once
                    self.reconnect().await?;
                    retried = true;
                }
                Err(e) => return Err(SdkError::from(e)),
            }
        }
    }

    /// Query claims
    pub async fn query(&mut self, filter: QueryFilter) -> Result<Vec<Claim>, SdkError> {
        let mut retried = false;

        loop {
            let client = self.grpc_client.as_mut().ok_or(SdkError::NotConnected)?;
            let token = self.session_token.as_ref().ok_or(SdkError::NotConnected)?;

            let grpc_filter = GrpcQueryFilter {
                namespace: filter.namespace.clone(),
                subject: filter.subject.clone(),
                predicate: filter.predicate.clone(),
                object: filter.object.clone(),
                min_confidence: filter.min_confidence,
                tier: filter.tier.map(grpc_tier_from_domain_tier),
                source_type: filter.source_type.clone(),
            };

            let request = QueryRequest {
                filter: Some(grpc_filter),
                mode: GrpcQueryMode::Fast as i32,
                limit: 100,
                auth_token: token.clone(),
            };

            match client.query(request).await {
                Ok(r) => {
                    let query_response: QueryResponse = r.into_inner();

                    // Convert gRPC claims to domain claims
                    let claims: Result<Vec<Claim>, _> = query_response
                        .claims
                        .into_iter()
                        .map(|c| grpc_claim_to_domain(&c))
                        .collect();

                    return claims.map_err(|e| {
                        SdkError::GrpcError(format!("Failed to convert claim: {}", e))
                    });
                }
                Err(e) if matches!(e.code(), tonic::Code::Unauthenticated) && !retried => {
                    // Session expired - try to reconnect once
                    self.reconnect().await?;
                    retried = true;
                }
                Err(e) => return Err(SdkError::from(e)),
            }
        }
    }

    /// Semantically search for claims similar to `query_text`.
    ///
    /// Returns up to `limit` `(claim, similarity)` pairs whose cosine similarity
    /// is at least `min_similarity`, ordered by similarity descending. When
    /// `namespace` is provided, results are restricted to that namespace prefix.
    pub async fn search(
        &mut self,
        query_text: &str,
        namespace: Option<String>,
        limit: usize,
        min_similarity: f64,
    ) -> Result<Vec<(Claim, f32)>, SdkError> {
        let mut retried = false;

        loop {
            let client = self.grpc_client.as_mut().ok_or(SdkError::NotConnected)?;
            let token = self.session_token.as_ref().ok_or(SdkError::NotConnected)?;

            let request = SearchRequest {
                query_text: query_text.to_string(),
                namespace: namespace.clone(),
                limit: limit as i32,
                min_similarity,
                auth_token: token.clone(),
            };

            match client.search(request).await {
                Ok(r) => {
                    let response: SearchResponse = r.into_inner();
                    let mut results = Vec::with_capacity(response.results.len());
                    for item in response.results {
                        let proto_claim = item.claim.ok_or_else(|| {
                            SdkError::GrpcError("Search result missing claim".to_string())
                        })?;
                        let claim = grpc_claim_to_domain(&proto_claim).map_err(|e| {
                            SdkError::GrpcError(format!("Failed to convert claim: {}", e))
                        })?;
                        results.push((claim, item.similarity as f32));
                    }
                    return Ok(results);
                }
                Err(e) if matches!(e.code(), tonic::Code::Unauthenticated) && !retried => {
                    self.reconnect().await?;
                    retried = true;
                }
                Err(e) => return Err(SdkError::from(e)),
            }
        }
    }

    /// Learn multiple claims in batch
    pub async fn learn(&mut self, claims: Vec<Claim>) -> Result<LearnResponse, SdkError> {
        let mut retried = false;

        loop {
            let client = self.grpc_client.as_mut().ok_or(SdkError::NotConnected)?;
            let token = self.session_token.as_ref().ok_or(SdkError::NotConnected)?;

            let grpc_claims: Vec<_> = claims
                .iter()
                .map(|c| domain_claim_to_grpc(c.clone()))
                .collect();

            let request = LearnRequest {
                claims: grpc_claims,
                skip_duplicates: false,
                auth_token: token.clone(),
            };

            match client.learn(request).await {
                Ok(r) => return Ok(r.into_inner()),
                Err(e) if matches!(e.code(), tonic::Code::Unauthenticated) && !retried => {
                    // Session expired - try to reconnect once
                    self.reconnect().await?;
                    retried = true;
                }
                Err(e) => return Err(SdkError::from(e)),
            }
        }
    }

    /// Forget (evict) claims
    pub async fn forget(&mut self, claim_ids: Vec<ClaimId>) -> Result<bool, SdkError> {
        let mut retried = false;

        'retry: loop {
            let client = self.grpc_client.as_mut().ok_or(SdkError::NotConnected)?;
            let token = self.session_token.as_ref().ok_or(SdkError::NotConnected)?;

            // Execute forget operations sequentially
            for claim_id in &claim_ids {
                let request = ForgetRequest {
                    claim_id: claim_id.to_string(),
                    reason: String::new(),
                    auth_token: token.clone(),
                };

                match client.forget(request).await {
                    Ok(r) => {
                        let forget_response: ForgetResponse = r.into_inner();
                        if !forget_response.success {
                            return Ok(false);
                        }
                    }
                    Err(e) if matches!(e.code(), tonic::Code::Unauthenticated) && !retried => {
                        // Session expired - try to reconnect once
                        self.reconnect().await?;
                        retried = true;
                        continue 'retry;
                    }
                    Err(e) => return Err(SdkError::from(e)),
                }
            }

            return Ok(true);
        }
    }

    /// Fetch a single claim by id. Returns `None` if no such claim exists.
    pub async fn get_claim(&mut self, claim_id: ClaimId) -> Result<Option<Claim>, SdkError> {
        let mut retried = false;

        loop {
            let client = self.grpc_client.as_mut().ok_or(SdkError::NotConnected)?;
            let token = self.session_token.as_ref().ok_or(SdkError::NotConnected)?;

            let request = GetClaimRequest {
                claim_id: claim_id.to_string(),
                auth_token: token.clone(),
            };

            match client.get_claim(request).await {
                Ok(r) => {
                    let response: GetClaimResponse = r.into_inner();
                    if !response.found {
                        return Ok(None);
                    }
                    let proto_claim = response.claim.ok_or_else(|| {
                        SdkError::GrpcError("get_claim response missing claim".to_string())
                    })?;
                    let claim = grpc_claim_to_domain(&proto_claim).map_err(|e| {
                        SdkError::GrpcError(format!("Failed to convert claim: {}", e))
                    })?;
                    return Ok(Some(claim));
                }
                Err(e) if matches!(e.code(), tonic::Code::Unauthenticated) && !retried => {
                    self.reconnect().await?;
                    retried = true;
                }
                Err(e) => return Err(SdkError::from(e)),
            }
        }
    }

    /// Fetch the relationships (provenance / contradiction graph) for a claim.
    pub async fn get_relationships(
        &mut self,
        claim_id: ClaimId,
    ) -> Result<Vec<Relationship>, SdkError> {
        let mut retried = false;

        loop {
            let client = self.grpc_client.as_mut().ok_or(SdkError::NotConnected)?;
            let token = self.session_token.as_ref().ok_or(SdkError::NotConnected)?;

            let request = GetRelationshipsRequest {
                claim_id: claim_id.to_string(),
                auth_token: token.clone(),
            };

            match client.get_relationships(request).await {
                Ok(r) => {
                    let response: GetRelationshipsResponse = r.into_inner();
                    let relationships: Result<Vec<Relationship>, _> = response
                        .relationships
                        .into_iter()
                        .map(relationship_from_proto)
                        .collect();
                    return relationships.map_err(|e| {
                        SdkError::GrpcError(format!("Failed to convert relationship: {}", e))
                    });
                }
                Err(e) if matches!(e.code(), tonic::Code::Unauthenticated) && !retried => {
                    self.reconnect().await?;
                    retried = true;
                }
                Err(e) => return Err(SdkError::from(e)),
            }
        }
    }

    /// Extract claims from unstructured text via the server-side LLM Extractor.
    ///
    /// `tier` and `source_id` may be empty, in which case the server applies its
    /// defaults (`task` tier, a synthetic source id).
    pub async fn extract(
        &mut self,
        text: &str,
        namespace: &str,
        tier: &str,
        source_id: &str,
    ) -> Result<ExtractResult, SdkError> {
        let mut retried = false;

        loop {
            let client = self.grpc_client.as_mut().ok_or(SdkError::NotConnected)?;
            let token = self.session_token.as_ref().ok_or(SdkError::NotConnected)?;

            let request = ExtractRequest {
                text: text.to_string(),
                namespace: namespace.to_string(),
                tier: tier.to_string(),
                source_id: source_id.to_string(),
                auth_token: token.clone(),
            };

            match client.extract(request).await {
                Ok(r) => {
                    let response: ExtractResponse = r.into_inner();
                    let claims_created: Result<Vec<Claim>, _> = response
                        .claims_created
                        .iter()
                        .map(grpc_claim_to_domain)
                        .collect();
                    let claims_created = claims_created.map_err(|e| {
                        SdkError::GrpcError(format!("Failed to convert claim: {}", e))
                    })?;
                    return Ok(ExtractResult {
                        claims_created,
                        created_count: response.created_count.max(0) as usize,
                        corroborated_count: response.corroborated_count.max(0) as usize,
                        failed_count: response.failed_count.max(0) as usize,
                        failures: response.failures,
                    });
                }
                Err(e) if matches!(e.code(), tonic::Code::Unauthenticated) && !retried => {
                    self.reconnect().await?;
                    retried = true;
                }
                Err(e) => return Err(SdkError::from(e)),
            }
        }
    }

    /// Whether the client has an established session and gRPC channel.
    pub fn is_connected(&self) -> bool {
        self.grpc_client.is_some() && self.session_token.is_some()
    }

    /// Connect if not already connected. Idempotent; safe to call before any
    /// operation to recover from a cold start where the instance was initially
    /// unreachable.
    pub async fn ensure_connected(&mut self) -> Result<(), SdkError> {
        if !self.is_connected() {
            self.connect().await?;
        }
        Ok(())
    }

    /// Query instance health via the `HealthCheck` RPC.
    ///
    /// Connects on demand; the RPC itself is unauthenticated.
    pub async fn health(&mut self) -> Result<HealthStatus, SdkError> {
        self.ensure_connected().await?;
        let client = self.grpc_client.as_mut().ok_or(SdkError::NotConnected)?;

        let response: HealthCheckResponse = client
            .health_check(HealthCheckRequest {})
            .await?
            .into_inner();

        let status = match health_check_response::Status::try_from(response.status) {
            Ok(health_check_response::Status::Healthy) => "healthy",
            Ok(health_check_response::Status::Degraded) => "degraded",
            Ok(health_check_response::Status::Unhealthy) => "unhealthy",
            _ => "unspecified",
        }
        .to_string();

        Ok(HealthStatus {
            status,
            version: response.version,
            uptime_seconds: response.uptime_seconds,
            claim_count: response.claim_count,
            message: response.message,
        })
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

    let source_type = if claim.source_type.is_empty() {
        Claim::SOURCE_ASSERTION.to_string()
    } else {
        claim.source_type.clone()
    };

    Ok(Claim {
        id: claim_id,
        namespace: claim.namespace.clone(),
        subject: claim.subject.clone(),
        predicate: claim.predicate.clone(),
        object: claim.object.clone(),
        source_type,
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
        source_type: claim.source_type,
    }
}
