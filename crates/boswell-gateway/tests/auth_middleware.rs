//! Integration tests for the auth + rate-limit middleware.
//!
//! These exercise the security-critical path (bearer auth, scope enforcement,
//! rate limiting) without a live backend: a dummy protected handler returns 200
//! once the middleware admits the request, so the client is never touched.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::get;
use axum::{middleware, Extension, Router};
use tower::ServiceExt; // for `oneshot`

use boswell_gateway::auth::{auth_middleware, hash_key, AuthContext, Scope};
use boswell_gateway::config::{ApiKeyConfig, GatewayConfig};
use boswell_gateway::error::ApiError;
use boswell_gateway::AppState;

async fn needs_write(Extension(ctx): Extension<AuthContext>) -> Result<&'static str, ApiError> {
    ctx.require(Scope::Write)?;
    Ok("ok")
}

fn app(state: AppState) -> Router {
    Router::new()
        .route("/protected", get(needs_write))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        .with_state(state)
}

const READ_KEY: &str = "read-only-key";
const WRITE_KEY: &str = "read-write-key";

fn test_state(rate: u32) -> AppState {
    let config = GatewayConfig {
        rate_limit_per_minute: rate,
        api_keys: vec![
            ApiKeyConfig {
                id: "reader".into(),
                key_hash: hash_key(READ_KEY),
                namespace: "team".into(),
                scopes: vec!["read".into()],
            },
            ApiKeyConfig {
                id: "writer".into(),
                key_hash: hash_key(WRITE_KEY),
                namespace: "team".into(),
                scopes: vec!["read".into(), "write".into()],
            },
        ],
        ..GatewayConfig::default()
    };
    AppState::from_config(&config)
}

async fn status_for(state: AppState, auth: Option<&str>) -> StatusCode {
    let mut builder = Request::builder().uri("/protected");
    if let Some(token) = auth {
        builder = builder.header("authorization", format!("Bearer {}", token));
    }
    let request = builder.body(Body::empty()).unwrap();
    app(state).oneshot(request).await.unwrap().status()
}

#[tokio::test]
async fn missing_header_is_401() {
    assert_eq!(
        status_for(test_state(0), None).await,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn invalid_key_is_401() {
    assert_eq!(
        status_for(test_state(0), Some("not-a-real-key")).await,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn out_of_scope_key_is_403() {
    // The read-only key authenticates but lacks the `write` scope.
    assert_eq!(
        status_for(test_state(0), Some(READ_KEY)).await,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn valid_scoped_key_is_200() {
    assert_eq!(
        status_for(test_state(0), Some(WRITE_KEY)).await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn rate_limit_blocks_after_capacity() {
    let state = test_state(1); // one request per minute
    assert_eq!(
        status_for(state.clone(), Some(WRITE_KEY)).await,
        StatusCode::OK
    );
    // The bucket is now empty; the next immediate request is rejected.
    assert_eq!(
        status_for(state, Some(WRITE_KEY)).await,
        StatusCode::TOO_MANY_REQUESTS
    );
}
