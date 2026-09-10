//! Procedure retrieval and outcome reporting — `boswell_query_procedures`,
//! `boswell_get_procedure`, `boswell_report_outcome` (design 15 §3.3).
//!
//! Retrieval is not free. Every procedure handed out carries an execution
//! receipt, and whoever holds it owes a report before it expires; an unanswered
//! receipt counts as `unknown` against the procedure, because silence is not
//! success. The two retrieval tools therefore return the receipt alongside the
//! procedure, in the shape design §3.3 specifies, so the model can see the
//! obligation it just took on and answer it with `boswell_report_outcome`.
//!
//! `issued_to` is **not** a tool parameter. The server names the principal, the
//! way the gateway names it from the API key rather than from the request body:
//! a caller that picks its own name on a receipt is not accountable for it. The
//! CLI's `--as` flag is the deliberate exception, and an operator at a terminal
//! is not the same trust as a model choosing arguments.
//!
//! The JSON shapes here are the gateway's, verbatim — see [`crate::tools::goals`]
//! for why.

use crate::error::McpError;
use boswell_sdk::{BoswellClient, IssuedProcedure, OutcomeReportSpec, ProcedureQuerySpec};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Parameters for `boswell_query_procedures`.
#[derive(Debug, Deserialize)]
pub struct QueryProceduresParams {
    /// Filter by namespace prefix.
    #[serde(default)]
    pub namespace: Option<String>,
    /// Filter to a single `goal` grouping key.
    #[serde(default)]
    pub goal: Option<String>,
    /// Filter by a case-insensitive substring of `intent`.
    #[serde(default)]
    pub intent_contains: Option<String>,
    /// Include superseded (non-current) versions.
    #[serde(default)]
    pub include_superseded: bool,
    /// Maximum number of results.
    #[serde(default)]
    pub limit: Option<u32>,
    /// Correlation: task id, stamped onto the issued receipts.
    #[serde(default)]
    pub task_id: Option<String>,
    /// Correlation: session id, stamped onto the issued receipts.
    #[serde(default)]
    pub session_id: Option<String>,
}

/// Result of `boswell_query_procedures`.
#[derive(Debug, Serialize)]
pub struct QueryProceduresResult {
    /// Number of procedures issued — and receipts now outstanding.
    pub count: usize,
    /// Each procedure with the receipt issued for it.
    pub procedures: Vec<IssuedProcedureDto>,
}

/// Parameters for `boswell_get_procedure`.
#[derive(Debug, Deserialize)]
pub struct GetProcedureParams {
    /// The procedure id.
    pub id: String,
    /// Confine the lookup to a namespace prefix. Checked *before* a receipt is
    /// issued, so an out-of-scope lookup leaves no obligation behind.
    #[serde(default)]
    pub namespace_scope: Option<String>,
    /// Correlation: task id, stamped onto the issued receipt.
    #[serde(default)]
    pub task_id: Option<String>,
    /// Correlation: session id, stamped onto the issued receipt.
    #[serde(default)]
    pub session_id: Option<String>,
}

/// Result of `boswell_get_procedure`.
///
/// A miss is `found: false`, not a JSON-RPC error — see
/// [`crate::tools::goals::GetGoalResult`].
#[derive(Debug, Serialize)]
pub struct GetProcedureResult {
    /// Whether a procedure with that id exists inside the scope.
    pub found: bool,
    /// The procedure and its receipt, when `found`.
    pub procedure: Option<IssuedProcedureDto>,
}

/// A procedure rendered alongside the execution receipt issued for it.
#[derive(Debug, Serialize)]
pub struct IssuedProcedureDto {
    /// The procedure body and signature.
    pub procedure: Value,
    /// The receipt, including the fields a report must and may carry.
    pub receipt: Value,
}

/// Parameters for `boswell_report_outcome`.
#[derive(Debug, Deserialize)]
pub struct ReportOutcomeParams {
    /// The receipt this report answers.
    pub receipt_id: String,
    /// `success` | `failure` | `abandoned`.
    pub outcome: String,
    /// `preconditions_stale` | `step_failed` | `bad_result` | `executor_error`.
    #[serde(default)]
    pub failure_mode: Option<String>,
    /// Names the step, when `failure_mode` is `step_failed`.
    #[serde(default)]
    pub failed_step: Option<String>,
    /// The executor's self-assessed confidence.
    #[serde(default)]
    pub executor_confidence: Option<f64>,
    /// The reported cost (units are executor-defined).
    #[serde(default)]
    pub cost: Option<f64>,
    /// Free-form notes.
    #[serde(default)]
    pub notes: Option<String>,
}

/// Result of `boswell_report_outcome`.
///
/// `accepted: false` with `already_final: false` means no outstanding receipt
/// carried that id. A report can be accepted and still not move the counters:
/// a negative report from a low-assurance executor against a shared procedure
/// is recorded and `quarantined` rather than applied.
#[derive(Debug, Serialize)]
pub struct ReportOutcomeResult {
    /// Whether the report was recorded against a live receipt.
    pub accepted: bool,
    /// Whether the receipt had already been answered.
    pub already_final: bool,
    /// Whether the report moved the procedure's counters.
    pub applied: bool,
    /// Whether the report was held back for review instead of applied.
    pub quarantined: bool,
    /// What the report did, field by field.
    pub effect: Value,
    /// The store's explanation.
    pub message: String,
}

fn issued_procedure_to_dto(d: &IssuedProcedure) -> IssuedProcedureDto {
    let p = &d.procedure;
    let r = &d.receipt;

    let preconditions: Vec<Value> = p
        .preconditions
        .iter()
        .map(|pc| {
            json!({
                "kind": pc.kind,
                "description": pc.description,
                "check": {
                    "match": {
                        "subject": pc.check.match_pattern.subject,
                        "predicate": pc.check.match_pattern.predicate,
                        "object": pc.check.match_pattern.object,
                    },
                    "min_confidence": pc.check.min_confidence,
                    "expect": pc.check.expect.as_str(),
                },
            })
        })
        .collect();

    let parameters: Vec<Value> = p
        .parameters
        .iter()
        .map(|param| {
            json!({
                "name": param.name,
                "type": param.type_name,
                "default": param.default,
                "desc": param.desc,
            })
        })
        .collect();

    IssuedProcedureDto {
        procedure: json!({
            "id": p.id.to_string(),
            "namespace": p.namespace,
            "name": p.name,
            "version": p.version,
            "is_current": p.is_current,
            "source": p.source.as_str(),
            "goal": p.goal,
            "intent": p.intent,
            "tags": p.tags,
            "parameters": parameters,
            "preconditions": preconditions,
            "required_tools": p.required_tools,
            "postconditions": p.postconditions,
            "usage_notes": p.usage_notes,
            "context_tags": p.context_tags,
            "body_format": p.body_format.as_str(),
            "content_type": p.content_type,
            "body": p.body,
            "tier": p.tier.as_str(),
            "effectiveness": {
                "use_count": p.use_count,
                "success_count": p.success_count,
                "failure_count": p.failure_count,
                "unknown_count": p.unknown_count,
            },
            "est_duration_sec": p.est_duration_sec,
            "last_used_at": p.last_used_at,
            "created_at": p.created_at,
            "updated_at": p.updated_at,
        }),
        receipt: json!({
            "receipt_id": r.receipt_id.to_string(),
            "procedure_id": r.procedure_id.to_string(),
            "version": r.version,
            "issued_to": r.issued_to,
            "task_id": r.task_id,
            "session_id": r.session_id,
            "issued_at": r.issued_at,
            "expires_at": r.expires_at,
            "report_to": r.report_to,
            "required": ["outcome"],
            "optional": ["failure_mode", "executor_confidence", "cost", "notes"],
        }),
    }
}

/// Handle `boswell_query_procedures` — retrieve procedures for a goal or intent.
///
/// `issued_to` names the principal the server runs as, not anything the caller
/// supplied. Every returned procedure leaves a receipt outstanding.
pub async fn handle_query_procedures(
    client: &mut BoswellClient,
    params: QueryProceduresParams,
    issued_to: &str,
) -> Result<QueryProceduresResult, McpError> {
    let spec = ProcedureQuerySpec {
        issued_to: issued_to.to_string(),
        namespace: params.namespace,
        goal: params.goal,
        intent_contains: params.intent_contains,
        include_superseded: params.include_superseded,
        limit: params.limit,
        task_id: params.task_id,
        session_id: params.session_id,
    };

    let issued = client
        .query_procedures(spec)
        .await
        .map_err(|e| McpError::BoswellError(e.to_string()))?;

    let procedures: Vec<IssuedProcedureDto> = issued.iter().map(issued_procedure_to_dto).collect();
    Ok(QueryProceduresResult {
        count: procedures.len(),
        procedures,
    })
}

/// Handle `boswell_get_procedure` — fetch one procedure, issuing a receipt.
pub async fn handle_get_procedure(
    client: &mut BoswellClient,
    params: GetProcedureParams,
    issued_to: &str,
) -> Result<GetProcedureResult, McpError> {
    let issued = client
        .get_procedure(
            &params.id,
            issued_to,
            params.namespace_scope,
            params.task_id,
            params.session_id,
        )
        .await
        .map_err(|e| McpError::BoswellError(e.to_string()))?;

    Ok(GetProcedureResult {
        found: issued.is_some(),
        procedure: issued.as_ref().map(issued_procedure_to_dto),
    })
}

/// Handle `boswell_report_outcome` — answer an outstanding execution receipt.
pub async fn handle_report_outcome(
    client: &mut BoswellClient,
    params: ReportOutcomeParams,
) -> Result<ReportOutcomeResult, McpError> {
    let spec = OutcomeReportSpec {
        receipt_id: params.receipt_id,
        outcome: params.outcome,
        failure_mode: params.failure_mode,
        failed_step: params.failed_step,
        executor_confidence: params.executor_confidence,
        cost: params.cost,
        notes: params.notes,
    };

    let resp = client
        .report_outcome(spec)
        .await
        .map_err(|e| McpError::BoswellError(e.to_string()))?;

    Ok(ReportOutcomeResult {
        accepted: resp.accepted,
        already_final: resp.already_final,
        applied: resp.applied,
        quarantined: resp.quarantined,
        effect: json!({
            "counted_as_success": resp.counted_as_success,
            "counted_as_failure": resp.counted_as_failure,
            "attributed_to_executor": resp.attributed_to_executor,
            "flagged_precondition_stale": resp.flagged_precondition_stale,
        }),
        message: resp.message,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_procedures_params_omit_issued_to() {
        // The principal is the server's to name. A caller that sends one gets
        // it ignored rather than honored.
        let params: QueryProceduresParams =
            serde_json::from_str(r#"{"issued_to": "someone-else"}"#).unwrap();
        assert!(!params.include_superseded);
        assert_eq!(params.namespace, None);
    }

    #[test]
    fn query_procedures_params_default_to_current_versions_only() {
        let params: QueryProceduresParams = serde_json::from_str("{}").unwrap();
        assert!(!params.include_superseded);
        assert_eq!(params.limit, None);
    }

    #[test]
    fn get_procedure_params_require_only_an_id() {
        let params: GetProcedureParams = serde_json::from_str(r#"{"id": "01ABC"}"#).unwrap();
        assert_eq!(params.id, "01ABC");
        assert_eq!(params.namespace_scope, None);
        assert_eq!(params.task_id, None);
    }

    #[test]
    fn report_outcome_params_require_a_receipt_and_an_outcome() {
        let params: ReportOutcomeParams =
            serde_json::from_str(r#"{"receipt_id": "r1", "outcome": "success"}"#).unwrap();
        assert_eq!(params.receipt_id, "r1");
        assert_eq!(params.outcome, "success");
        assert_eq!(params.failure_mode, None);

        assert!(serde_json::from_str::<ReportOutcomeParams>(r#"{"outcome": "success"}"#).is_err());
        assert!(serde_json::from_str::<ReportOutcomeParams>(r#"{"receipt_id": "r1"}"#).is_err());
    }
}
