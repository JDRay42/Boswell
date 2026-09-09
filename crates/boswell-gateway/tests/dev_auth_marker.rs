//! The `X-Boswell-Auth` marker (design 15 §7.2).
//!
//! Every response served while the instance behind the gateway runs a
//! development identity adapter must carry a visible marker, so a calling agent
//! can see that what it is reading came from fake, preset identities.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt; // for `oneshot`

use boswell_gateway::config::GatewayConfig;
use boswell_gateway::{build_router, AppState, DEV_AUTH_HEADER, DEV_AUTH_MARKER};

fn state() -> (GatewayConfig, AppState) {
    let cfg = GatewayConfig {
        rate_limit_per_minute: 1000,
        ..GatewayConfig::default()
    };
    let state = AppState::from_config(&cfg);
    (cfg, state)
}

/// A gateway that has not heard otherwise marks nothing. It must not *claim*
/// trustworthiness either — the absence of the header says only "no dev adapter
/// reported", which is the honest default.
#[tokio::test]
async fn no_marker_before_the_instance_says_so() {
    let (cfg, state) = state();
    assert!(!state.is_dev_auth());

    let app = build_router(&cfg, state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(response.headers().get(DEV_AUTH_HEADER).is_none());
}

/// Once the instance reports devAuth, every response carries the marker.
#[tokio::test]
async fn the_marker_is_stamped_when_the_instance_reports_dev_auth() {
    let (cfg, state) = state();
    state.set_dev_auth(true);

    let app = build_router(&cfg, state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.headers().get(DEV_AUTH_HEADER).unwrap(),
        DEV_AUTH_MARKER
    );
}

/// The marker rides on rejected requests too. An unauthenticated caller learns
/// nothing about the memory, but it is still talking to a dev-trust deployment,
/// and §7.2 asks for *every* response to say so.
#[tokio::test]
async fn even_rejected_requests_carry_the_marker() {
    let (cfg, state) = state();
    state.set_dev_auth(true);

    let app = build_router(&cfg, state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/claims")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response.headers().get(DEV_AUTH_HEADER).unwrap(),
        DEV_AUTH_MARKER
    );
}

/// The flag tracks the instance rather than latching, so an instance that
/// restarts without devAuth stops being marked.
#[tokio::test]
async fn the_marker_clears_when_the_instance_stops_reporting_dev_auth() {
    let (cfg, state) = state();
    state.set_dev_auth(true);
    state.set_dev_auth(false);

    let app = build_router(&cfg, state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(response.headers().get(DEV_AUTH_HEADER).is_none());
}
