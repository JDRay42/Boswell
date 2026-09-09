//! Route + scope tests for the goal-traversal endpoints.
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
const WRITE_ONLY_KEY: &str = "write-only-key";

const GOAL_ID: &str = "01890000-0000-7000-8000-000000000000";

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

async fn status_for(request: Request<Body>) -> StatusCode {
    let cfg = config();
    let app = build_router(&cfg, AppState::from_config(&cfg));
    app.oneshot(request).await.unwrap().status()
}

fn get(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

fn get_as(uri: &str, key: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .header("authorization", format!("Bearer {}", key))
        .body(Body::empty())
        .unwrap()
}

/// Traversal is authenticated like everything else under `/v1`.
#[tokio::test]
async fn querying_goals_requires_a_key() {
    assert_eq!(
        status_for(get("/v1/goals?intent_contains=breakfast")).await,
        StatusCode::UNAUTHORIZED
    );
}

/// All three traversal routes are mounted (an unauthenticated call is refused
/// for auth, not because the path is missing).
#[tokio::test]
async fn the_traversal_routes_are_mounted() {
    for uri in [
        "/v1/goals",
        &format!("/v1/goals/{}", GOAL_ID),
        &format!("/v1/goals/{}/expand", GOAL_ID),
    ] {
        assert_eq!(
            status_for(get(uri)).await,
            StatusCode::UNAUTHORIZED,
            "{} should be mounted",
            uri
        );
    }
}

/// Expanding a goal is a read of memory, so a write-only key is refused. Unlike
/// procedure retrieval it issues no receipt, but the surface it returns still
/// exposes a decomposition's child ids, usage notes, and the claim readings
/// behind its preconditions.
#[tokio::test]
async fn expanding_requires_the_read_scope() {
    let uri = format!("/v1/goals/{}/expand", GOAL_ID);
    assert_eq!(
        status_for(get_as(&uri, WRITE_ONLY_KEY)).await,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn fetching_a_goal_requires_the_read_scope() {
    let uri = format!("/v1/goals/{}", GOAL_ID);
    assert_eq!(
        status_for(get_as(&uri, WRITE_ONLY_KEY)).await,
        StatusCode::FORBIDDEN
    );
}

/// A key confined to `team` cannot ask for another namespace's goals: the
/// handler refuses before the request reaches the instance.
#[tokio::test]
async fn a_scoped_key_cannot_query_another_namespace() {
    assert_eq!(
        status_for(get_as("/v1/goals?namespace=someone-else", READ_KEY)).await,
        StatusCode::FORBIDDEN
    );
}
