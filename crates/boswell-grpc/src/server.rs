//! gRPC server configuration and lifecycle management
//!
//! Handles server initialization, binding and shutdown. Note what it does *not*
//! do: there is no TLS termination here (see [`ServerConfig::enable_tls`]), and
//! no authentication of any kind. Per [ADR-021] the gRPC instance sits *inside*
//! the security boundary that the HTTP gateway draws, so it binds to loopback by
//! construction — a routable bind address is a startup error, not a warning.
//!
//! [ADR-021]: ../../../docs/ADRs/021-gateway-is-the-security-boundary.md
//!
//! Shutdown is graceful. The entrypoints below wait on `ctrl_c` and hand that to
//! tonic's `serve_with_shutdown`, which stops accepting new connections and lets
//! the in-flight ones finish. [`start_server_with_shutdown`] takes the signal as
//! a parameter for callers with a lifecycle of their own.

use boswell_domain::traits::{ClaimStore, GoalStore, ProcedureStore};
use boswell_domain::IdentityProvider;
use std::future::Future;
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::{Arc, Mutex};
use tonic::transport::Server;

use crate::proto::bos_well_service_server::BosWellServiceServer;
use crate::service::{BosWellServiceImpl, MetricsSource, ServerExtractor};

/// Why the server refuses to start when `enable_tls` is set.
const TLS_NOT_IMPLEMENTED: &str = "enable_tls is set, but this server does not implement TLS. \
Terminate TLS at a reverse proxy or tunnel in front of the instance, and unset enable_tls.";

/// Why the server refuses to start on an address that is not loopback.
const NON_LOOPBACK_BIND: &str = "this server binds to loopback only. The instance sits inside \
the security boundary and does not authenticate; the HTTP gateway is the component that faces \
a network (ADR-021). Bind 127.0.0.1 or ::1 and put the gateway in front of it.";

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

    /// Get the full server address.
    ///
    /// A bare IPv6 literal is bracketed, because `::1:50051` parses as neither
    /// an address nor a host and port. Anything already bracketed, and every
    /// IPv4 address or hostname, is formatted unchanged.
    pub fn full_address(&self) -> String {
        if self.addr.contains(':') && !self.addr.starts_with('[') {
            format!("[{}]:{}", self.addr, self.port)
        } else {
            format!("{}:{}", self.addr, self.port)
        }
    }

    /// Refuse configurations the server cannot honour.
    ///
    /// Two rules: [`Self::enable_tls`] is not implemented, and the bind address
    /// must be loopback. Both live here, separate from binding, so they can be
    /// exercised without standing a server up — and so they run *before* the
    /// socket is opened.
    ///
    /// # Errors
    /// Returns an error if `enable_tls` is set, since TLS is not implemented at
    /// this layer, or if [`Self::addr`] does not resolve to loopback.
    pub fn ensure_startable(&self) -> Result<(), Box<dyn std::error::Error>> {
        if self.enable_tls {
            return Err(TLS_NOT_IMPLEMENTED.into());
        }
        self.loopback_address()?;
        Ok(())
    }

    /// Resolve [`Self::full_address`] to the socket the server will bind, and
    /// refuse it if it is not loopback.
    ///
    /// Loopback is a rule of construction, not a recommendation (ADR-021). The
    /// instance performs no authentication of its own, so anything that can
    /// reach its port has full write access to every tier; the README used to
    /// *ask* operators to keep it on `127.0.0.1`, which is a different thing
    /// from the server declining to be anywhere else.
    ///
    /// Resolution happens here rather than at the bind so the check and the bind
    /// can never disagree about which address is meant. Every resolved address
    /// must be loopback — a name that answers with one loopback address and one
    /// routable one is refused rather than bound to whichever came first.
    ///
    /// # Errors
    /// Returns an error if the address does not resolve, resolves to nothing, or
    /// resolves to any address that is not loopback.
    fn loopback_address(&self) -> Result<SocketAddr, Box<dyn std::error::Error>> {
        let full = self.full_address();
        let resolved: Vec<SocketAddr> = full.to_socket_addrs()?.collect();

        let first = *resolved
            .first()
            .ok_or_else(|| format!("{full} resolved to no address"))?;

        if let Some(routable) = resolved.iter().find(|a| !a.ip().is_loopback()) {
            return Err(
                format!("{NON_LOOPBACK_BIND} Got {full}, which resolves to {routable}.").into(),
            );
        }

        Ok(first)
    }
}

/// The optional components a server can be started with.
///
/// Every field is a port the instance can run without: no extractor means
/// `Extract` returns `FailedPrecondition`, no identity provider means every
/// self-report is stamped `Assurance::None`, and no metrics source means
/// `GetMetrics` reports `janitor_enabled: false`. Bundled into one struct so
/// attaching the next port does not add another positional parameter to every
/// entrypoint below.
#[derive(Default, Clone)]
pub struct ServerComponents {
    /// Backs the `Extract` RPC and LLM-mode hook ingest.
    pub extractor: Option<Arc<dyn ServerExtractor>>,
    /// The identity port (design §6), which grades a delegation chain into a
    /// tier ceiling.
    pub identity: Option<Arc<dyn IdentityProvider + Send + Sync>>,
    /// The running Janitor's counters, read by the `GetMetrics` RPC.
    pub metrics: Option<Arc<dyn MetricsSource>>,
}

/// Start the gRPC server with no optional components attached.
///
/// Use [`start_server_with_components`] to attach an extractor, an identity
/// provider or the Janitor's metrics.
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
    start_server_with_components(config, store, ServerComponents::default()).await
}

/// Start the gRPC server with the given optional components, shutting down on
/// `ctrl_c`.
///
/// The identity port is taken as a trait object, never a concrete adapter, so
/// this crate — and every other crate on the production path — stays ignorant of
/// which provider is in play.
///
/// # Errors
/// Returns error if server fails to start or bind to address
pub async fn start_server_with_components<S>(
    config: ServerConfig,
    store: Arc<Mutex<S>>,
    components: ServerComponents,
) -> Result<(), Box<dyn std::error::Error>>
where
    S: ClaimStore + GoalStore + ProcedureStore + Send + 'static,
    S::Error: std::fmt::Debug,
{
    start_server_with_shutdown(config, store, components, ctrl_c_signal()).await
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
    components: ServerComponents,
    shutdown: impl Future<Output = ()>,
) -> Result<(), Box<dyn std::error::Error>>
where
    S: ClaimStore + GoalStore + ProcedureStore + Send + 'static,
    S::Error: std::fmt::Debug,
{
    // Refuse before binding: an operator who set `enable_tls` must not end up
    // serving plaintext under the impression they are serving TLS, and an
    // instance that authenticates nothing must not end up on a routable address.
    config.ensure_startable()?;

    let addr = config.loopback_address()?;

    let mut service = BosWellServiceImpl::new(store);
    if let Some(extractor) = components.extractor {
        service = service.with_extractor(extractor);
    }
    if let Some(identity) = components.identity {
        service = service.with_identity_provider(identity);
    }
    if let Some(metrics) = components.metrics {
        service = service.with_metrics_source(metrics);
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
                ServerComponents::default(),
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
                ServerComponents::default(),
                std::future::ready(()),
            ),
        )
        .await
        .expect("an already-fired signal should stop the server at once");

        served.expect("a graceful shutdown is not an error");
    }

    /// The TLS refusal runs before the socket is opened, so it must reach the
    /// caller as an error rather than being overtaken by the shutdown signal.
    ///
    /// The `timeout` is the failure mode, not a performance guard: the signal is
    /// `pending()`, so a regression that lets the server bind would hang here
    /// forever instead of failing.
    #[tokio::test]
    async fn a_tls_config_is_refused_even_with_a_shutdown_signal_in_hand() {
        let err = tokio::time::timeout(
            Duration::from_secs(5),
            start_server_with_shutdown(
                ServerConfig::new("127.0.0.1", free_port()).with_tls("cert.pem", "key.pem"),
                in_memory_store(),
                ServerComponents::default(),
                std::future::pending(),
            ),
        )
        .await
        .expect("the refusal must come back, not leave the server serving")
        .expect_err("a TLS-requesting config must be refused, not served in the clear");

        assert!(
            err.to_string().contains("does not implement TLS"),
            "unexpected error: {err}"
        );
    }

    /// The loopback rule must bite at the real entrypoint, not only on the
    /// config method: a routable address has to fail *before* the socket opens.
    ///
    /// Same `timeout` reasoning as the TLS test above — without it, a regression
    /// binds `0.0.0.0` and waits on a signal that never fires.
    #[tokio::test]
    async fn a_routable_config_is_refused_before_the_socket_opens() {
        let err = tokio::time::timeout(
            Duration::from_secs(5),
            start_server_with_shutdown(
                ServerConfig::new("0.0.0.0", free_port()),
                in_memory_store(),
                ServerComponents::default(),
                std::future::pending(),
            ),
        )
        .await
        .expect("the refusal must come back, not leave the server bound to 0.0.0.0")
        .expect_err("a routable bind address must not reach the listener");

        assert!(
            err.to_string().contains("binds to loopback only"),
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
        let config = ServerConfig::new("127.0.0.1", 50052).with_tls("cert.pem", "key.pem");

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

    #[test]
    fn an_ipv6_literal_is_bracketed_so_it_resolves() {
        let config = ServerConfig::new("::1", 8080);
        assert_eq!(config.full_address(), "[::1]:8080");
        config
            .ensure_startable()
            .expect("the IPv6 loopback address is loopback");
    }

    /// The rule ADR-021 turned from a README request into a startup error.
    #[test]
    fn a_routable_bind_address_refuses_to_start() {
        for addr in ["0.0.0.0", "192.168.1.10", "::"] {
            let err = ServerConfig::new(addr, 50051)
                .ensure_startable()
                .expect_err("a routable bind address must be refused");
            assert!(
                err.to_string().contains("binds to loopback only"),
                "unexpected error for {addr}: {err}"
            );
        }
    }

    #[test]
    fn localhost_starts_because_it_resolves_to_loopback() {
        ServerConfig::new("localhost", 50051)
            .ensure_startable()
            .expect("localhost resolves to loopback");
    }

    #[test]
    fn the_resolved_bind_address_is_the_one_that_passed_the_check() {
        let resolved = ServerConfig::new("127.0.0.1", 50051)
            .loopback_address()
            .expect("127.0.0.1 is loopback");
        assert_eq!(resolved.to_string(), "127.0.0.1:50051");
    }
}
