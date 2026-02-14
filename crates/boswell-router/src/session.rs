//! Session management with JWT tokens per ADR-019.
//!
//! Sessions are topology discovery handshakes. The client receives a token,
//! mode indicator, and instances array for direct routing.

use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Session management error
#[derive(Debug, Error)]
pub enum SessionError {
    /// JWT encoding failed
    #[error("Failed to encode JWT: {0}")]
    JwtEncode(#[from] jsonwebtoken::errors::Error),

    /// Token expired
    #[error("Session token expired")]
    TokenExpired,

    /// Invalid token
    #[error("Invalid session token")]
    InvalidToken,
}

/// JWT claims for session tokens
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionClaims {
    /// User identifier (from mTLS or auth header)
    pub user_id: String,

    /// Token expiration timestamp (Unix epoch)
    pub exp: u64,

    /// Issued at timestamp (Unix epoch)
    pub iat: u64,
}

/// Session response (topology discovery)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionResponse {
    /// JWT session token
    pub token: String,

    /// Deployment mode: "router" or "instance"
    pub mode: String,

    /// Available instances for client-side routing
    pub instances: Vec<InstanceInfo>,
}

/// Instance information in session response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceInfo {
    /// Instance ID
    pub id: String,

    /// gRPC endpoint
    pub endpoint: String,

    /// Expertise profile (namespaces this instance handles)
    pub expertise: Vec<String>,

    /// Health status: "healthy", "degraded", "unhealthy"
    pub health: String,
}

/// Session manager handles JWT token generation and validation
pub struct SessionManager {
    encoding_key: EncodingKey,
    decoding_key: DecodingKey,
    token_expiry_secs: u64,
}

impl SessionManager {
    /// Create a new session manager with the given JWT secret and expiry
    pub fn new(jwt_secret: &str, token_expiry_secs: u64) -> Self {
        Self {
            encoding_key: EncodingKey::from_secret(jwt_secret.as_bytes()),
            decoding_key: DecodingKey::from_secret(jwt_secret.as_bytes()),
            token_expiry_secs,
        }
    }

    /// Generate a new session token for the given user
    pub fn generate_token(&self, user_id: &str) -> Result<String, SessionError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let claims = SessionClaims {
            user_id: user_id.to_string(),
            exp: now + self.token_expiry_secs,
            iat: now,
        };

        let token = encode(&Header::default(), &claims, &self.encoding_key)?;
        Ok(token)
    }

    /// Validate a session token and extract claims
    pub fn validate_token(&self, token: &str) -> Result<SessionClaims, SessionError> {
        let validation = Validation::default();
        let token_data = decode::<SessionClaims>(token, &self.decoding_key, &validation)
            .map_err(|e| match e.kind() {
                jsonwebtoken::errors::ErrorKind::ExpiredSignature => SessionError::TokenExpired,
                _ => SessionError::InvalidToken,
            })?;

        Ok(token_data.claims)
    }
}

/// Create a session response with topology information
pub fn create_session_response(
    token: String,
    instances: Vec<InstanceInfo>,
) -> SessionResponse {
    let mode = if instances.len() == 1 {
        "instance"
    } else {
        "router"
    };

    SessionResponse {
        token,
        mode: mode.to_string(),
        instances,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_and_validate_token() {
        let manager = SessionManager::new("test-secret", 3600);
        let token = manager.generate_token("test-user").unwrap();

        let claims = manager.validate_token(&token).unwrap();
        assert_eq!(claims.user_id, "test-user");
    }

    #[test]
    fn test_expired_token() {
        use jsonwebtoken::{encode, Header};

        let manager = SessionManager::new("test-secret", 3600);

        // Create a token that's already expired (exp in the past)
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let claims = SessionClaims {
            user_id: "test-user".to_string(),
            exp: now - 100, // Expired 100 seconds ago
            iat: now - 200, // Issued 200 seconds ago
        };

        let token = encode(&Header::default(), &claims, &manager.encoding_key).unwrap();

        let result = manager.validate_token(&token);
        assert!(matches!(result, Err(SessionError::TokenExpired)));
    }

    #[test]
    fn test_invalid_token() {
        let manager = SessionManager::new("test-secret", 3600);
        let result = manager.validate_token("invalid-token");
        assert!(matches!(result, Err(SessionError::InvalidToken)));
    }

    #[test]
    fn test_wrong_secret() {
        let manager1 = SessionManager::new("secret1", 3600);
        let manager2 = SessionManager::new("secret2", 3600);

        let token = manager1.generate_token("test-user").unwrap();
        let result = manager2.validate_token(&token);
        assert!(matches!(result, Err(SessionError::InvalidToken)));
    }

    #[test]
    fn test_session_response_single_instance() {
        let instances = vec![InstanceInfo {
            id: "default".to_string(),
            endpoint: "http://localhost:50051".to_string(),
            expertise: vec!["*".to_string()],
            health: "healthy".to_string(),
        }];

        let response = create_session_response("token123".to_string(), instances);
        assert_eq!(response.mode, "instance");
        assert_eq!(response.instances.len(), 1);
    }

    #[test]
    fn test_session_response_multi_instance() {
        let instances = vec![
            InstanceInfo {
                id: "instance1".to_string(),
                endpoint: "http://localhost:50051".to_string(),
                expertise: vec!["domain1".to_string()],
                health: "healthy".to_string(),
            },
            InstanceInfo {
                id: "instance2".to_string(),
                endpoint: "http://localhost:50052".to_string(),
                expertise: vec!["domain2".to_string()],
                health: "healthy".to_string(),
            },
        ];

        let response = create_session_response("token123".to_string(), instances);
        assert_eq!(response.mode, "router");
        assert_eq!(response.instances.len(), 2);
    }
}
