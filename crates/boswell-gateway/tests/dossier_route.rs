//! Route + validation tests for `POST /v1/dossier`. These stop at the handler,
//! so no live backend is needed: a refused request never reaches the SDK client.

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

async fn post(key: Option<&str>, body: &str) -> StatusCode {
    let cfg = config();
    let app = build_router(&cfg, AppState::from_config(&cfg));
    let mut builder = Request::builder()
        .method("POST")
        .uri("/v1/dossier")
        .header("content-type", "application/json");
    if let Some(key) = key {
        builder = builder.header("authorization", format!("Bearer {}", key));
    }
    app.oneshot(builder.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn the_route_is_mounted_and_authenticated() {
    assert_eq!(
        post(None, r#"{"subject":"X"}"#).await,
        StatusCode::UNAUTHORIZED
    );
}

/// A dossier is a read of memory, so a write-only key is refused.
#[tokio::test]
async fn a_dossier_requires_the_read_scope() {
    assert_eq!(
        post(Some(WRITE_ONLY_KEY), r#"{"subject":"X"}"#).await,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn a_scoped_key_cannot_read_another_namespace() {
    assert_eq!(
        post(
            Some(READ_KEY),
            r#"{"subject":"X","namespace":"someone-else"}"#
        )
        .await,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn a_dossier_needs_an_anchor() {
    assert_eq!(
        post(Some(READ_KEY), r#"{"subject":"  "}"#).await,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn an_unknown_view_is_refused() {
    assert_eq!(
        post(Some(READ_KEY), r#"{"subject":"X","view":"summary"}"#).await,
        StatusCode::BAD_REQUEST
    );
}
