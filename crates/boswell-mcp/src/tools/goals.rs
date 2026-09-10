//! Goal traversal tools — `boswell_query_goals`, `boswell_get_goal`,
//! `boswell_expand_goal` (design 15 §3.2, §4.1).
//!
//! The three live in one file because they share the JSON shapes below.
//! Those shapes are the gateway's, verbatim: `GET /v1/goals`,
//! `GET /v1/goals/{id}` and `GET /v1/goals/{id}/expand` already render goals,
//! candidates and factor readings for an agent, and a second rendering of the
//! same domain types would be a second thing to keep in step.
//!
//! Traversal issues no execution receipt, so none of these tools names a
//! principal and none of them leaves an obligation behind. Only fetching a leaf
//! procedure does that — see [`crate::tools::procedures`].

use crate::error::McpError;
use boswell_domain::{ChildRef, ExpandedCandidate, FactorReading, Goal};
use boswell_sdk::{BoswellClient, GoalQuerySpec};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Parameters for `boswell_query_goals`.
#[derive(Debug, Deserialize)]
pub struct QueryGoalsParams {
    /// Filter by namespace prefix.
    #[serde(default)]
    pub namespace: Option<String>,
    /// Filter by a case-insensitive substring of `intent`.
    #[serde(default)]
    pub intent_contains: Option<String>,
    /// Maximum number of results.
    #[serde(default)]
    pub limit: Option<u32>,
}

/// Result of `boswell_query_goals`.
#[derive(Debug, Serialize)]
pub struct QueryGoalsResult {
    /// Number of goals found.
    pub count: usize,
    /// The goals, in the gateway's `GET /v1/goals` shape.
    pub goals: Vec<Value>,
}

/// Parameters for `boswell_get_goal`.
#[derive(Debug, Deserialize)]
pub struct GetGoalParams {
    /// The goal id.
    pub id: String,
    /// Confine the lookup to a namespace prefix.
    #[serde(default)]
    pub namespace_scope: Option<String>,
}

/// Result of `boswell_get_goal`.
///
/// A miss is reported as `found: false` rather than a JSON-RPC error: "no goal
/// with that id" is an answer about memory, not a malformed call, and the
/// error codes in [`McpError::error_code`] have no honest slot for it.
#[derive(Debug, Serialize)]
pub struct GetGoalResult {
    /// Whether a goal with that id exists inside the scope.
    pub found: bool,
    /// The goal, when `found`.
    pub goal: Option<Value>,
}

/// Parameters for `boswell_expand_goal`.
#[derive(Debug, Deserialize)]
pub struct ExpandGoalParams {
    /// The goal to expand.
    pub goal_id: String,
    /// Situational tags, e.g. `["time:quick"]`, matched against edge tags.
    #[serde(default)]
    pub context_tags: Vec<String>,
    /// Confine the lookup to a namespace prefix.
    #[serde(default)]
    pub namespace_scope: Option<String>,
}

/// Result of `boswell_expand_goal`.
#[derive(Debug, Serialize)]
pub struct ExpandGoalResult {
    /// Whether the goal exists inside the scope. `found: true` with no
    /// candidates is a real goal whose children were all filtered out.
    pub found: bool,
    /// Ranked `accomplish` children whose edge preconditions hold.
    pub candidates: Vec<Value>,
    /// `decide` children — procedures that help choose among the candidates.
    pub decision_aids: Vec<Value>,
    /// The claim readings consulted while filtering, so the caller can see why.
    pub factor_readings: Vec<Value>,
}

fn goal_to_json(g: &Goal) -> Value {
    json!({
        "id": g.id.to_string(),
        "namespace": g.namespace,
        "name": g.name,
        "intent": g.intent,
        "definition_of_done": g.definition_of_done,
        "tier": g.tier.as_str(),
        "created_at": g.created_at,
        "updated_at": g.updated_at,
        "stale_at": g.stale_at,
    })
}

fn candidate_to_json(c: &ExpandedCandidate) -> Value {
    json!({
        "child_kind": c.child.kind().as_str(),
        "child_id": match c.child {
            ChildRef::Goal(id) => id.to_string(),
            ChildRef::Procedure(id) => id.to_string(),
        },
        "role": c.role.as_str(),
        "context_tags": c.context_tags,
        "usage_notes": c.usage_notes,
        "effectiveness": c.effectiveness,
        "context_match": c.context_match,
    })
}

fn factor_reading_to_json(f: &FactorReading) -> Value {
    json!({
        "subject": f.subject,
        "predicate": f.predicate,
        "object": f.object,
        "confidence": { "lower": f.confidence.0, "upper": f.confidence.1 },
    })
}

/// Handle `boswell_query_goals` — the entry hop into a decomposition.
pub async fn handle_query_goals(
    client: &mut BoswellClient,
    params: QueryGoalsParams,
) -> Result<QueryGoalsResult, McpError> {
    let spec = GoalQuerySpec {
        namespace: params.namespace,
        intent_contains: params.intent_contains,
        limit: params.limit,
    };

    let goals = client
        .query_goals(spec)
        .await
        .map_err(|e| McpError::BoswellError(e.to_string()))?;

    let goals: Vec<Value> = goals.iter().map(goal_to_json).collect();
    Ok(QueryGoalsResult {
        count: goals.len(),
        goals,
    })
}

/// Handle `boswell_get_goal` — fetch one goal by id.
pub async fn handle_get_goal(
    client: &mut BoswellClient,
    params: GetGoalParams,
) -> Result<GetGoalResult, McpError> {
    let goal = client
        .get_goal(&params.id, params.namespace_scope)
        .await
        .map_err(|e| McpError::BoswellError(e.to_string()))?;

    Ok(GetGoalResult {
        found: goal.is_some(),
        goal: goal.as_ref().map(goal_to_json),
    })
}

/// Handle `boswell_expand_goal` — one traversal hop.
///
/// The store surfaces precondition-filtered, deterministically ranked
/// candidates; the weighting is the caller's. Traversal is stateless, so the
/// caller holds the cursor and calls this again on whichever child it picks.
pub async fn handle_expand_goal(
    client: &mut BoswellClient,
    params: ExpandGoalParams,
) -> Result<ExpandGoalResult, McpError> {
    let result = client
        .expand(&params.goal_id, params.context_tags, params.namespace_scope)
        .await
        .map_err(|e| McpError::BoswellError(e.to_string()))?;

    let Some(result) = result else {
        return Ok(ExpandGoalResult {
            found: false,
            candidates: Vec::new(),
            decision_aids: Vec::new(),
            factor_readings: Vec::new(),
        });
    };

    Ok(ExpandGoalResult {
        found: true,
        candidates: result.candidates.iter().map(candidate_to_json).collect(),
        decision_aids: result.decision_aids.iter().map(candidate_to_json).collect(),
        factor_readings: result
            .factor_readings
            .iter()
            .map(factor_reading_to_json)
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use boswell_domain::{EdgeRole, GoalId, ProcedureId, Tier};

    #[test]
    fn query_goals_params_default_to_all_none() {
        let params: QueryGoalsParams = serde_json::from_str("{}").unwrap();
        assert_eq!(params.namespace, None);
        assert_eq!(params.intent_contains, None);
        assert_eq!(params.limit, None);
    }

    #[test]
    fn expand_params_default_to_an_empty_context() {
        let params: ExpandGoalParams = serde_json::from_str(r#"{"goal_id": "abc"}"#).unwrap();
        assert!(params.context_tags.is_empty());
        assert_eq!(params.namespace_scope, None);
    }

    #[test]
    fn goal_json_matches_the_gateway_field_names() {
        let goal = Goal {
            id: GoalId::new(),
            namespace: "person:jd".to_string(),
            name: "prepare-breakfast".to_string(),
            intent: "get breakfast made".to_string(),
            definition_of_done: vec!["food on the table".to_string()],
            tier: Tier::Project,
            created_at: 1,
            updated_at: 2,
            stale_at: None,
        };

        let value = goal_to_json(&goal);
        assert_eq!(value["namespace"], "person:jd");
        assert_eq!(value["name"], "prepare-breakfast");
        assert_eq!(value["intent"], "get breakfast made");
        assert_eq!(value["definition_of_done"][0], "food on the table");
        assert_eq!(value["tier"], "project");
        assert_eq!(value["created_at"], 1);
        assert!(value["stale_at"].is_null());
    }

    #[test]
    fn candidate_json_names_the_child_kind_and_id() {
        let child = ProcedureId::new();
        let candidate = ExpandedCandidate {
            child: ChildRef::Procedure(child),
            role: EdgeRole::Accomplish,
            context_tags: vec!["time:quick".to_string()],
            usage_notes: "when in a hurry".to_string(),
            effectiveness: 0.75,
            context_match: 1,
        };

        let value = candidate_to_json(&candidate);
        assert_eq!(value["child_kind"], "procedure");
        assert_eq!(value["child_id"], child.to_string());
        assert_eq!(value["role"], "accomplish");
        assert_eq!(value["effectiveness"], 0.75);
        assert_eq!(value["context_match"], 1);
    }

    #[test]
    fn factor_reading_json_splits_the_confidence_interval() {
        let reading = FactorReading {
            subject: "fridge".to_string(),
            predicate: "contains".to_string(),
            object: "eggs".to_string(),
            confidence: (0.6, 0.9),
        };

        let value = factor_reading_to_json(&reading);
        assert_eq!(value["confidence"]["lower"], 0.6);
        assert_eq!(value["confidence"]["upper"], 0.9);
    }
}
