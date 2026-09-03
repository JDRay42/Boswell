#![warn(missing_docs)]

//! Boswell public HTTP/JSON API gateway.
//!
//! A thin, authenticated HTTP front end over the in-repo
//! [`BoswellClient`](boswell_sdk::BoswellClient). It exposes the full memory
//! lifecycle under `/v1` so external (e.g. cloud) agents can use Boswell over
//! HTTPS via a reverse proxy or tunnel, while the gRPC instance stays private.
//!
//! Auth is static bearer API keys, each mapped to a namespace and scopes; keys
//! are stored as SHA-256 hashes in the gateway config.

pub mod auth;
pub mod config;
pub mod error;
pub mod handlers;
pub mod state;

use std::time::Duration;

use axum::http::StatusCode;
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

/// Build the axum application (routes + middleware) for the given config/state.
pub fn build_router(config: &GatewayConfig, state: AppState) -> Router {
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
        .route("/v1/search", post(handlers::search))
        .route("/v1/recall", post(handlers::recall))
        .route("/v1/extract", post(handlers::extract))
        .route("/v1/hooks/ingest", post(handlers::hooks_ingest))
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
        if let Err(e) = client.ensure_connected().await {
            tracing::warn!(
                "gateway: initial connect to {} failed ({}); will retry on demand",
                config.router_endpoint,
                e
            );
        }
    }

    let app = build_router(&config, state);
    let addr = config.bind_addr();

    tracing::info!(
        "boswell-gateway listening on {} ({} API key(s) loaded)",
        addr,
        config.api_keys.len()
    );

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| GatewayError::Serve(format!("failed to bind {}: {}", addr, e)))?;

    axum::serve(listener, app.into_make_service())
        .await
        .map_err(|e| GatewayError::Serve(e.to_string()))?;

    Ok(())
}
