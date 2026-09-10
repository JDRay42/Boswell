//! Integration tests for the auth + rate-limit middleware.
//!
//! These exercise the security-critical path (bearer auth, scope enforcement,
//! rate limiting) without a live backend: a dummy protected handler returns 200
//! once the middleware admits the request, so the client is never touched.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::get;
use axum::{middleware, Extension, Router};
use http_body_util::BodyExt; // for `collect`
use tower::ServiceExt; // for `oneshot`

use boswell_gateway::auth::{auth_middleware, hash_key, AuthContext, Scope};
use boswell_gateway::config::{ApiKeyConfig, GatewayConfig, OidcConfig, OidcPrincipalConfig};
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

// ---------------------------------------------------------------------------
// OIDC (ADR-022)
//
// The middleware's job here is narrow: decide whether a token that is not a
// configured API key gets handed to the verifier at all. Verification itself is
// unit-tested in `oidc.rs` against a fixed key set. These tests point the
// verifier at a port nothing listens on, so any test that reaches a provider
// fetch fails closed rather than reaching the network.
// ---------------------------------------------------------------------------

/// A JWT-shaped bearer token. The signature is meaningless — nothing here gets
/// far enough to check one.
const JWT_SHAPED: &str = "eyJhbGciOiJSUzI1NiIsImtpZCI6ImsifQ.eyJzdWIiOiJhIn0.c2ln";

fn oidc_state() -> AppState {
    let config = GatewayConfig {
        api_keys: vec![ApiKeyConfig {
            id: "writer".into(),
            key_hash: hash_key(WRITE_KEY),
            namespace: "team".into(),
            scopes: vec!["read".into(), "write".into()],
        }],
        oidc: Some(OidcConfig {
            issuer: "https://id.example.test".into(),
            audiences: vec!["boswell".into()],
            // Refused instantly; no test may depend on reaching a provider.
            jwks_uri: "http://127.0.0.1:1/jwks".into(),
            principals: vec![OidcPrincipalConfig {
                subject: "user-abc123".into(),
                namespace: "team".into(),
                scopes: vec!["read".into(), "write".into()],
            }],
            ..OidcConfig::default()
        }),
        ..GatewayConfig::default()
    };
    AppState::from_config(&config)
}

async fn message_for(state: AppState, token: &str) -> (StatusCode, String) {
    let request = Request::builder()
        .uri("/protected")
        .header("authorization", format!("Bearer {}", token))
        .body(Body::empty())
        .unwrap();
    let response = app(state).oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    (
        status,
        json["error"].as_str().unwrap_or_default().to_string(),
    )
}

#[tokio::test]
async fn configuring_oidc_does_not_disturb_api_keys() {
    assert_eq!(
        status_for(oidc_state(), Some(WRITE_KEY)).await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn a_bad_api_key_is_not_sent_to_the_verifier() {
    // A key-shaped token has no `kid` to look up and no signature to check.
    // Answering "invalid bearer token" would mislead the one caller — an
    // operator with a stale key — most likely to see it.
    let (status, message) = message_for(oidc_state(), "not-a-real-key").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(message, "Invalid API key");
}

#[tokio::test]
async fn a_jwt_is_refused_when_the_provider_cannot_be_reached() {
    let (status, message) = message_for(oidc_state(), JWT_SHAPED).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(message, "Invalid bearer token");
}

#[tokio::test]
async fn a_jwt_is_just_a_bad_key_when_no_issuer_is_configured() {
    // Without an `[oidc]` section there is nothing to verify against, so a JWT
    // must not get a different answer from any other unrecognized token.
    let (status, message) = message_for(test_state(0), JWT_SHAPED).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(message, "Invalid API key");
}
