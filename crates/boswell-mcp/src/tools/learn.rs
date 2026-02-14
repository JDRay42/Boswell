//! Learn tool - Batch insert multiple claims

use boswell_sdk::BoswellClient;
use boswell_domain::Tier;
use serde::{Deserialize, Serialize};
use crate::error::McpError;

/// A single claim to be learned
#[derive(Debug, Deserialize)]
pub struct LearnClaim {
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

/// Parameters for batch learning
#[derive(Debug, Deserialize)]
pub struct LearnParams {
    /// List of claims to insert
    pub claims: Vec<LearnClaim>,
}

/// Result of batch learning
#[derive(Debug, Serialize)]
pub struct LearnResult {
    /// Number of claims successfully inserted
    pub success_count: usize,
    /// Total number of claims attempted
    pub total_count: usize,
    /// Claim IDs of successfully inserted claims
    pub claim_ids: Vec<String>,
    /// Error messages for failed claims
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
}

/// Handle boswell_learn tool invocation
///
/// Inserts multiple claims in batch mode.
///
/// # Arguments
///
/// * `client` - Boswell client instance
/// * `params` - Learn parameters with claims array
///
/// # Returns
///
/// Result containing insertion summary or an error
pub async fn handle_learn(
    client: &mut BoswellClient,
    params: LearnParams,
) -> Result<LearnResult, McpError> {
    let total_count = params.claims.len();
    let mut success_count = 0;
    let mut claim_ids = Vec::new();
    let mut errors = Vec::new();

    for (idx, claim) in params.claims.into_iter().enumerate() {
        // Parse tier if provided
        let tier = match claim.tier {
            Some(ref t) => {
                match t.parse::<Tier>() {
                    Ok(tier_enum) => Some(tier_enum),
                    Err(_) => {
                        errors.push(format!("Claim {}: Invalid tier '{}'", idx, t));
                        continue;
                    }
                }
            }
            None => None,
        };

        // Assert the claim
        match client
            .assert(
                &claim.namespace,
                &claim.subject,
                &claim.predicate,
                &claim.object,
                claim.confidence,
                tier,
            )
            .await
        {
            Ok(claim_id) => {
                success_count += 1;
                claim_ids.push(claim_id.to_string());
            }
            Err(e) => {
                errors.push(format!("Claim {}: {}", idx, e));
            }
        }
    }

    Ok(LearnResult {
        success_count,
        total_count,
        claim_ids,
        errors,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_learn_params_deserialize() {
        let json = r#"{
            "claims": [
                {
                    "namespace": "test",
                    "subject": "entity1",
                    "predicate": "hasProperty",
                    "object": "value1",
                    "confidence": 0.9
                },
                {
                    "namespace": "test",
                    "subject": "entity2",
                    "predicate": "relatesTo",
                    "object": "entity1"
                }
            ]
        }"#;

        let params: LearnParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.claims.len(), 2);
        assert_eq!(params.claims[0].subject, "entity1");
        assert_eq!(params.claims[1].confidence, None);
    }
}
