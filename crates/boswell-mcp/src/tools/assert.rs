//! Assert tool - Add a new claim to Boswell

use boswell_sdk::BoswellClient;
use boswell_domain::Tier;
use serde::{Deserialize, Serialize};
use crate::error::McpError;

/// Parameters for asserting a claim
#[derive(Debug, Deserialize)]
pub struct AssertParams {
    /// Namespace for the claim
    pub namespace: String,
    /// Subject (entity or concept)
    pub subject: String,
    /// Predicate (relationship or attribute)
    pub predicate: String,
    /// Object (value or related entity)
    pub object: String,
    /// Confidence score (0.0 - 1.0)
    #[serde(default)]
    pub confidence: Option<f64>,
    /// Tier (Transient, Session, Permanent)
    #[serde(default)]
    pub tier: Option<String>,
}

/// Result of asserting a claim
#[derive(Debug, Serialize)]
pub struct AssertResult {
    /// Unique claim ID (ULID)
    pub claim_id: String,
    /// Success message
    pub message: String,
}

/// Handle boswell_assert tool invocation
///
/// Asserts a new claim into Boswell with optional confidence and tier.
///
/// # Arguments
///
/// * `client` - Boswell client instance
/// * `params` - Assert parameters
///
/// # Returns
///
/// Result containing the claim ID or an error
pub async fn handle_assert(
    client: &mut BoswellClient,
    params: AssertParams,
) -> Result<AssertResult, McpError> {
    // Parse tier if provided (convert to lowercase string format)
    let tier = match params.tier {
        Some(ref t) => {
            // Validate tier by parsing
            let tier_enum = t.parse::<Tier>()
                .map_err(|_| McpError::InvalidRequest(format!("Invalid tier: {}", t)))?;
            Some(tier_enum)
        }
        None => None,
    };

    // Assert the claim
    let claim_id = client
        .assert(
            &params.namespace,
            &params.subject,
            &params.predicate,
            &params.object,
            params.confidence,
            tier,
        )
        .await
        .map_err(|e| McpError::BoswellError(e.to_string()))?;

    Ok(AssertResult {
        claim_id: claim_id.to_string(),
        message: format!("Claim asserted successfully"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_assert_params_deserialize() {
        let json = r#"{
            "namespace": "test",
            "subject": "entity1",
            "predicate": "hasProperty",
            "object": "value1",
            "confidence": 0.9,
            "tier": "Permanent"
        }"#;

        let params: AssertParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.namespace, "test");
        assert_eq!(params.subject, "entity1");
        assert_eq!(params.predicate, "hasProperty");
        assert_eq!(params.object, "value1");
        assert_eq!(params.confidence, Some(0.9));
        assert_eq!(params.tier, Some("Permanent".to_string()));
    }

    #[test]
    fn test_assert_params_optional_fields() {
        let json = r#"{
            "namespace": "test",
            "subject": "entity1",
            "predicate": "hasProperty",
            "object": "value1"
        }"#;

        let params: AssertParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.confidence, None);
        assert_eq!(params.tier, None);
    }
}
