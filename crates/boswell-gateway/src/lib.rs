#![warn(missing_docs)]

//! Boswell public HTTP/JSON API gateway.
//!
//! A thin, authenticated HTTP front end over the in-repo
//! [`BoswellClient`](boswell_sdk::BoswellClient). It exposes the full memory
//! lifecycle under `/v1` so external (e.g. cloud) agents can use Boswell over
//! HTTPS via a reverse proxy or tunnel, while the gRPC instance stays private.
//!
//! Auth is bearer tokens on the `/v1` surface. A token is either a static API
//! key, stored as a SHA-256 hash in the gateway config and mapped to a
//! namespace and scopes, or — where an `[oidc]` section names an identity
//! provider — a JWT from that provider, verified against locally cached JWKS.
//! Both paths end in the same [`AuthContext`](auth::AuthContext); see
//! [`oidc`] for what verification does and does not grant.
//!
//! Where a `[tokens]` section is configured there is a third: an **attenuable
//! token** minted by `POST /v1/tokens` from either of the other two, which its
//! holder narrows offline for subagents. It ends in the same `AuthContext` as
//! well; see [`tokens`].

pub mod auth;
pub mod config;
pub mod error;
pub mod handlers;
pub mod metrics;
pub mod oidc;
pub mod state;
pub mod tokens;

use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderValue, Request, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use axum::routing::{get, post};
use axum::{middleware, Router};
use thiserror::Error;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

pub use config::{ConfigError, GatewayConfig};
pub use state::AppState;

/// Errors that can occur while starting or running the gateway.
#[derive(Debug, Error)]
pub enum GatewayError {
    /// Configuration could not be loaded.
    #[error("Configuration error: {0}")]
    Config(#[from] ConfigError),

    /// The HTTP server failed to bind or serve.
    #[error("Serve error: {0}")]
    Serve(String),
}

/// The `X-Boswell-Auth` header name (design §7.2).
pub const DEV_AUTH_HEADER: &str = "x-boswell-auth";

/// The marker value identifying a response served under a development identity
/// adapter. Mirrors `boswell_devauth::DEV_AUTH_MARKER`, restated here so no
/// production crate has to depend on the development one.
pub const DEV_AUTH_MARKER: &str = "dev-untrusted";

/// Stamp `X-Boswell-Auth: dev-untrusted` on every response served while the
/// instance behind us runs a development identity adapter (design §7.2).
///
/// Applied to the whole router, authenticated and public alike: a caller reading
/// `/v1/health` deserves the warning as much as one reading a claim. The marker
/// is derived from what the instance reports about itself, never from gateway
/// config, so it cannot drift out of sync with reality by an operator forgetting
/// to set it.
async fn dev_auth_marker(
    State(state): State<AppState>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let mut response = next.run(request).await;
    if state.is_dev_auth() {
        response
            .headers_mut()
            .insert(DEV_AUTH_HEADER, HeaderValue::from_static(DEV_AUTH_MARKER));
    }
    response
}

/// Build the axum application (routes + middleware) for the given config/state.
pub fn build_router(config: &GatewayConfig, state: AppState) -> Router {
    let state_for_marker = state.clone();

    // Authenticated `/v1` surface.
    let protected = Router::new()
        .route(
            "/v1/claims",
            post(handlers::assert_claim).get(handlers::query_claims),
        )
        .route("/v1/claims/batch", post(handlers::batch_learn))
        .route(
            "/v1/claims/:id",
            get(handlers::get_claim).delete(handlers::delete_claim),
        )
        .route(
            "/v1/claims/:id/relationships",
            get(handlers::get_relationships),
        )
        .route("/v1/tokens", post(handlers::mint_token))
        .route("/v1/search", post(handlers::search))
        .route("/v1/recall", post(handlers::recall))
        .route("/v1/extract", post(handlers::extract))
        .route("/v1/hooks/ingest", post(handlers::hooks_ingest))
        .route("/v1/goals", get(handlers::query_goals))
        .route("/v1/goals/:id", get(handlers::get_goal))
        .route("/v1/goals/:id/expand", get(handlers::expand_goal))
        .route("/v1/procedures", get(handlers::query_procedures))
        .route("/v1/procedures/:id", get(handlers::get_procedure))
        .route(
            "/v1/receipts/:receipt_id/report",
            post(handlers::report_outcome),
        )
        // Unversioned on purpose: Prometheus scrape configs default to
        // `/metrics`, and the exposition format carries its own version.
        .route("/metrics", get(metrics::metrics))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth::auth_middleware,
        ))
        .with_state(state.clone());

    // Unauthenticated liveness.
    let public = Router::new()
        .route("/v1/health", get(handlers::health))
        .with_state(state);

    public
        .merge(protected)
        .layer(middleware::from_fn_with_state(
            state_for_marker,
            dev_auth_marker,
        ))
        .layer(TraceLayer::new_for_http())
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(config.request_timeout_secs),
        ))
        .layer(RequestBodyLimitLayer::new(config.max_body_bytes))
}

/// Build state, connect (best effort), and serve until shut down.
pub async fn run(config: GatewayConfig) -> Result<(), GatewayError> {
    let state = AppState::from_config(&config);

    // Best-effort connect at startup; the SDK reconnects on demand otherwise.
    {
        let mut client = state.client().lock().await;
        match client.ensure_connected().await {
            // Ask once at startup so the very first response already carries the
            // marker; `/v1/health` refreshes it thereafter.
            Ok(()) => match client.health().await {
                Ok(h) => state.set_dev_auth(h.dev_auth),
                Err(e) => tracing::warn!("gateway: initial health check failed ({})", e),
            },
            Err(e) => tracing::warn!(
                "gateway: initial connect to {} failed ({}); will retry on demand",
                config.router_endpoint,
                e
            ),
        }
    }

    let app = build_router(&config, state);
    let addr = config.bind_addr();

    tracing::info!(
        "boswell-gateway listening on {} ({} API key(s) loaded, {})",
        addr,
        config.api_keys.len(),
        match &config.oidc {
            Some(oidc) => format!(
                "OIDC issuer {} with {} principal(s)",
                oidc.issuer,
                oidc.principals.len()
            ),
            None => "no OIDC issuer".to_string(),
        }
    );

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| GatewayError::Serve(format!("failed to bind {}: {}", addr, e)))?;

    axum::serve(listener, app.into_make_service())
        .await
        .map_err(|e| GatewayError::Serve(e.to_string()))?;

    Ok(())
}
