//! Integration tests for attenuable tokens on the wire (ADR-022).
//!
//! The unit tests in `tokens.rs` prove the Datalog. These prove the two things
//! only the assembled gateway can show: that a minted token is accepted by the
//! same middleware an API key goes through, and that an attenuated one is
//! refused by the handler rather than merely by a library call.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::get;
use axum::{middleware, Extension, Router};
use http_body_util::BodyExt; // for `collect`
use tower::ServiceExt; // for `oneshot`

use boswell_gateway::auth::{auth_middleware, hash_key, AuthContext, Scope};
use boswell_gateway::config::{ApiKeyConfig, GatewayConfig, TokenConfig};
use boswell_gateway::error::ApiError;
use boswell_gateway::tokens::{attenuate, Attenuation};
use boswell_gateway::AppState;

const WRITE_KEY: &str = "read-write-key";

/// A dummy protected handler standing in for a write route: it asks for the
/// write scope and for a namespace, which is exactly the pair a token can be
/// attenuated on.
async fn writes_to_team_sub(
    Extension(ctx): Extension<AuthContext>,
) -> Result<&'static str, ApiError> {
    ctx.require(Scope::Write)?;
    ctx.require_namespace("team:sub")?;
    Ok("ok")
}

async fn reads(Extension(ctx): Extension<AuthContext>) -> Result<&'static str, ApiError> {
    ctx.require(Scope::Read)?;
    Ok("ok")
}

fn app(state: AppState) -> Router {
    Router::new()
        .route("/write", get(writes_to_team_sub))
        .route("/read", get(reads))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        .with_state(state)
}

/// A gateway with one read+write API key and a fresh token root key.
fn test_state() -> AppState {
    let keypair = biscuit_auth::KeyPair::new_with_algorithm(biscuit_auth::Algorithm::Ed25519);
    AppState::from_config(&GatewayConfig {
        api_keys: vec![ApiKeyConfig {
            id: "writer".into(),
            key_hash: hash_key(WRITE_KEY),
            namespace: "team".into(),
            scopes: vec!["read".into(), "write".into()],
        }],
        tokens: Some(TokenConfig {
            root_private_key: keypair.private().to_bytes_hex(),
            default_ttl_secs: 3600,
            max_ttl_secs: 86400,
        }),
        ..GatewayConfig::default()
    })
}

async fn status_for(state: &AppState, path: &str, bearer: &str) -> StatusCode {
    let request = Request::builder()
        .uri(path)
        .header("authorization", format!("Bearer {}", bearer))
        .body(Body::empty())
        .unwrap();
    app(state.clone()).oneshot(request).await.unwrap().status()
}

/// Mint a root token the way `POST /v1/tokens` would, for the API key's grant.
fn root_token(state: &AppState) -> String {
    let authority = state.tokens().expect("tokens configured");
    let ctx = state
        .lookup_key(&hash_key(WRITE_KEY))
        .expect("key configured");
    authority.mint(&ctx, None).expect("mint").token
}

fn public_key(state: &AppState) -> biscuit_auth::PublicKey {
    biscuit_auth::PublicKey::from_bytes_hex(
        &state.tokens().unwrap().root_public_key_hex(),
        biscuit_auth::Algorithm::Ed25519,
    )
    .expect("public key")
}

#[tokio::test]
async fn a_root_token_is_accepted_wherever_the_key_it_came_from_is() {
    let state = test_state();
    let token = root_token(&state);
    assert_eq!(status_for(&state, "/write", &token).await, StatusCode::OK);
}

#[tokio::test]
async fn a_token_narrowed_to_read_is_refused_a_write() {
    let state = test_state();
    let narrowed = attenuate(
        &root_token(&state),
        public_key(&state),
        &Attenuation {
            scopes: Some(vec![Scope::Read]),
            ..Default::default()
        },
    )
    .expect("attenuate");

    assert_eq!(status_for(&state, "/read", &narrowed).await, StatusCode::OK);
    assert_eq!(
        status_for(&state, "/write", &narrowed).await,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn a_token_narrowed_to_a_sibling_namespace_is_refused() {
    let state = test_state();
    let narrowed = attenuate(
        &root_token(&state),
        public_key(&state),
        &Attenuation {
            namespace: Some("team:other".to_string()),
            ..Default::default()
        },
    )
    .expect("attenuate");

    // The scope survives; only the namespace check fails, which is what tells
    // us the two dimensions are evaluated independently.
    assert_eq!(status_for(&state, "/read", &narrowed).await, StatusCode::OK);
    assert_eq!(
        status_for(&state, "/write", &narrowed).await,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn a_refusal_names_the_dimension_and_not_the_datalog() {
    let state = test_state();
    let narrowed = attenuate(
        &root_token(&state),
        public_key(&state),
        &Attenuation {
            scopes: Some(vec![Scope::Read]),
            ..Default::default()
        },
    )
    .expect("attenuate");

    let request = Request::builder()
        .uri("/write")
        .header("authorization", format!("Bearer {}", narrowed))
        .body(Body::empty())
        .unwrap();
    let response = app(state).oneshot(request).await.unwrap();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let body = String::from_utf8(body.to_vec()).unwrap();

    assert!(body.contains("write"), "got {}", body);
    assert!(
        !body.contains("check") && !body.contains("reject if"),
        "a Datalog failure is not something a caller can act on: {}",
        body
    );
}

#[tokio::test]
async fn a_gateway_with_no_tokens_section_rejects_a_token_as_a_bad_key() {
    // Minted by a gateway that does issue them; presented to one that does not.
    let issuer = test_state();
    let token = root_token(&issuer);

    let bare = AppState::from_config(&GatewayConfig {
        api_keys: vec![ApiKeyConfig {
            id: "writer".into(),
            key_hash: hash_key(WRITE_KEY),
            namespace: "team".into(),
            scopes: vec!["read".into(), "write".into()],
        }],
        ..GatewayConfig::default()
    });

    assert_eq!(
        status_for(&bare, "/read", &token).await,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn a_token_from_another_gateways_root_key_is_401_not_403() {
    // 401: nothing about this token was established. A 403 would say the
    // signature was good and only the authority was short, which is a
    // disclosure and is also false.
    let theirs = test_state();
    let ours = test_state();
    assert_eq!(
        status_for(&ours, "/read", &root_token(&theirs)).await,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn an_api_key_still_reads_as_a_bad_key_when_tokens_are_configured() {
    // The regression #68 guarded against, in its token-shaped form: adding a
    // second credential type must not turn a stale API key into a confusing
    // token error.
    let state = test_state();
    let request = Request::builder()
        .uri("/read")
        .header("authorization", "Bearer stale-key")
        .body(Body::empty())
        .unwrap();
    let response = app(state).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let body = String::from_utf8(body.to_vec()).unwrap();
    assert!(body.contains("Invalid API key"), "got {}", body);
}
