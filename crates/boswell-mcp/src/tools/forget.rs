//! Forget tool - Remove claims from Boswell

use boswell_sdk::BoswellClient;
use boswell_domain::ClaimId;
use serde::{Deserialize, Serialize};
use crate::error::McpError;

/// Parameters for forgetting claims
#[derive(Debug, Deserialize)]
pub struct ForgetParams {
    /// List of claim IDs to remove
    pub claim_ids: Vec<String>,
}

/// Result of forgetting claims
#[derive(Debug, Serialize)]
pub struct ForgetResult {
    /// Number of claims successfully removed
    pub success_count: usize,
    /// Total number of claims attempted
    pub total_count: usize,
    /// Error messages for failed removals
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
}

/// Handle boswell_forget tool invocation
///
/// Removes claims by their IDs.
///
/// # Arguments
///
/// * `client` - Boswell client instance
/// * `params` - Forget parameters with claim IDs
///
/// # Returns
///
/// Result containing removal summary or an error
pub async fn handle_forget(
    client: &mut BoswellClient,
    params: ForgetParams,
) -> Result<ForgetResult, McpError> {
    let total_count = params.claim_ids.len();
    let mut errors = Vec::new();
    let mut parsed_ids = Vec::new();

    // Parse all claim IDs first
    for claim_id_str in params.claim_ids {
        match ClaimId::from_string(&claim_id_str) {
            Ok(id) => parsed_ids.push(id),
            Err(e) => {
                errors.push(format!("Invalid claim ID '{}': {}", claim_id_str, e));
            }
        }
    }

    // If we have valid IDs, forget them
    let success_count = if !parsed_ids.is_empty() {
        match client.forget(parsed_ids).await {
            Ok(true) => total_count - errors.len(),
            Ok(false) => {
                errors.push("Forget operation returned false".to_string());
                0
            }
            Err(e) => {
                errors.push(format!("Forget operation failed: {}", e));
                0
            }
        }
    } else {
        0
    };

    Ok(ForgetResult {
        success_count,
        total_count,
        errors,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_forget_params_deserialize() {
        let json = r#"{
            "claim_ids": [
                "01HX5ZZKJQH5KW8F5N3D9T7G2A",
                "01HX5ZZKJQH5KW8F5N3D9T7G2B"
            ]
        }"#;

        let params: ForgetParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.claim_ids.len(), 2);
    }
}
