//! Route + scope tests for the procedural-memory endpoints.
//!
//! These stop at the middleware and the handler's scope check, so no live
//! backend is needed: a request that is rejected for auth or scope never
//! reaches the SDK client.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt; // for `oneshot`

use boswell_gateway::auth::hash_key;
use boswell_gateway::config::{ApiKeyConfig, GatewayConfig};
use boswell_gateway::{build_router, AppState};

const READ_KEY: &str = "read-only-key";

fn config() -> GatewayConfig {
    GatewayConfig {
        rate_limit_per_minute: 1000,
        api_keys: vec![ApiKeyConfig {
            id: "reader".into(),
            key_hash: hash_key(READ_KEY),
            namespace: "team".into(),
            scopes: vec!["read".into()],
        }],
        ..GatewayConfig::default()
    }
}

async fn status_for(request: Request<Body>) -> StatusCode {
    let cfg = config();
    let app = build_router(&cfg, AppState::from_config(&cfg));
    app.oneshot(request).await.unwrap().status()
}

/// Procedure retrieval is authenticated like everything else under `/v1`.
#[tokio::test]
async fn querying_procedures_requires_a_key() {
    let request = Request::builder()
        .uri("/v1/procedures?goal=goal:team/deploy")
        .body(Body::empty())
        .unwrap();

    assert_eq!(status_for(request).await, StatusCode::UNAUTHORIZED);
}

/// Reporting an outcome moves a procedure's effectiveness counters, so it takes
/// the `write` scope — a read-only key is refused before the request ever
/// reaches the instance.
#[tokio::test]
async fn reporting_an_outcome_requires_the_write_scope() {
    let request = Request::builder()
        .method("POST")
        .uri("/v1/receipts/01890000-0000-7000-8000-000000000000/report")
        .header("authorization", format!("Bearer {}", READ_KEY))
        .header("content-type", "application/json")
        .body(Body::from(r#"{"outcome":"success"}"#))
        .unwrap();

    assert_eq!(status_for(request).await, StatusCode::FORBIDDEN);
}

/// The report route is mounted (an unauthenticated call is refused for auth,
/// not because the path is missing).
#[tokio::test]
async fn the_report_route_is_mounted() {
    let request = Request::builder()
        .method("POST")
        .uri("/v1/receipts/01890000-0000-7000-8000-000000000000/report")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"outcome":"success"}"#))
        .unwrap();

    assert_eq!(status_for(request).await, StatusCode::UNAUTHORIZED);
}
