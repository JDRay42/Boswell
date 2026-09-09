//! gRPC service implementation
//!
//! Implements the BosWellService trait generated from proto definitions.

use boswell_domain::traits::{ClaimQuery, ClaimStore, ProcedureStore};
use boswell_domain::{
    Assurance, Authority, Claim, ClaimId, DelegationChain, EvidenceType, ExecutionReceipt, Op,
    ProcedureQuery, ProvenanceStamp, Tier as DomainTier,
};
use std::sync::{Arc, Mutex};
use tonic::{Request, Response, Status};

use crate::conversions::{
    claim_from_proto, claim_to_proto, confidence_from_proto, outcome_report_from_proto,
    procedure_id_from_proto, procedure_to_proto, receipt_to_proto, relationship_to_proto,
    tier_from_proto,
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
    receipt_ttl_ms: u64,
}

/// How long an issued procedure's execution receipt stays open before it
/// expires unreported (and counts as `unknown` — "silence is not success",
/// design §3.3). One hour by default; override with
/// [`BosWellServiceImpl::with_receipt_ttl_ms`].
pub const DEFAULT_RECEIPT_TTL_MS: u64 = 60 * 60 * 1000;

impl<S: ClaimStore> BosWellServiceImpl<S> {
    /// Create a new service instance without a server-side extractor. The
    /// `Extract` RPC returns `FailedPrecondition` until one is attached with
    /// [`BosWellServiceImpl::with_extractor`].
    pub fn new(store: Arc<Mutex<S>>) -> Self {
        Self {
            store,
            start_time: std::time::Instant::now(),
            extractor: None,
            receipt_ttl_ms: DEFAULT_RECEIPT_TTL_MS,
        }
    }

    /// Set how long issued execution receipts stay open (Unix ms duration).
    pub fn with_receipt_ttl_ms(mut self, ttl_ms: u64) -> Self {
        self.receipt_ttl_ms = ttl_ms;
        self
    }

    /// Issue a procedure: persist an execution receipt for it (design §3.3).
    ///
    /// Every issued procedure gets one — retrieval is what creates the
    /// obligation to report — so the receipt is written before the procedure
    /// leaves the process.
    fn issue<P>(
        &self,
        store: &mut P,
        procedure: &boswell_domain::Procedure,
        issued_to: &str,
        task_id: Option<String>,
        session_id: Option<String>,
        now: u64,
    ) -> Result<crate::proto::ExecutionReceipt, Status>
    where
        P: ProcedureStore,
        P::Error: std::fmt::Debug,
    {
        let mut receipt = ExecutionReceipt::issue(procedure, issued_to, now, self.receipt_ttl_ms);
        receipt.task_id = task_id;
        receipt.session_id = session_id;

        store
            .issue_receipt(&receipt)
            .map_err(|e| Status::internal(format!("Failed to issue receipt: {:?}", e)))?;

        Ok(receipt_to_proto(&receipt))
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
    S: ClaimStore + ProcedureStore + Send + 'static,
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

    // ---- Procedural memory (design 15 §3.3, §4.1) ----

    async fn query_procedures(
        &self,
        request: Request<QueryProceduresRequest>,
    ) -> Result<Response<QueryProceduresResponse>, Status> {
        let req = request.into_inner();
        if req.auth_token.is_empty() {
            return Err(Status::unauthenticated("Missing authentication token"));
        }
        let issued_to = require_issued_to(&req.issued_to)?;

        let query = ProcedureQuery {
            namespace: req.namespace,
            goal: req.goal,
            intent_contains: req.intent_contains,
            include_superseded: req.include_superseded,
            limit: req.limit.map(|l| l as usize),
        };

        let now = now_ms();
        let mut store = self.store.lock().unwrap();
        require_procedures(&*store)?;

        let procedures = store
            .query_procedures(&query, now)
            .map_err(|e| Status::internal(format!("Failed to query procedures: {:?}", e)))?;

        let mut issued = Vec::with_capacity(procedures.len());
        for procedure in &procedures {
            let receipt = self.issue(
                &mut *store,
                procedure,
                issued_to,
                req.task_id.clone(),
                req.session_id.clone(),
                now,
            )?;
            issued.push(IssuedProcedure {
                procedure: Some(procedure_to_proto(procedure)),
                receipt: Some(receipt),
            });
        }

        let count = issued.len() as i32;
        Ok(Response::new(QueryProceduresResponse {
            procedures: issued,
            count,
            message: format!("{} procedure(s) issued", count),
        }))
    }

    async fn get_procedure(
        &self,
        request: Request<GetProcedureRequest>,
    ) -> Result<Response<GetProcedureResponse>, Status> {
        let req = request.into_inner();
        if req.auth_token.is_empty() {
            return Err(Status::unauthenticated("Missing authentication token"));
        }
        let issued_to = require_issued_to(&req.issued_to)?;
        let id = procedure_id_from_proto(&req.id)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let now = now_ms();
        let mut store = self.store.lock().unwrap();
        require_procedures(&*store)?;

        let found = store
            .get_procedure(id)
            .map_err(|e| Status::internal(format!("Failed to get procedure: {:?}", e)))?;

        // Scope is checked before the receipt is issued, not after: issuing one for a
        // procedure the caller may not see would leave an obligation nobody can
        // answer, and the resulting expiry would count as `unknown` against
        // another namespace's procedure.
        let procedure = match found {
            Some(p) if namespace_in_scope(req.namespace_scope.as_deref(), &p.namespace) => p,
            _ => {
                return Ok(Response::new(GetProcedureResponse {
                    found: false,
                    procedure: None,
                    message: format!("No procedure with id {}", req.id),
                }));
            }
        };

        let receipt = self.issue(
            &mut *store,
            &procedure,
            issued_to,
            req.task_id.clone(),
            req.session_id.clone(),
            now,
        )?;

        Ok(Response::new(GetProcedureResponse {
            found: true,
            procedure: Some(IssuedProcedure {
                procedure: Some(procedure_to_proto(&procedure)),
                receipt: Some(receipt),
            }),
            message: "Procedure issued".to_string(),
        }))
    }

    async fn report_outcome(
        &self,
        request: Request<ReportOutcomeRequest>,
    ) -> Result<Response<ReportOutcomeResponse>, Status> {
        let req = request.into_inner();
        if req.auth_token.is_empty() {
            return Err(Status::unauthenticated("Missing authentication token"));
        }
        let receipt_id = procedure_id_from_proto(&req.receipt_id)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let report = outcome_report_from_proto(
            receipt_id,
            &req.outcome,
            req.failure_mode.as_deref(),
            req.failed_step.as_deref(),
            req.executor_confidence,
            req.cost,
            req.notes,
        )
        .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let now = now_ms();
        let mut store = self.store.lock().unwrap();
        require_procedures(&*store)?;

        // The stamp's author is the principal the receipt was issued to, not
        // whoever is calling: a report is only ever a self-report against an
        // outstanding receipt.
        let Some(stored) = store
            .get_receipt(receipt_id)
            .map_err(|e| Status::internal(format!("Failed to load receipt: {:?}", e)))?
        else {
            return Ok(Response::new(not_found_report(&req.receipt_id)));
        };

        // Scope the reporter's authority to the procedure's own namespace where
        // the procedure still exists; an orphaned receipt gets an empty scope
        // and the report will no-op in the store.
        let namespace = store
            .get_procedure(stored.receipt.procedure_id)
            .map_err(|e| Status::internal(format!("Failed to load procedure: {:?}", e)))?
            .map(|p| p.namespace)
            .unwrap_or_default();

        let stamp = self_report_stamp(&stored.receipt, namespace, now);

        let outcome = store
            .report_receipt(receipt_id, &report, &stamp, now)
            .map_err(|e| Status::internal(format!("Failed to report outcome: {:?}", e)))?;

        Ok(Response::new(match outcome {
            None => not_found_report(&req.receipt_id),
            Some(o) => report_to_proto(o),
        }))
    }
}

// ---- Procedural-memory helpers ----

/// Current wall-clock time in Unix milliseconds (the unit procedural memory uses).
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Reject a request that names no principal: an execution receipt with
/// nobody on the hook for reporting is not a contract (design §3.3).
fn require_issued_to(issued_to: &str) -> Result<&str, Status> {
    if issued_to.trim().is_empty() {
        return Err(Status::invalid_argument(
            "issued_to is required: retrieving a procedure obliges a principal to report",
        ));
    }
    Ok(issued_to)
}

/// Answer `Unimplemented` rather than an empty result when the backing store
/// holds no procedures, so a claim-only deployment is distinguishable from a
/// genuine no-match.
fn require_procedures<S: ProcedureStore>(store: &S) -> Result<(), Status> {
    if !store.supports_procedures() {
        return Err(Status::unimplemented(
            "this instance's store does not hold procedures",
        ));
    }
    Ok(())
}

/// Whether `namespace` falls within a caller's `scope`.
///
/// An absent, empty, or `"*"` scope is unrestricted; otherwise the namespace
/// must equal the scope or be a child of it (`"<scope>:..."`), matching
/// [`Authority::allows_namespace`].
fn namespace_in_scope(scope: Option<&str>, namespace: &str) -> bool {
    match scope {
        None => true,
        Some(s) if s.is_empty() || s == "*" => true,
        Some(s) => namespace == s || namespace.starts_with(&format!("{}:", s)),
    }
}

/// The provenance stamp for an executor's self-report (design §3.3).
///
/// Assurance is [`Assurance::None`] because this transport has no
/// `IdentityProvider` wired: the identity is self-claimed. That is deliberate —
/// it is what makes the gatekeeper quarantine a negative self-report against a
/// team-tier procedure instead of letting one executor tank a shared how-to.
fn self_report_stamp(receipt: &ExecutionReceipt, namespace: String, now: u64) -> ProvenanceStamp {
    ProvenanceStamp {
        author: receipt.issued_to.clone(),
        delegation_chain: DelegationChain(vec![receipt.issued_to.clone()]),
        authority: Authority {
            namespaces: if namespace.is_empty() {
                Vec::new()
            } else {
                vec![namespace]
            },
            max_tier: DomainTier::Ephemeral,
            ops: vec![Op::Read, Op::Write],
        },
        // The executor watched its own run, so the evidence is first-hand; the
        // assurance above is what bounds how far it can move a shared procedure.
        evidence: EvidenceType::Observed,
        assurance: Assurance::None,
        task_id: receipt.task_id.clone(),
        session_id: receipt.session_id.clone(),
        timestamp: now,
        dev_provider: false,
    }
}

/// The response for a report against a receipt this instance has never issued.
fn not_found_report(receipt_id: &str) -> ReportOutcomeResponse {
    ReportOutcomeResponse {
        accepted: false,
        already_final: false,
        applied: false,
        quarantined: false,
        counted_as_success: false,
        counted_as_failure: false,
        attributed_to_executor: false,
        flagged_precondition_stale: false,
        message: format!("No outstanding receipt with id {}", receipt_id),
    }
}

/// Flatten a store-side report outcome onto the wire response.
fn report_to_proto(outcome: boswell_domain::ReceiptReportOutcome) -> ReportOutcomeResponse {
    if outcome.already_final {
        return ReportOutcomeResponse {
            accepted: false,
            already_final: true,
            applied: false,
            quarantined: false,
            counted_as_success: false,
            counted_as_failure: false,
            attributed_to_executor: false,
            flagged_precondition_stale: false,
            message: "Receipt was already reported or expired".to_string(),
        };
    }

    let quarantined = outcome.applied.map(|a| a.quarantined).unwrap_or(false);
    let effect = outcome.applied.and_then(|a| a.effect);
    let message = match (&effect, quarantined) {
        (_, true) => "Report recorded but quarantined: reporter assurance too low for this \
             procedure's tier"
            .to_string(),
        (Some(_), _) => "Report applied".to_string(),
        (None, _) => "Receipt closed, but its procedure no longer exists".to_string(),
    };

    ReportOutcomeResponse {
        accepted: true,
        already_final: false,
        applied: effect.is_some(),
        quarantined,
        counted_as_success: effect.map(|e| e.counted_as_success).unwrap_or(false),
        counted_as_failure: effect.map(|e| e.counted_as_failure).unwrap_or(false),
        attributed_to_executor: effect.map(|e| e.attributed_to_executor).unwrap_or(false),
        flagged_precondition_stale: effect
            .map(|e| e.flagged_precondition_stale)
            .unwrap_or(false),
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use boswell_domain::Relationship;

    // Mock store for testing
    struct MockStore;

    // Claim-only mock: the procedural defaults ("this store holds no
    // procedures") are exactly right, so the impl is empty.
    impl ProcedureStore for MockStore {}

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

    // Claim-only mock: the procedural defaults ("this store holds no
    // procedures") are exactly right, so the impl is empty.
    impl ProcedureStore for SemanticMockStore {}

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

    // ---- Procedural memory (design 15 §3.3, §4.1) ----

    mod procedural {
        use super::*;
        use boswell_domain::{
            BodyFormat, ClaimMatch, Expect, Precondition, PreconditionCheck, Procedure,
            ProcedureId, ProcedureSource, Tier as DomainTierEnum,
        };
        use boswell_store::SqliteStore;

        const NOW: u64 = 1_700_000_000_000;

        fn service_with(procedures: Vec<Procedure>) -> BosWellServiceImpl<SqliteStore> {
            let mut store = SqliteStore::new(":memory:", false, 0).unwrap();

            // The fixture procedure is gated on "jd has eggs"; the store filters
            // issuing on preconditions, so the backing claim has to exist for
            // the procedure to be retrievable at all.
            store
                .assert_claim(Claim::new(
                    ClaimId::new(),
                    "person:jd".into(),
                    "jd".into(),
                    "has".into(),
                    "eggs".into(),
                    (0.7, 0.8),
                    "project".into(),
                    NOW,
                ))
                .unwrap();

            for p in &procedures {
                store.upsert_procedure(p).unwrap();
            }
            BosWellServiceImpl::new(Arc::new(Mutex::new(store)))
        }

        fn mk(name: &str, tier: DomainTierEnum) -> Procedure {
            Procedure {
                id: ProcedureId::new(),
                namespace: "person:jd".into(),
                name: name.into(),
                version: 1,
                supersedes: None,
                is_current: true,
                source: ProcedureSource::Authored,
                goal: "goal:person:jd/cook-eggs".into(),
                intent: format!("intent for {}", name),
                tags: vec!["breakfast".into()],
                parameters: vec![boswell_domain::Parameter {
                    name: "eggs".into(),
                    type_name: "int".into(),
                    default: Some("2".into()),
                    desc: Some("how many".into()),
                }],
                preconditions: vec![Precondition {
                    kind: "resource".into(),
                    description: "eggs on hand".into(),
                    check: PreconditionCheck {
                        match_pattern: ClaimMatch {
                            subject: "jd".into(),
                            predicate: "has".into(),
                            object: "eggs".into(),
                        },
                        min_confidence: 0.6,
                        expect: Expect::Exists,
                    },
                }],
                required_tools: vec!["pan".into()],
                postconditions: vec!["eggs are cooked".into()],
                est_duration_sec: Some(300),
                usage_notes: "keep the heat low".into(),
                context_tags: vec!["kitchen".into()],
                body_format: BodyFormat::Prose,
                content_type: "text/plain".into(),
                body: format!("body of {}", name),
                tier,
                use_count: 0,
                success_count: 0,
                failure_count: 0,
                unknown_count: 0,
                last_used_at: None,
                created_at: NOW,
                updated_at: NOW,
                stale_at: None,
            }
        }

        fn query_req(issued_to: &str) -> QueryProceduresRequest {
            QueryProceduresRequest {
                namespace: None,
                goal: Some("goal:person:jd/cook-eggs".into()),
                intent_contains: None,
                include_superseded: false,
                limit: None,
                issued_to: issued_to.into(),
                task_id: Some("task-1".into()),
                session_id: Some("session-1".into()),
                auth_token: "token".into(),
            }
        }

        fn report_req(receipt_id: &str, outcome: &str) -> ReportOutcomeRequest {
            ReportOutcomeRequest {
                receipt_id: receipt_id.into(),
                outcome: outcome.into(),
                failure_mode: None,
                failed_step: None,
                executor_confidence: None,
                cost: None,
                notes: None,
                auth_token: "token".into(),
            }
        }

        /// Retrieval returns the procedure *and* a receipt naming the
        /// principal on the hook — the obligation is created when the store
        /// issues the procedure (design §3.3), not when the executor opts in.
        #[tokio::test]
        async fn issuing_a_procedure_creates_a_receipt() {
            let service = service_with(vec![mk("omelette", DomainTierEnum::Task)]);

            let resp = service
                .query_procedures(Request::new(query_req("agent:cook-1")))
                .await
                .unwrap()
                .into_inner();

            assert_eq!(resp.count, 1);
            let issued = &resp.procedures[0];
            let receipt = issued.receipt.as_ref().unwrap();
            assert_eq!(receipt.issued_to, "agent:cook-1");
            assert_eq!(receipt.task_id.as_deref(), Some("task-1"));
            assert_eq!(receipt.session_id.as_deref(), Some("session-1"));
            assert!(receipt.expires_at > receipt.issued_at);
            assert_eq!(receipt.procedure_id, issued.procedure.as_ref().unwrap().id);
        }

        /// A procedure issued to nobody is not a contract, so it is rejected
        /// rather than silently issuing an unanswerable receipt.
        #[tokio::test]
        async fn issuing_requires_a_principal() {
            let service = service_with(vec![mk("omelette", DomainTierEnum::Task)]);

            let err = service
                .query_procedures(Request::new(query_req("  ")))
                .await
                .unwrap_err();

            assert_eq!(err.code(), tonic::Code::InvalidArgument);
        }

        /// The whole point of Phase 7a: a hook can close the loop. A success
        /// report moves the procedure's counters.
        #[tokio::test]
        async fn reporting_success_records_effectiveness() {
            let procedure = mk("omelette", DomainTierEnum::Task);
            let procedure_id = procedure.id;
            let service = service_with(vec![procedure]);

            let issued = service
                .query_procedures(Request::new(query_req("agent:cook-1")))
                .await
                .unwrap()
                .into_inner();
            let receipt_id = issued.procedures[0]
                .receipt
                .as_ref()
                .unwrap()
                .receipt_id
                .clone();

            let resp = service
                .report_outcome(Request::new(report_req(&receipt_id, "success")))
                .await
                .unwrap()
                .into_inner();

            assert!(resp.accepted, "{}", resp.message);
            assert!(resp.applied);
            assert!(resp.counted_as_success);
            assert!(!resp.quarantined);

            let store = service.store.lock().unwrap();
            let after = ProcedureStore::get_procedure(&*store, procedure_id)
                .unwrap()
                .unwrap();
            assert_eq!(after.success_count, 1);
            assert_eq!(after.use_count, 1);
        }

        /// "Silence is not success" has a mirror: a receipt may only be answered
        /// once, so a chatty executor cannot stack repeat credit.
        #[tokio::test]
        async fn a_receipt_can_only_be_answered_once() {
            let service = service_with(vec![mk("omelette", DomainTierEnum::Task)]);

            let issued = service
                .query_procedures(Request::new(query_req("agent:cook-1")))
                .await
                .unwrap()
                .into_inner();
            let receipt_id = issued.procedures[0]
                .receipt
                .as_ref()
                .unwrap()
                .receipt_id
                .clone();

            service
                .report_outcome(Request::new(report_req(&receipt_id, "success")))
                .await
                .unwrap();

            let second = service
                .report_outcome(Request::new(report_req(&receipt_id, "success")))
                .await
                .unwrap()
                .into_inner();

            assert!(second.already_final);
            assert!(!second.applied);
        }

        /// A negative self-report against a team-tier procedure is recorded but
        /// quarantined: this transport carries no `IdentityProvider`, so the
        /// reporter's assurance is `none` and one executor cannot tank a shared
        /// how-to (design §3.3).
        #[tokio::test]
        async fn a_low_assurance_negative_report_is_quarantined() {
            let procedure = mk("omelette", DomainTierEnum::Project);
            let procedure_id = procedure.id;
            let service = service_with(vec![procedure]);

            let issued = service
                .query_procedures(Request::new(query_req("agent:cook-1")))
                .await
                .unwrap()
                .into_inner();
            let receipt_id = issued.procedures[0]
                .receipt
                .as_ref()
                .unwrap()
                .receipt_id
                .clone();

            let mut req = report_req(&receipt_id, "failure");
            req.failure_mode = Some("bad_result".into());

            let resp = service
                .report_outcome(Request::new(req))
                .await
                .unwrap()
                .into_inner();

            assert!(resp.accepted);
            assert!(resp.quarantined, "{}", resp.message);
            assert!(!resp.counted_as_failure);

            let store = service.store.lock().unwrap();
            let after = ProcedureStore::get_procedure(&*store, procedure_id)
                .unwrap()
                .unwrap();
            assert_eq!(after.failure_count, 0);
        }

        /// `executor_error` attributes the failure to the runner, not the
        /// how-to, so the procedure's failure counter stays put (design §3.3).
        #[tokio::test]
        async fn executor_error_does_not_blame_the_procedure() {
            let procedure = mk("omelette", DomainTierEnum::Task);
            let procedure_id = procedure.id;
            let service = service_with(vec![procedure]);

            let issued = service
                .query_procedures(Request::new(query_req("agent:cook-1")))
                .await
                .unwrap()
                .into_inner();
            let receipt_id = issued.procedures[0]
                .receipt
                .as_ref()
                .unwrap()
                .receipt_id
                .clone();

            let mut req = report_req(&receipt_id, "failure");
            req.failure_mode = Some("executor_error".into());

            let resp = service
                .report_outcome(Request::new(req))
                .await
                .unwrap()
                .into_inner();

            assert!(resp.attributed_to_executor, "{}", resp.message);
            assert!(!resp.counted_as_failure);

            let store = service.store.lock().unwrap();
            let after = ProcedureStore::get_procedure(&*store, procedure_id)
                .unwrap()
                .unwrap();
            assert_eq!(after.failure_count, 0);
        }

        /// A failure attribution on a success report would silently mis-file the
        /// outcome, so it is refused at the boundary.
        #[tokio::test]
        async fn failure_mode_on_a_success_is_rejected() {
            let service = service_with(vec![mk("omelette", DomainTierEnum::Task)]);

            let issued = service
                .query_procedures(Request::new(query_req("agent:cook-1")))
                .await
                .unwrap()
                .into_inner();
            let receipt_id = issued.procedures[0]
                .receipt
                .as_ref()
                .unwrap()
                .receipt_id
                .clone();

            let mut req = report_req(&receipt_id, "success");
            req.failure_mode = Some("bad_result".into());

            let err = service.report_outcome(Request::new(req)).await.unwrap_err();
            assert_eq!(err.code(), tonic::Code::InvalidArgument);
        }

        /// Reporting against a receipt this instance never issued is answered
        /// plainly rather than being silently counted.
        #[tokio::test]
        async fn reporting_an_unknown_receipt_is_not_accepted() {
            let service = service_with(vec![]);
            let unknown = ProcedureId::new().to_string();

            let resp = service
                .report_outcome(Request::new(report_req(&unknown, "success")))
                .await
                .unwrap()
                .into_inner();

            assert!(!resp.accepted);
            assert!(!resp.already_final);
        }

        /// The signature travels with the body: an executor needs the
        /// preconditions and parameters to decide whether the procedure applies.
        #[tokio::test]
        async fn the_wire_shape_carries_the_full_signature() {
            let service = service_with(vec![mk("omelette", DomainTierEnum::Task)]);

            let resp = service
                .query_procedures(Request::new(query_req("agent:cook-1")))
                .await
                .unwrap()
                .into_inner();

            let wire = resp.procedures[0].procedure.as_ref().unwrap();
            assert_eq!(wire.parameters.len(), 1);
            assert_eq!(wire.parameters[0].name, "eggs");
            assert_eq!(wire.preconditions.len(), 1);
            let check = wire.preconditions[0].check.as_ref().unwrap();
            assert_eq!(check.expect, "exists");
            assert_eq!(check.match_pattern.as_ref().unwrap().object, "eggs");
            assert_eq!(wire.required_tools, vec!["pan".to_string()]);
            assert_eq!(wire.est_duration_sec, Some(300));
        }

        /// An out-of-scope lookup must not issue a receipt. If it did, the
        /// probe would leave an obligation nobody can answer, and its expiry
        /// would count `unknown` against another namespace's procedure.
        #[tokio::test]
        async fn an_out_of_scope_lookup_issues_no_receipt() {
            let procedure = mk("omelette", DomainTierEnum::Task);
            let procedure_id = procedure.id;
            let service = service_with(vec![procedure]);

            let resp = service
                .get_procedure(Request::new(GetProcedureRequest {
                    id: procedure_id.to_string(),
                    issued_to: "agent:intruder".into(),
                    namespace_scope: Some("person:someone-else".into()),
                    task_id: None,
                    session_id: None,
                    auth_token: "token".into(),
                }))
                .await
                .unwrap()
                .into_inner();

            assert!(!resp.found);

            // No pending receipt was written, so nothing can expire against the
            // procedure later.
            let store = service.store.lock().unwrap();
            assert_eq!(
                store
                    .count_receipts(boswell_domain::ReceiptStatus::Pending)
                    .unwrap(),
                0
            );
        }

        /// An in-scope lookup still issues normally.
        #[tokio::test]
        async fn an_in_scope_lookup_issues() {
            let procedure = mk("omelette", DomainTierEnum::Task);
            let procedure_id = procedure.id;
            let service = service_with(vec![procedure]);

            let resp = service
                .get_procedure(Request::new(GetProcedureRequest {
                    id: procedure_id.to_string(),
                    issued_to: "agent:cook-1".into(),
                    namespace_scope: Some("person:jd".into()),
                    task_id: None,
                    session_id: None,
                    auth_token: "token".into(),
                }))
                .await
                .unwrap()
                .into_inner();

            assert!(resp.found, "{}", resp.message);
            assert!(resp.procedure.unwrap().receipt.is_some());
        }

        /// A claim-only store is distinguishable from a genuine no-match, so a
        /// caller is never told "no procedures" by a deployment that could never
        /// have had any.
        #[tokio::test]
        async fn a_claim_only_store_reports_unimplemented() {
            let service = BosWellServiceImpl::new(Arc::new(Mutex::new(MockStore)));

            let err = service
                .query_procedures(Request::new(query_req("agent:cook-1")))
                .await
                .unwrap_err();

            assert_eq!(err.code(), tonic::Code::Unimplemented);
        }
    }
}
