//! Route, scope and rendering tests for `GET /metrics`.
//!
//! No live instance is stood up. Two of these stop at the middleware, and the
//! third deliberately runs against an unreachable instance — that is the case
//! the `boswell_instance_up` gauge exists for.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt; // for `oneshot`

use boswell_gateway::auth::hash_key;
use boswell_gateway::config::{ApiKeyConfig, GatewayConfig};
use boswell_gateway::{build_router, AppState};

const READ_KEY: &str = "read-only-key";
const WRITE_ONLY_KEY: &str = "write-only-key";

fn config() -> GatewayConfig {
    GatewayConfig {
        rate_limit_per_minute: 1000,
        api_keys: vec![
            ApiKeyConfig {
                id: "reader".into(),
                key_hash: hash_key(READ_KEY),
                namespace: "team".into(),
                scopes: vec!["read".into()],
            },
            ApiKeyConfig {
                id: "writer".into(),
                key_hash: hash_key(WRITE_ONLY_KEY),
                namespace: "team".into(),
                scopes: vec!["write".into()],
            },
        ],
        ..GatewayConfig::default()
    }
}

fn request(key: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().uri("/metrics");
    if let Some(key) = key {
        builder = builder.header("authorization", format!("Bearer {}", key));
    }
    builder.body(Body::empty()).unwrap()
}

async fn respond(key: Option<&str>) -> (StatusCode, String, String) {
    let cfg = config();
    let app = build_router(&cfg, AppState::from_config(&cfg));
    let response = app.oneshot(request(key)).await.unwrap();
    let status = response.status();
    let content_type = response
        .headers()
        .get("content-type")
        .map(|v| v.to_str().unwrap().to_string())
        .unwrap_or_default();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        content_type,
        String::from_utf8(body.to_vec()).unwrap(),
    )
}

/// The scrape endpoint is authenticated like the rest of the gateway. Left
/// public it would tell an unauthenticated caller how much memory the
/// deployment holds and how fast it decays.
#[tokio::test]
async fn metrics_requires_a_key() {
    let (status, _, _) = respond(None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

/// A key that can write but not read is not a key that may scrape.
#[tokio::test]
async fn metrics_requires_the_read_scope() {
    let (status, _, _) = respond(Some(WRITE_ONLY_KEY)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

/// An unreachable instance is a *successful* scrape reporting `up 0`. Failing
/// the request instead would be indistinguishable, to Prometheus, from a
/// gateway it could not reach at all.
#[tokio::test]
async fn an_unreachable_instance_still_scrapes_clean() {
    let (status, content_type, body) = respond(Some(READ_KEY)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(content_type, "text/plain; version=0.0.4; charset=utf-8");
    assert!(
        body.contains("boswell_instance_up 0\n"),
        "body was: {}",
        body
    );
}
