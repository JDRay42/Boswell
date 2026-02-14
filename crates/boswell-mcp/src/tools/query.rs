//! Query tool - Search for claims in Boswell

use boswell_sdk::{BoswellClient, QueryFilter};
use boswell_domain::{Claim, Tier};
use serde::{Deserialize, Serialize};
use crate::error::McpError;

/// Parameters for querying claims
#[derive(Debug, Deserialize)]
pub struct QueryParams {
    /// Filter by namespace
    #[serde(default)]
    pub namespace: Option<String>,
    /// Filter by subject
    #[serde(default)]
    pub subject: Option<String>,
    /// Filter by predicate
    #[serde(default)]
    pub predicate: Option<String>,
    /// Filter by object
    #[serde(default)]
    pub object: Option<String>,
    /// Minimum confidence threshold
    #[serde(default)]
    pub min_confidence: Option<f64>,
    /// Filter by tier
    #[serde(default)]
    pub tier: Option<String>,
}

/// Result of querying claims
#[derive(Debug, Serialize)]
pub struct QueryResult {
    /// Number of claims found
    pub count: usize,
    /// List of claims
    pub claims: Vec<ClaimInfo>,
}

/// Simplified claim information for display
#[derive(Debug, Serialize)]
pub struct ClaimInfo {
    /// Claim ID
    pub id: String,
    /// Namespace
    pub namespace: String,
    /// Subject
    pub subject: String,
    /// Predicate
    pub predicate: String,
    /// Object
    pub object: String,
    /// Confidence interval [lower, upper]
    pub confidence: (f64, f64),
    /// Tier
    pub tier: String,
}

impl From<Claim> for ClaimInfo {
    fn from(claim: Claim) -> Self {
        Self {
            id: claim.id.to_string(),
            namespace: claim.namespace,
            subject: claim.subject,
            predicate: claim.predicate,
            object: claim.object,
            confidence: claim.confidence,
            tier: claim.tier,
        }
    }
}

/// Handle boswell_query tool invocation
///
/// Queries claims from Boswell based on filters.
///
/// # Arguments
///
/// * `client` - Boswell client instance
/// * `params` - Query parameters
///
/// # Returns
///
/// Result containing matching claims or an error
pub async fn handle_query(
    client: &mut BoswellClient,
    params: QueryParams,
) -> Result<QueryResult, McpError> {
    // Parse tier if provided
    let tier = match params.tier {
        Some(ref t) => {
            let tier_enum = t.parse::<Tier>()
                .map_err(|_| McpError::InvalidRequest(format!("Invalid tier: {}", t)))?;
            Some(tier_enum)
        }
        None => None,
    };

    // Build query filter
    let filter = QueryFilter {
        namespace: params.namespace,
        subject: params.subject,
        predicate: params.predicate,
        object: params.object,
        min_confidence: params.min_confidence,
        tier,
    };

    // Execute query
    let claims = client
        .query(filter)
        .await
        .map_err(|e| McpError::BoswellError(e.to_string()))?;

    let count = claims.len();
    let claims_info: Vec<ClaimInfo> = claims.into_iter().map(ClaimInfo::from).collect();

    Ok(QueryResult {
        count,
        claims: claims_info,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_query_params_deserialize_all() {
        let json = r#"{
            "namespace": "test",
            "subject": "entity1",
            "predicate": "hasProperty",
            "object": "value1",
            "min_confidence": 0.8,
            "tier": "Permanent"
        }"#;

        let params: QueryParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.namespace, Some("test".to_string()));
        assert_eq!(params.min_confidence, Some(0.8));
    }

    #[test]
    fn test_query_params_deserialize_empty() {
        let json = "{}";
        let params: QueryParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.namespace, None);
        assert_eq!(params.subject, None);
    }
}
