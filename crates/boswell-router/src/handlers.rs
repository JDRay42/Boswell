//! HTTP request handlers for the Router service.
//!
//! Implements session establishment and health check endpoints using axum.

use crate::registry::InstanceRegistry;
use crate::session::{create_session_response, SessionManager, SessionResponse};
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::{get, post},
    Router as AxumRouter,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Shared application state
#[derive(Clone)]
pub struct AppState {
    /// Session manager for JWT token operations
    pub session_manager: Arc<SessionManager>,
    /// Instance registry for topology discovery
    pub registry: Arc<InstanceRegistry>,
}

/// Session establishment request
#[derive(Debug, Deserialize)]
pub struct EstablishSessionRequest {
    /// Optional user ID (for Phase 2, defaults to "default-user")
    #[serde(default = "default_user_id")]
    pub user_id: Option<String>,
}

fn default_user_id() -> Option<String> {
    Some("default-user".to_string())
}

/// Health check response
#[derive(Debug, Serialize, Deserialize)]
pub struct HealthCheckResponse {
    /// Overall health status
    pub status: String,
    /// Total number of registered instances
    pub instance_count: usize,
    /// Number of healthy instances
    pub healthy_instances: usize,
}

/// Error response
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    /// Error message
    pub error: String,
}

/// Application error type
#[derive(Debug)]
pub enum AppError {
    /// Session-related error
    SessionError(crate::session::SessionError),
    /// Registry-related error
    RegistryError(crate::registry::RegistryError),
    /// Internal server error
    InternalError(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            AppError::SessionError(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
            AppError::RegistryError(e) => (StatusCode::SERVICE_UNAVAILABLE, e.to_string()),
            AppError::InternalError(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg),
        };

        let body = Json(ErrorResponse { error: message });
        (status, body).into_response()
    }
}

impl From<crate::session::SessionError> for AppError {
    fn from(e: crate::session::SessionError) -> Self {
        AppError::SessionError(e)
    }
}

impl From<crate::registry::RegistryError> for AppError {
    fn from(e: crate::registry::RegistryError) -> Self {
        AppError::RegistryError(e)
    }
}

/// POST /session/establish - Establish a new session
///
/// Returns topology information (token + instances) per ADR-019.
async fn establish_session(
    State(state): State<AppState>,
    Json(request): Json<EstablishSessionRequest>,
) -> Result<Json<SessionResponse>, AppError> {
    let user_id = request.user_id.unwrap_or_else(|| "default-user".to_string());

    // Generate session token
    let token = state.session_manager.generate_token(&user_id)?;

    // Get instance topology
    let instances = state.registry.get_all_instances();

    if instances.is_empty() {
        return Err(AppError::InternalError(
            "No instances registered".to_string(),
        ));
    }

    // Create session response with topology
    let response = create_session_response(token, instances);

    Ok(Json(response))
}

/// GET /health - Aggregated health check
async fn health_check(State(state): State<AppState>) -> Json<HealthCheckResponse> {
    let instances = state.registry.get_all_instances();
    let healthy_instances = state.registry.get_healthy_instances();

    let status = if healthy_instances.is_empty() {
        "unhealthy"
    } else if healthy_instances.len() < instances.len() {
        "degraded"
    } else {
        "healthy"
    };

    Json(HealthCheckResponse {
        status: status.to_string(),
        instance_count: instances.len(),
        healthy_instances: healthy_instances.len(),
    })
}

/// Create the axum router with all routes
pub fn create_router(state: AppState) -> AxumRouter {
    AxumRouter::new()
        .route("/session/establish", post(establish_session))
        .route("/health", get(health_check))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::InstanceConfig;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt; // for oneshot

    fn create_test_state() -> AppState {
        let session_manager = Arc::new(SessionManager::new("test-secret", 3600));
        let registry = Arc::new(InstanceRegistry::from_config(vec![InstanceConfig {
            id: "test-instance".to_string(),
            endpoint: "http://localhost:50051".to_string(),
            expertise: vec!["*".to_string()],
        }]));

        AppState {
            session_manager,
            registry,
        }
    }

    #[tokio::test]
    async fn test_health_check() {
        let state = create_test_state();
        let app = create_router(state);

        let request = Request::builder()
            .uri("/health")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_establish_session() {
        let state = create_test_state();
        let app = create_router(state);

        let request = Request::builder()
            .method("POST")
            .uri("/session/establish")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"user_id": "test-user"}"#))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}
