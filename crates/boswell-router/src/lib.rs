//! Boswell Router
//!
//! Session management and instance registry per ADR-019.
//! Provides topology discovery for client-side routing.

#![warn(missing_docs)]

pub mod config;
pub mod handlers;
pub mod registry;
pub mod session;

use config::RouterConfig;
use handlers::{create_router, AppState};
use registry::InstanceRegistry;
use session::SessionManager;
use std::future::Future;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::info;

/// Router error
#[derive(Debug, thiserror::Error)]
pub enum RouterError {
    /// Configuration error
    #[error("Configuration error: {0}")]
    Config(#[from] config::ConfigError),

    /// Server binding error
    #[error("Failed to bind server: {0}")]
    Bind(#[from] std::io::Error),

    /// Server error
    #[error("Server error: {0}")]
    Server(String),
}

/// Start the Router HTTP server, running until `ctrl_c`.
///
/// Initializes the registry and session manager from `config` and serves the
/// axum app. This is the entrypoint for the binary; a caller with a lifecycle of
/// its own — a supervisor, a test — wants [`start_server_with_shutdown`].
///
/// Tracing is *not* initialized here. `main` installs the subscriber, so that a
/// caller embedding the router does not have a global subscriber installed out
/// from under it.
pub async fn start_server(config: RouterConfig) -> Result<(), RouterError> {
    start_server_with_shutdown(config, ctrl_c_signal()).await
}

/// Start the Router HTTP server, shutting it down when `shutdown` resolves.
///
/// Mirrors `boswell_grpc::server::start_server_with_shutdown`: the signal is a
/// parameter rather than a hardcoded `ctrl_c`, which is what lets a test stand
/// the router up and take it down again in-process.
///
/// Shutdown is graceful in axum's sense — the listener closes, in-flight
/// requests finish, and only then does this future return `Ok`. There is no
/// bound on how long a hung handler holds shutdown open; a caller that needs a
/// deadline wraps the call in `tokio::time::timeout`.
///
/// # Errors
/// Returns an error if the bind address is unavailable or the server fails.
pub async fn start_server_with_shutdown(
    config: RouterConfig,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), RouterError> {
    info!("Starting Boswell Router");
    info!("Bind address: {}", config.bind_addr());
    info!("Token expiry: {} seconds", config.token_expiry_secs);
    info!("Registered instances: {}", config.instances.len());

    // Create session manager
    let session_manager = Arc::new(SessionManager::new(
        &config.jwt_secret,
        config.token_expiry_secs,
    ));

    // Create instance registry from config
    let registry = Arc::new(InstanceRegistry::from_config(config.instances.clone()));

    // Create application state
    let state = AppState {
        session_manager,
        registry,
    };

    // Create router
    let app = create_router(state);

    // Bind and serve
    let listener = TcpListener::bind(&config.bind_addr()).await?;
    info!("Router listening on {}", config.bind_addr());

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
        .map_err(|e| RouterError::Server(e.to_string()))?;

    info!("Router stopped");

    Ok(())
}

/// The default shutdown signal: `ctrl_c`, matching the gRPC instance.
///
/// A failure to *install* the handler resolves the future rather than
/// propagating, which would shut the router down at startup. That case is
/// vanishingly rare, so it is reported rather than swallowed; the alternative —
/// pending forever — would leave a router that cannot be stopped by the one
/// signal an operator will try.
async fn ctrl_c_signal() {
    if let Err(e) = tokio::signal::ctrl_c().await {
        eprintln!("Failed to listen for shutdown signal, stopping: {e}");
        return;
    }
    info!("Shutdown signal received, stopping Boswell Router");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_router_config() {
        let config = RouterConfig::default_test_config();
        assert_eq!(config.instances.len(), 1);
        assert_eq!(config.token_expiry_secs, 3600);
    }
}
