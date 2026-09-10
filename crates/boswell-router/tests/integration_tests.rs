//! Integration tests for the Router service

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use boswell_router::{
    config::{InstanceConfig, RouterConfig},
    handlers::{create_router, AppState, HealthCheckResponse},
    registry::InstanceRegistry,
    session::{SessionManager, SessionResponse},
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

// ============================================================================
// Server lifecycle
//
// The regression these exist for: `start_server` used to call `axum::serve`
// with no shutdown signal, so it never returned and the only way to stop the
// router was to kill the process. That is what kept the SDK's full-stack tests
// `#[ignore]`d — nothing could stand a router up and take it down again.
// ============================================================================

/// Claim an ephemeral port and release it so the router can bind it.
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("the loopback interface should hand out an ephemeral port")
        .local_addr()
        .expect("a bound listener has a local address")
        .port()
}

fn test_config(port: u16) -> RouterConfig {
    RouterConfig {
        bind_address: "127.0.0.1".to_string(),
        bind_port: port,
        jwt_secret: "test-secret-key".to_string(),
        token_expiry_secs: 3600,
        instances: vec![InstanceConfig {
            id: "instance1".to_string(),
            endpoint: "http://localhost:50051".to_string(),
            expertise: vec!["*".to_string()],
        }],
    }
}

/// The `timeout` is the assertion, not a performance guard: a router that
/// ignores its shutdown signal hangs here rather than failing.
#[tokio::test]
async fn the_router_returns_when_the_shutdown_signal_fires_while_it_is_serving() {
    let port = free_port();
    let (tx, rx) = tokio::sync::oneshot::channel();

    // Signal only once the router is actually accepting, so a pass means it
    // served *and* stopped, not that it never started.
    tokio::spawn(async move {
        for _ in 0..200 {
            if tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .is_ok()
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let _ = tx.send(());
    });

    let served = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        boswell_router::start_server_with_shutdown(test_config(port), async move {
            let _ = rx.await;
        }),
    )
    .await
    .expect("the router should stop on the signal, not run until the test times out");

    served.expect("a graceful shutdown is not an error");
}

/// A signal that has already fired must still leave the router bound cleanly
/// first — shutdown is not an error path.
#[tokio::test]
async fn a_shutdown_signal_that_has_already_fired_stops_the_router_cleanly() {
    let served = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        boswell_router::start_server_with_shutdown(
            test_config(free_port()),
            std::future::ready(()),
        ),
    )
    .await
    .expect("an already-fired signal should stop the router at once");

    served.expect("a graceful shutdown is not an error");
}
