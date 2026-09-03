//! HTTP/JSON request handlers for the gateway `/v1` API.
//!
//! Each handler authenticates via the [`AuthContext`] injected by the auth
//! middleware, enforces scope and namespace isolation, then delegates to the
//! in-repo [`BoswellClient`](boswell_sdk::BoswellClient). Mutations emit an audit
//! log line (key id, namespace, operation, count).

use axum::extract::{Path, Query, State};
use axum::{Extension, Json};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::{SystemTime, UNIX_EPOCH};

use boswell_domain::{Claim, ClaimId, Relationship, RelationshipType, Tier};
use boswell_sdk::QueryFilter;

use crate::auth::{AuthContext, Scope};
use crate::error::ApiError;
use crate::state::AppState;

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

/// JSON representation of a claim (mirrors the CLI claim DTO, plus source_type).
#[derive(Debug, Serialize)]
pub struct ClaimDto {
    id: String,
    namespace: String,
    subject: String,
    predicate: String,
    object: String,
    confidence: ConfidenceDto,
    tier: String,
    source_type: String,
    created_at: u64,
    stale_at: Option<u64>,
}

#[derive(Debug, Serialize)]
struct ConfidenceDto {
    lower: f64,
    upper: f64,
}

impl From<&Claim> for ClaimDto {
    fn from(c: &Claim) -> Self {
        ClaimDto {
            id: c.id.to_string(),
            namespace: c.namespace.clone(),
            subject: c.subject.clone(),
            predicate: c.predicate.clone(),
            object: c.object.clone(),
            confidence: ConfidenceDto {
                lower: c.confidence.0,
                upper: c.confidence.1,
            },
            tier: c.tier.clone(),
            source_type: c.source_type.clone(),
            created_at: c.created_at,
            stale_at: c.stale_at,
        }
    }
}

/// JSON representation of a relationship between two claims.
#[derive(Debug, Serialize)]
pub struct RelationshipDto {
    from_claim: String,
    to_claim: String,
    relationship_type: String,
    strength: f64,
    created_at: u64,
}

fn relationship_type_str(rt: RelationshipType) -> &'static str {
    match rt {
        RelationshipType::Supports => "supports",
        RelationshipType::Contradicts => "contradicts",
        RelationshipType::DerivedFrom => "derived_from",
        RelationshipType::References => "references",
        RelationshipType::Supersedes => "supersedes",
    }
}

impl From<&Relationship> for RelationshipDto {
    fn from(r: &Relationship) -> Self {
        RelationshipDto {
            from_claim: r.from_claim.to_string(),
            to_claim: r.to_claim.to_string(),
            relationship_type: relationship_type_str(r.relationship_type).to_string(),
            strength: r.strength,
            created_at: r.created_at,
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Parse an optional tier string into a domain [`Tier`], erroring on unknown.
fn parse_tier(tier: &Option<String>) -> Result<Option<Tier>, ApiError> {
    match tier {
        Some(s) if !s.trim().is_empty() => Tier::parse(s)
            .map(Some)
            .ok_or_else(|| ApiError::bad_request(format!("invalid tier: '{}'", s))),
        _ => Ok(None),
    }
}

/// Truncate free text used as a claim object, keeping claims bounded.
fn truncate(text: &str, max: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max {
        trimmed.to_string()
    } else {
        let kept: String = trimmed.chars().take(max).collect();
        format!("{}…", kept)
    }
}

fn audit(ctx: &AuthContext, operation: &str, namespace: &str, count: usize) {
    tracing::info!(
        key_id = %ctx.key_id,
        namespace = %namespace,
        operation = %operation,
        count = count,
        "gateway mutation"
    );
}

// ---------------------------------------------------------------------------
// Health
// ---------------------------------------------------------------------------

/// `GET /v1/health` — gateway liveness plus best-effort instance health.
/// Unauthenticated. Always 200 if the gateway process is up.
pub async fn health(State(state): State<AppState>) -> Json<Value> {
    let mut client = state.client().lock().await;
    match client.health().await {
        Ok(h) => Json(json!({
            "status": "ok",
            "service": "boswell-gateway",
            "instance": {
                "status": h.status,
                "version": h.version,
                "uptime_seconds": h.uptime_seconds,
                "claim_count": h.claim_count,
            }
        })),
        Err(e) => Json(json!({
            "status": "degraded",
            "service": "boswell-gateway",
            "instance": { "status": "unavailable", "detail": e.to_string() }
        })),
    }
}

// ---------------------------------------------------------------------------
// Claims: assert / batch / query / get / relationships / delete
// ---------------------------------------------------------------------------

/// Body for `POST /v1/claims`.
#[derive(Debug, Deserialize)]
pub struct AssertBody {
    namespace: String,
    subject: String,
    predicate: String,
    object: String,
    /// Point confidence in [0,1]; defaults to 0.9 when omitted.
    confidence: Option<f64>,
    tier: Option<String>,
}

/// `POST /v1/claims` — assert a single claim.
pub async fn assert_claim(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthContext>,
    Json(body): Json<AssertBody>,
) -> Result<Json<Value>, ApiError> {
    ctx.require(Scope::Write)?;
    ctx.require_namespace(&body.namespace)?;

    if body.subject.trim().is_empty() || body.predicate.trim().is_empty() {
        return Err(ApiError::bad_request("subject and predicate are required"));
    }
    let tier = parse_tier(&body.tier)?;
    let confidence = body.confidence.unwrap_or(0.9);
    if !(0.0..=1.0).contains(&confidence) {
        return Err(ApiError::bad_request("confidence must be in [0, 1]"));
    }

    let mut client = state.client().lock().await;
    client.ensure_connected().await?;
    let id = client
        .assert(
            &body.namespace,
            &body.subject,
            &body.predicate,
            &body.object,
            Some(confidence),
            tier,
        )
        .await?;

    audit(&ctx, "assert", &body.namespace, 1);
    Ok(Json(json!({ "id": id.to_string() })))
}

/// One claim in a batch `POST /v1/claims/batch`.
#[derive(Debug, Deserialize)]
pub struct ClaimInput {
    namespace: String,
    subject: String,
    predicate: String,
    object: String,
    confidence: Option<ConfidenceInput>,
    tier: Option<String>,
    source_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ConfidenceInput {
    lower: f64,
    upper: f64,
}

/// Body for `POST /v1/claims/batch`.
#[derive(Debug, Deserialize)]
pub struct BatchBody {
    claims: Vec<ClaimInput>,
}

/// `POST /v1/claims/batch` — bulk-learn many claims.
pub async fn batch_learn(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthContext>,
    Json(body): Json<BatchBody>,
) -> Result<Json<Value>, ApiError> {
    ctx.require(Scope::Write)?;
    if body.claims.is_empty() {
        return Err(ApiError::bad_request("claims must not be empty"));
    }

    let now = now_secs();
    let mut domain_claims = Vec::with_capacity(body.claims.len());
    for input in &body.claims {
        ctx.require_namespace(&input.namespace)?;
        let tier = parse_tier(&input.tier)?.unwrap_or(Tier::Task);
        let (lower, upper) = match &input.confidence {
            Some(c) => (c.lower, c.upper),
            None => (0.9, 0.9),
        };
        if !(0.0..=1.0).contains(&lower) || !(0.0..=1.0).contains(&upper) || lower > upper {
            return Err(ApiError::bad_request(
                "confidence bounds must satisfy 0 <= lower <= upper <= 1",
            ));
        }
        let mut claim = Claim::new(
            ClaimId::new(),
            input.namespace.clone(),
            input.subject.clone(),
            input.predicate.clone(),
            input.object.clone(),
            (lower, upper),
            tier.as_str().to_string(),
            now,
        );
        if let Some(st) = &input.source_type {
            if !st.trim().is_empty() {
                claim = claim.with_source_type(st.clone());
            }
        }
        domain_claims.push(claim);
    }

    let count = domain_claims.len();
    let mut client = state.client().lock().await;
    client.ensure_connected().await?;
    let result = client.learn(domain_claims).await?;

    audit(&ctx, "batch_learn", "(multiple)", count);
    Ok(Json(json!({
        "inserted_count": result.inserted_count,
        "duplicate_count": result.duplicate_count,
        "error_count": result.error_count,
        "errors": result.errors,
    })))
}

/// Query string for `GET /v1/claims`.
#[derive(Debug, Deserialize)]
pub struct QueryParams {
    namespace: Option<String>,
    subject: Option<String>,
    predicate: Option<String>,
    object: Option<String>,
    tier: Option<String>,
    min_confidence: Option<f64>,
    source_type: Option<String>,
    #[allow(dead_code)]
    limit: Option<usize>,
}

/// `GET /v1/claims` — query claims with filters.
pub async fn query_claims(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthContext>,
    Query(params): Query<QueryParams>,
) -> Result<Json<Vec<ClaimDto>>, ApiError> {
    ctx.require(Scope::Read)?;
    let namespace = ctx.read_namespace(params.namespace)?;
    let tier = parse_tier(&params.tier)?;

    let filter = QueryFilter {
        namespace,
        subject: params.subject,
        predicate: params.predicate,
        object: params.object,
        min_confidence: params.min_confidence,
        tier,
        source_type: params.source_type,
    };

    let mut client = state.client().lock().await;
    client.ensure_connected().await?;
    let claims = client.query(filter).await?;

    // Defensive namespace isolation on top of the server-side prefix filter.
    let dtos: Vec<ClaimDto> = claims
        .iter()
        .filter(|c| ctx.allows_namespace(&c.namespace))
        .map(ClaimDto::from)
        .collect();
    Ok(Json(dtos))
}

/// `GET /v1/claims/{id}` — fetch one claim.
pub async fn get_claim(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthContext>,
    Path(id): Path<String>,
) -> Result<Json<ClaimDto>, ApiError> {
    ctx.require(Scope::Read)?;
    let claim_id = ClaimId::from_string(&id)
        .map_err(|e| ApiError::bad_request(format!("invalid id: {}", e)))?;

    let mut client = state.client().lock().await;
    client.ensure_connected().await?;
    let claim = client.get_claim(claim_id).await?;

    match claim {
        Some(c) if ctx.allows_namespace(&c.namespace) => Ok(Json(ClaimDto::from(&c))),
        // Hide claims outside the key's namespace behind the same 404 as missing.
        _ => Err(ApiError::not_found("claim not found")),
    }
}

/// `GET /v1/claims/{id}/relationships` — provenance / contradiction graph.
pub async fn get_relationships(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthContext>,
    Path(id): Path<String>,
) -> Result<Json<Vec<RelationshipDto>>, ApiError> {
    ctx.require(Scope::Read)?;
    let claim_id = ClaimId::from_string(&id)
        .map_err(|e| ApiError::bad_request(format!("invalid id: {}", e)))?;

    let mut client = state.client().lock().await;
    client.ensure_connected().await?;

    // Ownership check: only expose relationships for a claim in the key's scope.
    match client.get_claim(claim_id).await? {
        Some(c) if ctx.allows_namespace(&c.namespace) => {}
        _ => return Err(ApiError::not_found("claim not found")),
    }

    let relationships = client.get_relationships(claim_id).await?;
    let dtos: Vec<RelationshipDto> = relationships.iter().map(RelationshipDto::from).collect();
    Ok(Json(dtos))
}

/// `DELETE /v1/claims/{id}` — forget (real delete).
pub async fn delete_claim(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthContext>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    ctx.require(Scope::Delete)?;
    let claim_id = ClaimId::from_string(&id)
        .map_err(|e| ApiError::bad_request(format!("invalid id: {}", e)))?;

    let mut client = state.client().lock().await;
    client.ensure_connected().await?;

    // Ownership check before deleting.
    let namespace = match client.get_claim(claim_id).await? {
        Some(c) if ctx.allows_namespace(&c.namespace) => c.namespace,
        _ => return Err(ApiError::not_found("claim not found")),
    };

    let deleted = client.forget(vec![claim_id]).await?;
    audit(&ctx, "delete", &namespace, 1);
    Ok(Json(json!({ "deleted": deleted })))
}

// ---------------------------------------------------------------------------
// Search / recall
// ---------------------------------------------------------------------------

/// Body for `POST /v1/search`.
#[derive(Debug, Deserialize)]
pub struct SearchBody {
    query: String,
    namespace: Option<String>,
    limit: Option<usize>,
    min_similarity: Option<f64>,
}

/// `POST /v1/search` — semantic search.
pub async fn search(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthContext>,
    Json(body): Json<SearchBody>,
) -> Result<Json<Value>, ApiError> {
    ctx.require(Scope::Read)?;
    if body.query.trim().is_empty() {
        return Err(ApiError::bad_request("query must not be empty"));
    }
    let namespace = ctx.read_namespace(body.namespace)?;
    let limit = body.limit.unwrap_or(10);
    let min_similarity = body.min_similarity.unwrap_or(0.0);

    let mut client = state.client().lock().await;
    client.ensure_connected().await?;
    let hits = client
        .search(&body.query, namespace, limit, min_similarity)
        .await?;

    let results: Vec<Value> = hits
        .iter()
        .filter(|(c, _)| ctx.allows_namespace(&c.namespace))
        .map(|(c, sim)| json!({ "claim": ClaimDto::from(c), "similarity": sim }))
        .collect();

    Ok(Json(json!({ "results": results })))
}

/// Body for `POST /v1/recall`.
#[derive(Debug, Deserialize)]
pub struct RecallBody {
    /// Structured subject to fetch claims about (exact match).
    subject: Option<String>,
    /// Free-text topic for semantic search.
    text: Option<String>,
    namespace: Option<String>,
    limit: Option<usize>,
    min_similarity: Option<f64>,
}

/// `POST /v1/recall` — one-call context: merge structured query (by subject)
/// with semantic search (by text), deduped and ranked (semantic hits first).
pub async fn recall(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthContext>,
    Json(body): Json<RecallBody>,
) -> Result<Json<Value>, ApiError> {
    ctx.require(Scope::Read)?;
    if body.subject.is_none() && body.text.is_none() {
        return Err(ApiError::bad_request(
            "at least one of 'subject' or 'text' is required",
        ));
    }
    let namespace = ctx.read_namespace(body.namespace)?;
    let limit = body.limit.unwrap_or(10);
    let min_similarity = body.min_similarity.unwrap_or(0.0);

    let mut client = state.client().lock().await;
    client.ensure_connected().await?;

    let mut seen = std::collections::HashSet::new();
    let mut results: Vec<Value> = Vec::new();

    // 1. Semantic hits first (ranked by similarity), if text was provided.
    if let Some(text) = &body.text {
        if !text.trim().is_empty() {
            let hits = client
                .search(text, namespace.clone(), limit, min_similarity)
                .await?;
            for (claim, sim) in hits {
                if !ctx.allows_namespace(&claim.namespace) {
                    continue;
                }
                if seen.insert(claim.id.to_string()) {
                    results.push(json!({ "claim": ClaimDto::from(&claim), "similarity": sim }));
                }
            }
        }
    }

    // 2. Structured matches by subject.
    if let Some(subject) = &body.subject {
        if !subject.trim().is_empty() {
            let filter = QueryFilter {
                namespace: namespace.clone(),
                subject: Some(subject.clone()),
                ..Default::default()
            };
            let claims = client.query(filter).await?;
            for claim in claims {
                if !ctx.allows_namespace(&claim.namespace) {
                    continue;
                }
                if seen.insert(claim.id.to_string()) {
                    results.push(json!({ "claim": ClaimDto::from(&claim), "similarity": null }));
                }
            }
        }
    }

    results.truncate(limit.max(results.len().min(limit)));
    Ok(Json(json!({ "results": results })))
}

// ---------------------------------------------------------------------------
// Extract
// ---------------------------------------------------------------------------

/// Body for `POST /v1/extract`.
#[derive(Debug, Deserialize)]
pub struct ExtractBody {
    text: String,
    namespace: String,
    tier: Option<String>,
    source_id: Option<String>,
}

/// `POST /v1/extract` — text → claims via the server-side LLM Extractor.
pub async fn extract(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthContext>,
    Json(body): Json<ExtractBody>,
) -> Result<Json<Value>, ApiError> {
    ctx.require(Scope::Write)?;
    ctx.require_namespace(&body.namespace)?;
    if body.text.trim().is_empty() {
        return Err(ApiError::bad_request("text must not be empty"));
    }
    // Validate tier eagerly for a clean 400 rather than an upstream error.
    let tier = parse_tier(&body.tier)?;
    let tier_str = tier.map(|t| t.as_str().to_string()).unwrap_or_default();
    let source_id = body.source_id.unwrap_or_default();

    let mut client = state.client().lock().await;
    client.ensure_connected().await?;
    let result = client
        .extract(&body.text, &body.namespace, &tier_str, &source_id)
        .await?;

    audit(&ctx, "extract", &body.namespace, result.created_count);
    Ok(Json(extract_result_json(&result)))
}

fn extract_result_json(result: &boswell_sdk::ExtractResult) -> Value {
    let created: Vec<ClaimDto> = result.claims_created.iter().map(ClaimDto::from).collect();
    json!({
        "claims_created": created,
        "created_count": result.created_count,
        "corroborated_count": result.corroborated_count,
        "failed_count": result.failed_count,
        "failures": result.failures,
    })
}

// ---------------------------------------------------------------------------
// Hook ingest
// ---------------------------------------------------------------------------

/// A Claude Code hook event (a subset of the fields; unknown fields ignored).
#[derive(Debug, Deserialize)]
pub struct HookEvent {
    #[serde(alias = "hookEventName")]
    hook_event_name: Option<String>,
    #[serde(alias = "toolName")]
    tool_name: Option<String>,
    #[serde(alias = "toolInput")]
    tool_input: Option<Value>,
    prompt: Option<String>,
    #[serde(alias = "sessionId")]
    session_id: Option<String>,
    /// Optional namespace override; defaults to the key's namespace.
    namespace: Option<String>,
}

/// Query string for `POST /v1/hooks/ingest`.
#[derive(Debug, Deserialize)]
pub struct IngestParams {
    /// `deterministic` (default) or `llm`.
    mode: Option<String>,
}

/// `POST /v1/hooks/ingest` — Claude Code hook event → claims.
///
/// Default mode is deterministic mapping via `Learn`. `?mode=llm` routes the
/// salient text through the LLM Extractor; if extraction is unavailable, it
/// falls back to the deterministic mapping so ingest keeps working.
pub async fn hooks_ingest(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthContext>,
    Query(params): Query<IngestParams>,
    Json(event): Json<HookEvent>,
) -> Result<Json<Value>, ApiError> {
    ctx.require(Scope::Write)?;

    // Resolve target namespace: explicit override, else the key's namespace,
    // else a sensible default for unrestricted keys.
    let namespace = match &event.namespace {
        Some(ns) if !ns.trim().is_empty() => ns.clone(),
        _ => {
            if ctx.namespace.is_empty() || ctx.namespace == "*" {
                "agent".to_string()
            } else {
                ctx.namespace.clone()
            }
        }
    };
    ctx.require_namespace(&namespace)?;

    let mode = params.mode.as_deref().unwrap_or("deterministic");

    if mode == "llm" {
        if let Some(text) = salient_text(&event) {
            let mut client = state.client().lock().await;
            client.ensure_connected().await?;
            let source_id = event
                .session_id
                .clone()
                .map(|s| format!("hook:{}", s))
                .unwrap_or_else(|| "hook".to_string());
            match client.extract(&text, &namespace, "task", &source_id).await {
                Ok(result) => {
                    audit(&ctx, "hooks_ingest:llm", &namespace, result.created_count);
                    return Ok(Json(json!({
                        "mode": "llm",
                        "ingested": result.created_count,
                        "result": extract_result_json(&result),
                    })));
                }
                Err(e) => {
                    // Fall back to deterministic mapping so ingest keeps working.
                    tracing::warn!(
                        "LLM ingest failed ({}); falling back to deterministic mapping",
                        e
                    );
                }
            }
        }
    }

    // Deterministic mapping → Learn.
    let claims = map_hook_event(&event, &namespace, now_secs());
    if claims.is_empty() {
        return Ok(Json(json!({ "mode": "deterministic", "ingested": 0 })));
    }
    let count = claims.len();
    let mut client = state.client().lock().await;
    client.ensure_connected().await?;
    let result = client.learn(claims).await?;
    audit(&ctx, "hooks_ingest:deterministic", &namespace, count);
    Ok(Json(json!({
        "mode": "deterministic",
        "ingested": result.inserted_count,
        "error_count": result.error_count,
    })))
}

/// Pick the most salient free text from a hook event for LLM extraction.
fn salient_text(event: &HookEvent) -> Option<String> {
    if let Some(prompt) = &event.prompt {
        if !prompt.trim().is_empty() {
            return Some(prompt.clone());
        }
    }
    if let Some(input) = &event.tool_input {
        let s = input.to_string();
        if s.len() > 2 {
            return Some(s);
        }
    }
    None
}

/// Deterministically map a hook event to zero or more claims.
fn map_hook_event(event: &HookEvent, namespace: &str, now: u64) -> Vec<Claim> {
    let session = event
        .session_id
        .clone()
        .unwrap_or_else(|| "unknown".to_string());
    let subject = format!("agent:{}", session);
    let name = event.hook_event_name.as_deref().unwrap_or("");

    let mut claims = Vec::new();
    let mut push = |predicate: &str, object: String, tier: Tier| {
        if object.trim().is_empty() {
            return;
        }
        claims.push(
            Claim::new(
                ClaimId::new(),
                namespace.to_string(),
                subject.clone(),
                predicate.to_string(),
                object,
                (0.9, 1.0),
                tier.as_str().to_string(),
                now,
            )
            .with_source_type(Claim::SOURCE_ASSERTION),
        );
    };

    match name {
        "UserPromptSubmit" => {
            if let Some(prompt) = &event.prompt {
                push("submitted_prompt", truncate(prompt, 500), Tier::Ephemeral);
            }
        }
        "PostToolUse" => {
            let tool = event.tool_name.as_deref().unwrap_or("");
            match tool {
                "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => {
                    if let Some(path) = tool_input_str(event, "file_path") {
                        push("edited", format!("file:{}", path), Tier::Task);
                    }
                }
                "Bash" => {
                    if let Some(cmd) = tool_input_str(event, "command") {
                        push("ran_command", truncate(&cmd, 500), Tier::Task);
                    }
                }
                other if !other.is_empty() => {
                    push("used_tool", format!("tool:{}", other), Tier::Ephemeral);
                }
                _ => {}
            }
        }
        // SessionStart is a recall event, Stop/SubagentStop carry no structured
        // triple by default — nothing deterministic to assert.
        _ => {}
    }

    claims
}

/// Read a string field out of the event's `tool_input` JSON object.
fn tool_input_str(event: &HookEvent, key: &str) -> Option<String> {
    event
        .tool_input
        .as_ref()
        .and_then(|v| v.get(key))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(json: Value) -> HookEvent {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn test_map_post_tool_use_edit() {
        let e = event(json!({
            "hook_event_name": "PostToolUse",
            "tool_name": "Edit",
            "tool_input": { "file_path": "/src/main.rs" },
            "session_id": "s1"
        }));
        let claims = map_hook_event(&e, "agent", 100);
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].subject, "agent:s1");
        assert_eq!(claims[0].predicate, "edited");
        assert_eq!(claims[0].object, "file:/src/main.rs");
        assert_eq!(claims[0].tier, "task");
        assert_eq!(claims[0].namespace, "agent");
    }

    #[test]
    fn test_map_user_prompt_submit() {
        let e = event(json!({
            "hook_event_name": "UserPromptSubmit",
            "prompt": "add a login page",
            "session_id": "s2"
        }));
        let claims = map_hook_event(&e, "agent", 100);
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].predicate, "submitted_prompt");
        assert_eq!(claims[0].object, "add a login page");
        assert_eq!(claims[0].tier, "ephemeral");
    }

    #[test]
    fn test_map_session_start_is_empty() {
        let e = event(json!({ "hook_event_name": "SessionStart", "session_id": "s3" }));
        assert!(map_hook_event(&e, "agent", 100).is_empty());
    }

    #[test]
    fn test_camelcase_aliases_parse() {
        let e = event(json!({
            "hookEventName": "PostToolUse",
            "toolName": "Bash",
            "toolInput": { "command": "cargo test" },
            "sessionId": "s4"
        }));
        let claims = map_hook_event(&e, "agent", 100);
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].predicate, "ran_command");
        assert_eq!(claims[0].object, "cargo test");
    }

    #[test]
    fn test_truncate() {
        assert_eq!(truncate("  hi  ", 10), "hi");
        assert_eq!(truncate("abcdef", 3), "abc…");
    }
}
