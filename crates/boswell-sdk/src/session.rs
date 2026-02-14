//! Session management for Router communication.

use crate::error::SdkError;
use serde::{Deserialize, Serialize};

/// Session establishment request
#[derive(Debug, Serialize)]
pub struct EstablishSessionRequest {
    /// User ID (defaults to "default-user" for Phase 2)
    pub user_id: Option<String>,
}

/// Instance information from Router
#[derive(Debug, Clone, Deserialize)]
pub struct InstanceInfo {
    /// Instance ID
    pub id: String,
    /// gRPC endpoint
    pub endpoint: String,
    /// Expertise profile (namespaces)
    pub expertise: Vec<String>,
    /// Health status
    pub health: String,
}

/// Session response from Router
#[derive(Debug, Clone, Deserialize)]
pub struct SessionResponse {
    /// JWT session token
    pub token: String,
    /// Deployment mode
    pub mode: String,
    /// Available instances
    pub instances: Vec<InstanceInfo>,
}

/// Establish a session with the Router
pub async fn establish_session(
    http_client: &reqwest::Client,
    router_endpoint: &str,
) -> Result<SessionResponse, SdkError> {
    let url = format!("{}/session/establish", router_endpoint);
    
    let request = EstablishSessionRequest {
        user_id: Some("default-user".to_string()),
    };

    let response = http_client
        .post(&url)
        .json(&request)
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
        return Err(SdkError::RouterError(format!("HTTP {}: {}", status, error_text)));
    }

    let session_response: SessionResponse = response.json().await?;
    
    // Validate we have at least one instance
    if session_response.instances.is_empty() {
        return Err(SdkError::NoInstancesAvailable);
    }

    Ok(session_response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_response_parsing() {
        let json = r#"{
            "token": "eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9...",
            "mode": "router",
            "instances": [
                {
                    "id": "instance-1",
                    "endpoint": "http://localhost:50051",
                    "expertise": ["personal", "work"],
                    "health": "healthy"
                }
            ]
        }"#;

        let response: SessionResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.mode, "router");
        assert_eq!(response.instances.len(), 1);
        assert_eq!(response.instances[0].endpoint, "http://localhost:50051");
    }
}
