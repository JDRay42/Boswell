//! Error types for the Boswell SDK.

use thiserror::Error;

/// SDK operation errors
#[derive(Debug, Error)]
pub enum SdkError {
    /// Router connection or API error
    #[error("Router error: {0}")]
    RouterError(String),

    /// gRPC communication error
    #[error("gRPC error: {0}")]
    GrpcError(String),

    /// Session establishment or management error
    #[error("Session error: {0}")]
    SessionError(String),

    /// Connection error (network, DNS, etc.)
    #[error("Connection error: {0}")]
    ConnectionError(String),

    /// Authentication or authorization error
    #[error("Authentication error: {0}")]
    AuthError(String),

    /// No instances available
    #[error("No instances available")]
    NoInstancesAvailable,

    /// Client not connected (connect() must be called first)
    #[error("Client not connected - call connect() first")]
    NotConnected,
}

impl From<reqwest::Error> for SdkError {
    fn from(e: reqwest::Error) -> Self {
        if e.is_connect() {
            SdkError::ConnectionError(e.to_string())
        } else if e.is_status() {
            match e.status() {
                Some(status) if status.is_client_error() => {
                    SdkError::RouterError(format!("HTTP {}: {}", status, e))
                }
                Some(status) if status.is_server_error() => {
                    SdkError::RouterError(format!("Server error (HTTP {})", status))
                }
                _ => SdkError::RouterError(e.to_string()),
            }
        } else {
            SdkError::RouterError(e.to_string())
        }
    }
}

impl From<tonic::Status> for SdkError {
    fn from(status: tonic::Status) -> Self {
        use tonic::Code;
        
        match status.code() {
            Code::Unauthenticated => SdkError::AuthError(status.message().to_string()),
            Code::PermissionDenied => SdkError::AuthError(status.message().to_string()),
            Code::Unavailable => SdkError::ConnectionError(format!("gRPC unavailable: {}", status.message())),
            Code::DeadlineExceeded => SdkError::ConnectionError("Request timeout".to_string()),
            _ => SdkError::GrpcError(format!("{}: {}", status.code(), status.message())),
        }
    }
}

impl From<serde_json::Error> for SdkError {
    fn from(e: serde_json::Error) -> Self {
        SdkError::SessionError(format!("JSON parsing error: {}", e))
    }
}
