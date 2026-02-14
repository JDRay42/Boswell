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

/// Start the Router HTTP server
///
/// Loads configuration, initializes registry and session manager,
/// and starts the axum server.
pub async fn start_server(config: RouterConfig) -> Result<(), RouterError> {
    // Initialize tracing
    tracing_subscriber::fmt::init();

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
        .await
        .map_err(|e| RouterError::Server(e.to_string()))?;

    Ok(())
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

