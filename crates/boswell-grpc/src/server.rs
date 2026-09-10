//! gRPC server configuration and lifecycle management
//!
//! Handles server initialization, binding and shutdown. Note what it does *not*
//! do: there is no TLS termination here (see [`ServerConfig::enable_tls`]).
//!
//! Shutdown is graceful. The entrypoints below wait on `ctrl_c` and hand that to
//! tonic's `serve_with_shutdown`, which stops accepting new connections and lets
//! the in-flight ones finish. [`start_server_with_shutdown`] takes the signal as
//! a parameter for callers with a lifecycle of their own.

use boswell_domain::traits::{ClaimStore, GoalStore, ProcedureStore};
use boswell_domain::IdentityProvider;
use std::future::Future;
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
    /// instance — the standing position recorded in the README and in
    /// `boswell-gateway/src/config.rs`.
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
    start_server_with_shutdown(config, store, extractor, identity, ctrl_c_signal()).await
}

/// Start the gRPC server, shutting it down when `shutdown` resolves.
///
/// This is the entrypoint the others delegate to; they supply `ctrl_c` as the
/// signal. Take this one when the process has a lifecycle of its own — a
/// supervisor, a test — and `ctrl_c` is the wrong thing to wait on.
///
/// Shutdown is graceful in tonic's sense: the listener closes, in-flight
/// requests run to completion, and only then does this future return `Ok`.
/// A caller that needs a deadline should wrap the call in `tokio::time::timeout`
/// — there is no bound on how long a hung handler can hold shutdown open.
///
/// # Errors
/// Returns error if server fails to start or bind to address
pub async fn start_server_with_shutdown<S>(
    config: ServerConfig,
    store: Arc<Mutex<S>>,
    extractor: Option<Arc<dyn ServerExtractor>>,
    identity: Option<Arc<dyn IdentityProvider + Send + Sync>>,
    shutdown: impl Future<Output = ()>,
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
        .serve_with_shutdown(addr, shutdown)
        .await?;

    println!("BosWell gRPC server stopped");

    Ok(())
}

/// The default shutdown signal: `ctrl_c`, matching the Janitor and Synthesizer
/// workers.
///
/// A failure to *install* the handler resolves the future rather than
/// propagating, which would shut the server down at startup. That case is
/// vanishingly rare, so it is reported rather than swallowed; the alternative —
/// pending forever — would leave a server that cannot be stopped by the one
/// signal an operator will try.
async fn ctrl_c_signal() {
    if let Err(e) = tokio::signal::ctrl_c().await {
        eprintln!("Failed to listen for shutdown signal, stopping: {e}");
        return;
    }
    println!("Shutdown signal received, stopping BosWell gRPC server");
}

#[cfg(test)]
mod tests {
    use super::*;
    use boswell_store::SqliteStore;
    use std::time::Duration;

    /// Ask the OS for a free port, then let go of it.
    ///
    /// There is a race here — nothing stops another process taking the port
    /// between the drop and the server's bind — but the alternative is a
    /// hard-coded port, which races with every *other* run of the suite. The
    /// server cannot report its own bound port, since `serve_with_shutdown`
    /// takes a `SocketAddr` and returns nothing until it stops.
    fn free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0")
            .expect("the loopback interface should hand out an ephemeral port")
            .local_addr()
            .expect("a bound listener has a local address")
            .port()
    }

    fn in_memory_store() -> Arc<Mutex<SqliteStore>> {
        Arc::new(Mutex::new(
            SqliteStore::new(":memory:", false, 0).expect("in-memory store should open"),
        ))
    }

    /// The regression this slice exists for: before `serve_with_shutdown`, this
    /// future never returned, and the only way to stop the server was to kill
    /// the process.
    #[tokio::test]
    async fn the_server_returns_when_the_shutdown_signal_fires_while_it_is_serving() {
        let port = free_port();
        let (tx, rx) = tokio::sync::oneshot::channel();

        // Signal only once the server is actually accepting connections, so a
        // pass means it served *and* stopped, not that it never started.
        tokio::spawn(async move {
            for _ in 0..200 {
                if tokio::net::TcpStream::connect(("127.0.0.1", port))
                    .await
                    .is_ok()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            let _ = tx.send(());
        });

        let served = tokio::time::timeout(
            Duration::from_secs(10),
            start_server_with_shutdown(
                ServerConfig::new("127.0.0.1", port),
                in_memory_store(),
                None,
                None,
                async move {
                    let _ = rx.await;
                },
            ),
        )
        .await
        .expect("the server should stop on the signal, not run until the test times out");

        served.expect("a graceful shutdown is not an error");
    }

    /// A signal that resolves immediately must still leave the server bound
    /// cleanly first — shutdown is not an error path.
    #[tokio::test]
    async fn a_shutdown_signal_that_has_already_fired_stops_the_server_cleanly() {
        let served = tokio::time::timeout(
            Duration::from_secs(10),
            start_server_with_shutdown(
                ServerConfig::new("127.0.0.1", free_port()),
                in_memory_store(),
                None,
                None,
                std::future::ready(()),
            ),
        )
        .await
        .expect("an already-fired signal should stop the server at once");

        served.expect("a graceful shutdown is not an error");
    }

    /// The TLS refusal runs before the socket is opened, so it must reach the
    /// caller as an error rather than being overtaken by the shutdown signal.
    #[tokio::test]
    async fn a_tls_config_is_refused_even_with_a_shutdown_signal_in_hand() {
        let err = start_server_with_shutdown(
            ServerConfig::new("127.0.0.1", free_port()).with_tls("cert.pem", "key.pem"),
            in_memory_store(),
            None,
            None,
            std::future::pending(),
        )
        .await
        .expect_err("a TLS-requesting config must be refused, not served in the clear");

        assert!(
            err.to_string().contains("does not implement TLS"),
            "unexpected error: {err}"
        );
    }

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
