//! Route + validation tests for `GET /v1/feed`. These stop at the handler, so
//! no live backend is needed.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt; // for `oneshot`

use boswell_gateway::auth::hash_key;
use boswell_gateway::config::{ApiKeyConfig, GatewayConfig};
use boswell_gateway::{build_router, AppState};

const READ_KEY: &str = "read-key";
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

async fn get(uri: &str, key: Option<&str>) -> StatusCode {
    let cfg = config();
    let app = build_router(&cfg, AppState::from_config(&cfg));
    let mut builder = Request::builder().uri(uri);
    if let Some(key) = key {
        builder = builder.header("authorization", format!("Bearer {}", key));
    }
    app.oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn the_feed_is_mounted_and_authenticated() {
    assert_eq!(get("/v1/feed", None).await, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn the_feed_requires_the_read_scope() {
    assert_eq!(
        get("/v1/feed", Some(WRITE_ONLY_KEY)).await,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn a_scoped_key_cannot_follow_another_namespace() {
    assert_eq!(
        get("/v1/feed?namespace=someone-else", Some(READ_KEY)).await,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn a_malformed_cursor_is_refused() {
    assert_eq!(
        get("/v1/feed?since=yesterday", Some(READ_KEY)).await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        get("/v1/feed?from=tail", Some(READ_KEY)).await,
        StatusCode::BAD_REQUEST
    );
}
