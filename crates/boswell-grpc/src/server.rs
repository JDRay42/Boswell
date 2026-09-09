//! gRPC server configuration and lifecycle management
//!
//! Handles server initialization and binding. Note what it does *not* do:
//! there is no TLS termination here (see [`ServerConfig::enable_tls`]) and no
//! graceful shutdown — the server runs until the process is killed.

use boswell_domain::traits::{ClaimStore, GoalStore, ProcedureStore};
use boswell_domain::IdentityProvider;
use std::sync::{Arc, Mutex};
use tonic::transport::Server;

use crate::proto::bos_well_service_server::BosWellServiceServer;
use crate::service::{BosWellServiceImpl, ServerExtractor};

/// Why the server refuses to start when `enable_tls` is set.
const TLS_NOT_IMPLEMENTED: &str = "enable_tls is set, but this server does not implement TLS. \
Terminate TLS at a reverse proxy or tunnel in front of the instance, and unset enable_tls.";

/// Server configuration
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Server listen address
    pub addr: String,

    /// Server port
    pub port: u16,

    /// Request TLS (per ADR-017).
    ///
    /// **TLS is not implemented at this layer, and setting this refuses to
    /// start.** Terminate TLS at a reverse proxy or tunnel in front of the
    /// instance, as `docs/development/gateway-plan.md` decided and the README
    /// documents.
    ///
    /// The flag is kept, rather than deleted, so that an operator who believes
    /// they configured TLS gets an error instead of silently getting plaintext.
    /// Earlier this printed "TLS enabled (certificate validation deferred)" and
    /// then served cleartext, which misreported the security posture to exactly
    /// the person who had tried to secure it.
    pub enable_tls: bool,

    /// TLS certificate path. Accepted, never read — see [`Self::enable_tls`].
    pub tls_cert_path: Option<String>,

    /// TLS key path. Accepted, never read — see [`Self::enable_tls`].
    pub tls_key_path: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            addr: "127.0.0.1".to_string(),
            port: 50051,
            enable_tls: false,
            tls_cert_path: None,
            tls_key_path: None,
        }
    }
}

impl ServerConfig {
    /// Create a new server configuration
    pub fn new(addr: impl Into<String>, port: u16) -> Self {
        Self {
            addr: addr.into(),
            port,
            ..Default::default()
        }
    }

    /// Request TLS with certificate paths.
    ///
    /// A config built this way **will not start** — see [`Self::enable_tls`].
    pub fn with_tls(mut self, cert_path: impl Into<String>, key_path: impl Into<String>) -> Self {
        self.enable_tls = true;
        self.tls_cert_path = Some(cert_path.into());
        self.tls_key_path = Some(key_path.into());
        self
    }

    /// Get the full server address
    pub fn full_address(&self) -> String {
        format!("{}:{}", self.addr, self.port)
    }

    /// Refuse configurations the server cannot honour.
    ///
    /// Today that is exactly one: [`Self::enable_tls`]. The check lives here,
    /// separate from binding, so it can be exercised without standing a server
    /// up — and so it runs *before* the socket is opened.
    ///
    /// # Errors
    /// Returns an error if `enable_tls` is set, since TLS is not implemented
    /// at this layer.
    pub fn ensure_startable(&self) -> Result<(), Box<dyn std::error::Error>> {
        if self.enable_tls {
            return Err(TLS_NOT_IMPLEMENTED.into());
        }
        Ok(())
    }
}

/// Start the gRPC server.
///
/// The `Extract` RPC returns `FailedPrecondition` under this entrypoint; use
/// [`start_server_with_extractor`] to enable server-side LLM extraction.
///
/// # Errors
/// Returns error if server fails to start or bind to address
pub async fn start_server<S>(
    config: ServerConfig,
    store: Arc<Mutex<S>>,
) -> Result<(), Box<dyn std::error::Error>>
where
    // See `BosWellServiceImpl`: `Send` suffices because access is via `Arc<Mutex<S>>`.
    S: ClaimStore + GoalStore + ProcedureStore + Send + 'static,
    S::Error: std::fmt::Debug,
{
    start_server_with_extractor(config, store, None).await
}

/// Start the gRPC server, optionally attaching a server-side [`ServerExtractor`]
/// that backs the `Extract` RPC (and LLM-mode hook ingest).
///
/// # Errors
/// Returns error if server fails to start or bind to address
pub async fn start_server_with_extractor<S>(
    config: ServerConfig,
    store: Arc<Mutex<S>>,
    extractor: Option<Arc<dyn ServerExtractor>>,
) -> Result<(), Box<dyn std::error::Error>>
where
    // See `BosWellServiceImpl`: `Send` suffices because access is via `Arc<Mutex<S>>`.
    S: ClaimStore + GoalStore + ProcedureStore + Send + 'static,
    S::Error: std::fmt::Debug,
{
    start_server_with_identity(config, store, extractor, None).await
}

/// Start the gRPC server with an optional extractor **and** an optional
/// [`IdentityProvider`] (design §6).
///
/// The identity port is taken as a trait object, never a concrete adapter, so
/// this crate — and every other crate on the production path — stays ignorant of
/// which provider is in play. Passing `None` leaves every self-report stamped
/// `Assurance::None`.
pub async fn start_server_with_identity<S>(
    config: ServerConfig,
    store: Arc<Mutex<S>>,
    extractor: Option<Arc<dyn ServerExtractor>>,
    identity: Option<Arc<dyn IdentityProvider + Send + Sync>>,
) -> Result<(), Box<dyn std::error::Error>>
where
    S: ClaimStore + GoalStore + ProcedureStore + Send + 'static,
    S::Error: std::fmt::Debug,
{
    // Refuse before binding: an operator who set `enable_tls` must not end up
    // serving plaintext under the impression they are serving TLS.
    config.ensure_startable()?;

    let addr = config.full_address().parse()?;

    let mut service = BosWellServiceImpl::new(store);
    if let Some(extractor) = extractor {
        service = service.with_extractor(extractor);
    }
    if let Some(identity) = identity {
        service = service.with_identity_provider(identity);
    }
    let service_server = BosWellServiceServer::new(service);

    println!("BosWell gRPC server starting on {}", addr);

    Server::builder()
        .add_service(service_server)
        .serve(addr)
        .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = ServerConfig::default();
        assert_eq!(config.addr, "127.0.0.1");
        assert_eq!(config.port, 50051);
        assert!(!config.enable_tls);
    }

    #[test]
    fn test_config_with_tls() {
        let config = ServerConfig::new("0.0.0.0", 50052).with_tls("cert.pem", "key.pem");

        assert!(config.enable_tls);
        assert_eq!(config.tls_cert_path, Some("cert.pem".to_string()));
        assert_eq!(config.tls_key_path, Some("key.pem".to_string()));
    }

    /// The config above parses happily; starting with it must not. This is the
    /// half that was missing — `test_config_with_tls` passed while the server
    /// served plaintext.
    #[test]
    fn requesting_tls_refuses_to_start_rather_than_serving_plaintext() {
        let config = ServerConfig::new("0.0.0.0", 50052).with_tls("cert.pem", "key.pem");

        let err = config
            .ensure_startable()
            .expect_err("a TLS-requesting config must be refused, not served in the clear");

        assert!(
            err.to_string().contains("does not implement TLS"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn a_plaintext_config_starts() {
        assert!(ServerConfig::default().ensure_startable().is_ok());
    }

    #[test]
    fn test_full_address() {
        let config = ServerConfig::new("localhost", 8080);
        assert_eq!(config.full_address(), "localhost:8080");
    }
}
