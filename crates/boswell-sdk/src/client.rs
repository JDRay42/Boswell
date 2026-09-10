//! Boswell client implementation.

use crate::error::SdkError;
use crate::retry::{Idempotency, RetryAction, RetryPolicy, RetryState};
use crate::session::establish_session;
use boswell_domain::{
    Claim, ClaimId, ExecutionReceipt, ExpandResult, Goal, Procedure, Relationship, Tier,
};
use boswell_grpc::conversions::{
    expanded_candidate_from_proto, factor_reading_from_proto, goal_from_proto,
    procedure_from_proto, receipt_from_proto, relationship_from_proto,
};
use boswell_grpc::proto::{
    bos_well_service_client::BosWellServiceClient, health_check_response, AssertRequest,
    AssertResponse, ConfidenceInterval, ExpandRequest, ExpandResponse, ExtractRequest,
    ExtractResponse, ForgetRequest, ForgetResponse, GetClaimRequest, GetClaimResponse,
    GetGoalRequest, GetGoalResponse, GetProcedureRequest, GetProcedureResponse,
    GetRelationshipsRequest, GetRelationshipsResponse, HealthCheckRequest, HealthCheckResponse,
    IssuedProcedure as GrpcIssuedProcedure, LearnRequest, LearnResponse,
    QueryFilter as GrpcQueryFilter, QueryGoalsRequest, QueryGoalsResponse,
    QueryMode as GrpcQueryMode, QueryProceduresRequest, QueryProceduresResponse, QueryRequest,
    QueryResponse, ReportOutcomeRequest, ReportOutcomeResponse, SearchRequest, SearchResponse,
    Tier as GrpcTier,
};
use tonic::transport::Channel;

/// What to retrieve in a [`query_procedures`](BoswellClient::query_procedures)
/// call (design §4.1).
///
/// `issued_to` is required: it names the principal that takes on the reporting
/// obligation for every procedure the call issues.
#[derive(Debug, Clone, Default)]
pub struct ProcedureQuerySpec {
    /// The principal the execution receipts are issued to.
    pub issued_to: String,
    /// Filter by namespace prefix.
    pub namespace: Option<String>,
    /// Filter by exact `goal` grouping key.
    pub goal: Option<String>,
    /// Filter by a case-insensitive substring of `intent`.
    pub intent_contains: Option<String>,
    /// Include superseded (non-current) versions.
    pub include_superseded: bool,
    /// Maximum results to return.
    pub limit: Option<u32>,
    /// Correlation: task id, stamped onto the issued receipts.
    pub task_id: Option<String>,
    /// Correlation: session id, stamped onto the issued receipts.
    pub session_id: Option<String>,
}

impl ProcedureQuerySpec {
    /// A query issuing receipts to `issued_to`.
    pub fn new(issued_to: impl Into<String>) -> Self {
        Self {
            issued_to: issued_to.into(),
            ..Default::default()
        }
    }

    /// Filter to a single `goal` grouping key.
    pub fn with_goal(mut self, goal: impl Into<String>) -> Self {
        self.goal = Some(goal.into());
        self
    }

    /// Filter to a namespace prefix.
    pub fn with_namespace(mut self, namespace: impl Into<String>) -> Self {
        self.namespace = Some(namespace.into());
        self
    }
}

/// What to retrieve in a [`query_goals`](BoswellClient::query_goals) call.
///
/// Unlike [`ProcedureQuerySpec`] this names no principal: traversal issues no
/// receipt, so there is no obligation to attach to anyone.
#[derive(Debug, Clone, Default)]
pub struct GoalQuerySpec {
    /// Filter by namespace prefix.
    pub namespace: Option<String>,
    /// Filter by a case-insensitive substring of `intent`.
    pub intent_contains: Option<String>,
    /// Maximum number of results.
    pub limit: Option<u32>,
}

impl GoalQuerySpec {
    /// A query for goals whose intent contains `text`.
    pub fn matching(text: impl Into<String>) -> Self {
        Self {
            intent_contains: Some(text.into()),
            ..Self::default()
        }
    }

    /// Confine the query to a namespace prefix.
    pub fn with_namespace(mut self, namespace: impl Into<String>) -> Self {
        self.namespace = Some(namespace.into());
        self
    }
}

/// Rebuild a domain [`ExpandResult`] from an expand response.
fn expand_result_from_proto(r: &ExpandResponse) -> Result<ExpandResult, SdkError> {
    let convert = |cs: &[boswell_grpc::proto::ExpandedCandidate]| {
        cs.iter()
            .map(|c| {
                expanded_candidate_from_proto(c)
                    .map_err(|e| SdkError::GrpcError(format!("Failed to convert candidate: {}", e)))
            })
            .collect::<Result<Vec<_>, _>>()
    };

    Ok(ExpandResult {
        candidates: convert(&r.candidates)?,
        decision_aids: convert(&r.decision_aids)?,
        factor_readings: r
            .factor_readings
            .iter()
            .map(|f| {
                factor_reading_from_proto(f).map_err(|e| {
                    SdkError::GrpcError(format!("Failed to convert factor reading: {}", e))
                })
            })
            .collect::<Result<Vec<_>, _>>()?,
    })
}

/// A procedure issued together with the execution receipt for it.
///
/// Holding one is holding an obligation: answer `receipt.receipt_id` with
/// [`report_outcome`](BoswellClient::report_outcome) before
/// `receipt.expires_at`, or the run counts as `unknown` against the procedure.
#[derive(Debug, Clone)]
pub struct IssuedProcedure {
    /// The procedure to execute.
    pub procedure: Procedure,
    /// The reporting receipt issued with the procedure.
    pub receipt: ExecutionReceipt,
}

fn issued_from_proto(d: &GrpcIssuedProcedure) -> Result<IssuedProcedure, SdkError> {
    let procedure = d
        .procedure
        .as_ref()
        .ok_or_else(|| SdkError::GrpcError("issued procedure missing its body".to_string()))?;
    let receipt = d.receipt.as_ref().ok_or_else(|| {
        SdkError::GrpcError("issued procedure missing its execution receipt".to_string())
    })?;

    Ok(IssuedProcedure {
        procedure: procedure_from_proto(procedure)
            .map_err(|e| SdkError::GrpcError(format!("Failed to convert procedure: {}", e)))?,
        receipt: receipt_from_proto(receipt)
            .map_err(|e| SdkError::GrpcError(format!("Failed to convert receipt: {}", e)))?,
    })
}

/// An outcome report against an outstanding receipt (design §3.3).
///
/// `outcome` is `success`, `failure`, or `abandoned`; `failure_mode` is only
/// valid alongside `failure` and is one of `preconditions_stale`, `step_failed`,
/// `bad_result`, or `executor_error`. `failed_step` names the step when the mode
/// is `step_failed`.
#[derive(Debug, Clone, Default)]
pub struct OutcomeReportSpec {
    /// The receipt this report answers.
    pub receipt_id: String,
    /// `success` | `failure` | `abandoned`.
    pub outcome: String,
    /// Failure attribution, when the outcome is `failure`.
    pub failure_mode: Option<String>,
    /// The step that failed, when `failure_mode` is `step_failed`.
    pub failed_step: Option<String>,
    /// The executor's self-assessed confidence.
    pub executor_confidence: Option<f64>,
    /// The reported cost (units are executor-defined).
    pub cost: Option<f64>,
    /// Free-form notes.
    pub notes: Option<String>,
}

impl OutcomeReportSpec {
    /// A minimal success report against `receipt_id`.
    pub fn success(receipt_id: impl Into<String>) -> Self {
        Self {
            receipt_id: receipt_id.into(),
            outcome: "success".to_string(),
            ..Default::default()
        }
    }

    /// A failure report against `receipt_id` with the given attribution.
    pub fn failure(receipt_id: impl Into<String>, failure_mode: impl Into<String>) -> Self {
        Self {
            receipt_id: receipt_id.into(),
            outcome: "failure".to_string(),
            failure_mode: Some(failure_mode.into()),
            ..Default::default()
        }
    }
}

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
    /// Whether the instance is running under a development identity adapter
    /// (design §7.2). When true, nothing this instance serves may be trusted
    /// for long-term memory, and downstream layers must say so.
    pub dev_auth: bool,
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
    /// Whether the connected instance runs a development identity adapter.
    /// Learned from the health check; devAuth is a startup decision, so this is
    /// accurate for the life of the connection.
    dev_auth: bool,
    /// How failed RPCs are retried. See [`RetryPolicy`].
    retry: RetryPolicy,
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
            dev_auth: false,
            retry: RetryPolicy::default(),
        }
    }

    /// Replace the retry policy (default: [`RetryPolicy::default`]).
    ///
    /// The policy governs the backoff path only. Re-establishing an expired
    /// session is not optional and happens under every policy, including
    /// [`RetryPolicy::none`].
    pub fn with_retry_policy(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    /// The retry policy currently in force.
    pub fn retry_policy(&self) -> RetryPolicy {
        self.retry
    }

    /// Whether the connected instance runs a development identity adapter
    /// (design §7.2), as reported by the last health check.
    ///
    /// A caller that surfaces Boswell's answers onward — a gateway, a UI — must
    /// mark them when this is true: they came from fake, preset identities and
    /// must not be trusted for long-term memory.
    pub fn is_dev_auth(&self) -> bool {
        self.dev_auth
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

    /// Act on a failed RPC: re-establish the session, sleep out a backoff, or
    /// give up by handing `status` back to the caller.
    ///
    /// `Ok(())` means the caller should send the request again. `idempotency`
    /// says whether that is safe to do for a transport failure, where the
    /// handler may already have run — see [`Idempotency`].
    async fn handle_retry(
        &mut self,
        status: tonic::Status,
        state: &mut RetryState,
        idempotency: Idempotency,
    ) -> Result<(), SdkError> {
        match state.on_error(&status, idempotency) {
            RetryAction::Reconnect => self.reconnect().await,
            RetryAction::Backoff(delay) => {
                tokio::time::sleep(delay).await;
                Ok(())
            }
            RetryAction::Fail => Err(SdkError::from(status)),
        }
    }

    /// Assert a claim.
    ///
    /// `confidence` is an interval `(lower, upper)`, matching the domain claim
    /// model and ADR-003. Callers that genuinely have only a point estimate pass
    /// `(c, c)`; callers that have a range must not pre-collapse it, since the
    /// width of the interval is itself meaningful — a narrow interval asserts
    /// that the system has a clear picture.
    pub async fn assert(
        &mut self,
        namespace: &str,
        subject: &str,
        predicate: &str,
        object: &str,
        confidence: Option<(f64, f64)>,
        tier: Option<Tier>,
    ) -> Result<ClaimId, SdkError> {
        let mut retry = RetryState::new(self.retry);

        loop {
            let client = self.grpc_client.as_mut().ok_or(SdkError::NotConnected)?;
            let token = self.session_token.as_ref().ok_or(SdkError::NotConnected)?;

            let confidence_interval =
                confidence.map(|(lower, upper)| ConfidenceInterval { lower, upper });

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
                Err(e) => {
                    self.handle_retry(e, &mut retry, Idempotency::Unsafe)
                        .await?
                }
            }
        }
    }

    /// Query claims
    pub async fn query(&mut self, filter: QueryFilter) -> Result<Vec<Claim>, SdkError> {
        let mut retry = RetryState::new(self.retry);

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
                Err(e) => self.handle_retry(e, &mut retry, Idempotency::Safe).await?,
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
        let mut retry = RetryState::new(self.retry);

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
                Err(e) => self.handle_retry(e, &mut retry, Idempotency::Safe).await?,
            }
        }
    }

    /// Learn multiple claims in batch
    pub async fn learn(&mut self, claims: Vec<Claim>) -> Result<LearnResponse, SdkError> {
        let mut retry = RetryState::new(self.retry);

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
                Err(e) => {
                    self.handle_retry(e, &mut retry, Idempotency::Unsafe)
                        .await?
                }
            }
        }
    }

    /// Forget (evict) claims
    pub async fn forget(&mut self, claim_ids: Vec<ClaimId>) -> Result<bool, SdkError> {
        let mut retry = RetryState::new(self.retry);

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
                    Err(e) => {
                        self.handle_retry(e, &mut retry, Idempotency::Unsafe)
                            .await?;
                        continue 'retry;
                    }
                }
            }

            return Ok(true);
        }
    }

    /// Fetch a single claim by id. Returns `None` if no such claim exists.
    pub async fn get_claim(&mut self, claim_id: ClaimId) -> Result<Option<Claim>, SdkError> {
        let mut retry = RetryState::new(self.retry);

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
                Err(e) => self.handle_retry(e, &mut retry, Idempotency::Safe).await?,
            }
        }
    }

    /// Fetch the relationships (provenance / contradiction graph) for a claim.
    pub async fn get_relationships(
        &mut self,
        claim_id: ClaimId,
    ) -> Result<Vec<Relationship>, SdkError> {
        let mut retry = RetryState::new(self.retry);

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
                Err(e) => self.handle_retry(e, &mut retry, Idempotency::Safe).await?,
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
        let mut retry = RetryState::new(self.retry);

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
                Err(e) => {
                    self.handle_retry(e, &mut retry, Idempotency::Unsafe)
                        .await?
                }
            }
        }
    }

    // ---- Procedural memory (design 15 §3.3, §4.1) ----

    /// Retrieve procedures for a goal/intent, each with the execution receipt
    /// issued for it.
    ///
    /// Retrieval is not free: every returned procedure carries a receipt, and
    /// the caller is obliged to answer it with [`report_outcome`] before the
    /// receipt expires. An unreported receipt counts as `unknown` against the
    /// procedure ("silence is not success", design §3.3).
    ///
    /// [`report_outcome`]: BoswellClient::report_outcome
    pub async fn query_procedures(
        &mut self,
        query: ProcedureQuerySpec,
    ) -> Result<Vec<IssuedProcedure>, SdkError> {
        let mut retry = RetryState::new(self.retry);

        loop {
            let client = self.grpc_client.as_mut().ok_or(SdkError::NotConnected)?;
            let token = self.session_token.as_ref().ok_or(SdkError::NotConnected)?;

            let request = QueryProceduresRequest {
                namespace: query.namespace.clone(),
                goal: query.goal.clone(),
                intent_contains: query.intent_contains.clone(),
                include_superseded: query.include_superseded,
                limit: query.limit,
                issued_to: query.issued_to.clone(),
                task_id: query.task_id.clone(),
                session_id: query.session_id.clone(),
                auth_token: token.clone(),
            };

            match client.query_procedures(request).await {
                Ok(r) => {
                    let response: QueryProceduresResponse = r.into_inner();
                    return response.procedures.iter().map(issued_from_proto).collect();
                }
                Err(e) => {
                    self.handle_retry(e, &mut retry, Idempotency::Unsafe)
                        .await?
                }
            }
        }
    }

    /// Fetch one procedure by id, issuing an execution receipt for it.
    ///
    /// Returns `None` if no such procedure exists, or if it lies outside
    /// `namespace_scope` — the scope is enforced server-side *before* a receipt
    /// is issued, so an out-of-scope lookup leaves no obligation behind. As with
    /// [`query_procedures`](BoswellClient::query_procedures), a returned
    /// procedure carries a reporting obligation.
    pub async fn get_procedure(
        &mut self,
        id: &str,
        issued_to: &str,
        namespace_scope: Option<String>,
        task_id: Option<String>,
        session_id: Option<String>,
    ) -> Result<Option<IssuedProcedure>, SdkError> {
        let mut retry = RetryState::new(self.retry);

        loop {
            let client = self.grpc_client.as_mut().ok_or(SdkError::NotConnected)?;
            let token = self.session_token.as_ref().ok_or(SdkError::NotConnected)?;

            let request = GetProcedureRequest {
                id: id.to_string(),
                issued_to: issued_to.to_string(),
                namespace_scope: namespace_scope.clone(),
                task_id: task_id.clone(),
                session_id: session_id.clone(),
                auth_token: token.clone(),
            };

            match client.get_procedure(request).await {
                Ok(r) => {
                    let response: GetProcedureResponse = r.into_inner();
                    return match response.procedure.filter(|_| response.found) {
                        Some(d) => issued_from_proto(&d).map(Some),
                        None => Ok(None),
                    };
                }
                Err(e) => {
                    self.handle_retry(e, &mut retry, Idempotency::Unsafe)
                        .await?
                }
            }
        }
    }

    /// Report an execution outcome against an outstanding receipt (design §3.3).
    ///
    /// This is the call the capture hooks make. The report is a
    /// provenance-stamped, gatekept write: a negative report from a
    /// low-assurance executor against a team-tier procedure is recorded but
    /// quarantined rather than applied, so one executor cannot tank a shared
    /// how-to.
    pub async fn report_outcome(
        &mut self,
        report: OutcomeReportSpec,
    ) -> Result<ReportOutcomeResponse, SdkError> {
        let mut retry = RetryState::new(self.retry);

        loop {
            let client = self.grpc_client.as_mut().ok_or(SdkError::NotConnected)?;
            let token = self.session_token.as_ref().ok_or(SdkError::NotConnected)?;

            let request = ReportOutcomeRequest {
                receipt_id: report.receipt_id.clone(),
                outcome: report.outcome.clone(),
                failure_mode: report.failure_mode.clone(),
                failed_step: report.failed_step.clone(),
                executor_confidence: report.executor_confidence,
                cost: report.cost,
                notes: report.notes.clone(),
                auth_token: token.clone(),
            };

            match client.report_outcome(request).await {
                Ok(r) => return Ok(r.into_inner()),
                Err(e) => {
                    self.handle_retry(e, &mut retry, Idempotency::Unsafe)
                        .await?
                }
            }
        }
    }

    // ---- Goal traversal (design 15 §3.2, §4.1) ----

    /// Retrieve goals by namespace/intent — the entry hop into a decomposition.
    ///
    /// Unlike procedure retrieval, traversal is free: no execution receipt is
    /// issued and no reporting obligation is created. Only fetching a leaf
    /// procedure for execution costs the caller an obligation.
    pub async fn query_goals(&mut self, query: GoalQuerySpec) -> Result<Vec<Goal>, SdkError> {
        let mut retry = RetryState::new(self.retry);

        loop {
            let client = self.grpc_client.as_mut().ok_or(SdkError::NotConnected)?;
            let token = self.session_token.as_ref().ok_or(SdkError::NotConnected)?;

            let request = QueryGoalsRequest {
                namespace: query.namespace.clone(),
                intent_contains: query.intent_contains.clone(),
                limit: query.limit,
                auth_token: token.clone(),
            };

            match client.query_goals(request).await {
                Ok(r) => {
                    let response: QueryGoalsResponse = r.into_inner();
                    return response
                        .goals
                        .iter()
                        .map(|g| {
                            goal_from_proto(g).map_err(|e| {
                                SdkError::GrpcError(format!("Failed to convert goal: {}", e))
                            })
                        })
                        .collect();
                }
                Err(e) => self.handle_retry(e, &mut retry, Idempotency::Safe).await?,
            }
        }
    }

    /// Fetch one goal by id.
    ///
    /// Returns `None` if no such goal exists, or if it lies outside
    /// `namespace_scope`.
    pub async fn get_goal(
        &mut self,
        id: &str,
        namespace_scope: Option<String>,
    ) -> Result<Option<Goal>, SdkError> {
        let mut retry = RetryState::new(self.retry);

        loop {
            let client = self.grpc_client.as_mut().ok_or(SdkError::NotConnected)?;
            let token = self.session_token.as_ref().ok_or(SdkError::NotConnected)?;

            let request = GetGoalRequest {
                id: id.to_string(),
                namespace_scope: namespace_scope.clone(),
                auth_token: token.clone(),
            };

            match client.get_goal(request).await {
                Ok(r) => {
                    let response: GetGoalResponse = r.into_inner();
                    return match response.goal.filter(|_| response.found) {
                        Some(g) => goal_from_proto(&g).map(Some).map_err(|e| {
                            SdkError::GrpcError(format!("Failed to convert goal: {}", e))
                        }),
                        None => Ok(None),
                    };
                }
                Err(e) => self.handle_retry(e, &mut retry, Idempotency::Safe).await?,
            }
        }
    }

    /// Expand one goal into its ranked candidate surface — a single traversal
    /// hop (design §4.1).
    ///
    /// Traversal is stateless and agent-driven: the caller holds the cursor and
    /// calls this again on whichever child it chooses. The store surfaces
    /// precondition-filtered, deterministically ranked candidates along with the
    /// factor readings behind them; the *weighting* is the caller's, not the
    /// store's.
    ///
    /// Returns `None` when no such goal exists or it lies outside
    /// `namespace_scope` — distinct from `Some(result)` with no candidates,
    /// which is a real goal whose children were all filtered out.
    pub async fn expand(
        &mut self,
        goal_id: &str,
        context_tags: Vec<String>,
        namespace_scope: Option<String>,
    ) -> Result<Option<ExpandResult>, SdkError> {
        let mut retry = RetryState::new(self.retry);

        loop {
            let client = self.grpc_client.as_mut().ok_or(SdkError::NotConnected)?;
            let token = self.session_token.as_ref().ok_or(SdkError::NotConnected)?;

            let request = ExpandRequest {
                goal_id: goal_id.to_string(),
                context: Some(boswell_grpc::proto::TraversalContext {
                    context_tags: context_tags.clone(),
                }),
                namespace_scope: namespace_scope.clone(),
                auth_token: token.clone(),
            };

            match client.expand(request).await {
                Ok(r) => {
                    let response: ExpandResponse = r.into_inner();
                    if !response.found {
                        return Ok(None);
                    }
                    return expand_result_from_proto(&response).map(Some);
                }
                Err(e) => self.handle_retry(e, &mut retry, Idempotency::Safe).await?,
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

        // Remembered so callers that do not poll health can still tell: devAuth
        // is a startup decision (the adapter fails closed at construction), so
        // learning it at connect time is accurate for the life of the channel.
        self.dev_auth = response.dev_auth;

        Ok(HealthStatus {
            status,
            version: response.version,
            uptime_seconds: response.uptime_seconds,
            claim_count: response.claim_count,
            message: response.message,
            dev_auth: response.dev_auth,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// A client wired to a gRPC endpoint that nothing is listening on.
    ///
    /// `connect_lazy` does not dial, so the failure lands on the first RPC as a
    /// transport error rather than at construction. That is exactly the shape of
    /// the transient failure the backoff path exists for, without needing a
    /// server to kill.
    fn client_pointed_at_a_closed_port(retry: RetryPolicy) -> BoswellClient {
        let mut client = BoswellClient::new("http://127.0.0.1:1").with_retry_policy(retry);
        client.session_token = Some("test-token".to_string());
        client.instance_endpoint = Some("http://127.0.0.1:1".to_string());
        let channel = Channel::from_static("http://127.0.0.1:1").connect_lazy();
        client.grpc_client = Some(BosWellServiceClient::new(channel));
        client
    }

    #[tokio::test]
    async fn a_read_backs_off_between_attempts() {
        let policy = RetryPolicy::default()
            .without_jitter()
            .with_max_retries(2)
            .with_initial_backoff(Duration::from_millis(50));
        let mut client = client_pointed_at_a_closed_port(policy);

        let started = Instant::now();
        let result = client.query(QueryFilter::default()).await;
        let elapsed = started.elapsed();

        assert!(result.is_err(), "a closed port cannot answer a query");
        // Two retries at 50ms then 100ms. Anything faster means the backoff
        // never ran and the loop gave up on the first failure.
        assert!(
            elapsed >= Duration::from_millis(150),
            "query returned after {elapsed:?}, too fast to have backed off twice"
        );
    }

    #[tokio::test]
    async fn a_write_fails_without_backing_off() {
        let policy = RetryPolicy::default()
            .without_jitter()
            .with_max_retries(5)
            .with_initial_backoff(Duration::from_millis(200));
        let mut client = client_pointed_at_a_closed_port(policy);

        let started = Instant::now();
        let result = client
            .assert("test", "s", "p", "o", Some((0.5, 0.5)), None)
            .await;
        let elapsed = started.elapsed();

        assert!(result.is_err(), "a closed port cannot accept a claim");
        // Retrying an assert would duplicate the claim, so the first transport
        // failure is final. One backoff would already have cost 200ms.
        assert!(
            elapsed < Duration::from_millis(200),
            "assert took {elapsed:?}; it backed off when it should not have"
        );
    }

    #[tokio::test]
    async fn a_zero_retry_policy_gives_up_immediately() {
        let mut client = client_pointed_at_a_closed_port(RetryPolicy::none());

        let started = Instant::now();
        assert!(client.query(QueryFilter::default()).await.is_err());

        assert!(started.elapsed() < Duration::from_millis(100));
    }

    #[test]
    fn the_default_policy_is_the_one_a_new_client_gets() {
        let client = BoswellClient::new("http://localhost:8080");

        assert_eq!(client.retry_policy(), RetryPolicy::default());
    }
}
