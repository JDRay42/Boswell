//! Integration tests for the Router service

use boswell_router::{
    config::{InstanceConfig, RouterConfig},
    handlers::{create_router, AppState, HealthCheckResponse},
    registry::InstanceRegistry,
    session::{SessionManager, SessionResponse},
};
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use std::sync::Arc;
use tower::ServiceExt; // for oneshot

/// Helper to create test application state
fn create_test_state() -> AppState {
    let session_manager = Arc::new(SessionManager::new("test-secret-key", 3600));

    let instances = vec![
        InstanceConfig {
            id: "instance1".to_string(),
            endpoint: "http://localhost:50051".to_string(),
            expertise: vec!["domain1".to_string(), "domain2".to_string()],
        },
        InstanceConfig {
            id: "instance2".to_string(),
            endpoint: "http://localhost:50052".to_string(),
            expertise: vec!["domain3".to_string()],
        },
    ];

    let registry = Arc::new(InstanceRegistry::from_config(instances));

    AppState {
        session_manager,
        registry,
    }
}

#[tokio::test]
async fn test_health_check_endpoint() {
    let state = create_test_state();
    let app = create_router(state);

    let request = Request::builder()
        .method("GET")
        .uri("/health")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let health: HealthCheckResponse = serde_json::from_slice(&body).unwrap();

    assert_eq!(health.status, "healthy");
    assert_eq!(health.instance_count, 2);
    assert_eq!(health.healthy_instances, 2);
}

#[tokio::test]
async fn test_establish_session_with_user_id() {
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

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let session: SessionResponse = serde_json::from_slice(&body).unwrap();

    // Verify response structure
    assert!(!session.token.is_empty());
    assert_eq!(session.mode, "router"); // 2 instances = router mode
    assert_eq!(session.instances.len(), 2);

    // Verify instance information
    assert_eq!(session.instances[0].id, "instance1");
    assert_eq!(session.instances[0].endpoint, "http://localhost:50051");
    assert_eq!(session.instances[0].expertise, vec!["domain1", "domain2"]);
    assert_eq!(session.instances[0].health, "healthy");
}

#[tokio::test]
async fn test_establish_session_without_user_id() {
    let state = create_test_state();
    let app = create_router(state);

    let request = Request::builder()
        .method("POST")
        .uri("/session/establish")
        .header("content-type", "application/json")
        .body(Body::from(r#"{}"#))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let session: SessionResponse = serde_json::from_slice(&body).unwrap();

    assert!(!session.token.is_empty());
}

#[tokio::test]
async fn test_session_token_validation() {
    let session_manager = SessionManager::new("test-secret-key", 3600);

    // Generate a token
    let token = session_manager.generate_token("test-user").unwrap();

    // Validate the token
    let claims = session_manager.validate_token(&token).unwrap();
    assert_eq!(claims.user_id, "test-user");
}

#[tokio::test]
async fn test_single_instance_mode() {
    let session_manager = Arc::new(SessionManager::new("test-secret-key", 3600));

    let instances = vec![InstanceConfig {
        id: "single".to_string(),
        endpoint: "http://localhost:50051".to_string(),
        expertise: vec!["*".to_string()],
    }];

    let registry = Arc::new(InstanceRegistry::from_config(instances));

    let state = AppState {
        session_manager,
        registry,
    };

    let app = create_router(state);

    let request = Request::builder()
        .method("POST")
        .uri("/session/establish")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"user_id": "test-user"}"#))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let session: SessionResponse = serde_json::from_slice(&body).unwrap();

    // Single instance should return "instance" mode
    assert_eq!(session.mode, "instance");
    assert_eq!(session.instances.len(), 1);
}

#[test]
fn test_router_config_from_toml() {
    let toml = r#"
        bind_address = "0.0.0.0"
        bind_port = 9000
        jwt_secret = "my-secret-key"
        token_expiry_secs = 7200

        [[instances]]
        id = "instance1"
        endpoint = "http://localhost:50051"
        expertise = ["domain1"]
    "#;

    let config: RouterConfig = toml::from_str(toml).unwrap();
    assert_eq!(config.bind_address, "0.0.0.0");
    assert_eq!(config.bind_port, 9000);
    assert_eq!(config.jwt_secret, "my-secret-key");
    assert_eq!(config.token_expiry_secs, 7200);
    assert_eq!(config.instances.len(), 1);
}

#[test]
fn test_default_token_expiry() {
    let toml = r#"
        bind_address = "127.0.0.1"
        bind_port = 8080
        jwt_secret = "secret"
    "#;

    let config: RouterConfig = toml::from_str(toml).unwrap();
    assert_eq!(config.token_expiry_secs, 3600); // Default
}
