//! End-to-end integration tests for the Boswell SDK.
//!
//! These run the whole path — SDK → Router → gRPC → store — with no manual
//! setup and nothing `#[ignore]`d. [`stack::Stack`] stands a gRPC instance and a
//! router up in-process on ephemeral ports, over an in-memory SQLite store, and
//! stops both when it drops. Each test gets its own stack, so they neither share
//! a store nor collide on a namespace.
//!
//! Before this, the full-stack tests were ignored and expected servers started
//! by hand on fixed ports, which meant CI never exercised the SDK against a real
//! router at all.

use boswell_domain::Tier;
use boswell_sdk::{BoswellClient, QueryFilter, SdkError};
use stack::Stack;

/// A full Boswell stack, running in the test process.
mod stack {
    use boswell_grpc::server::{start_server_with_shutdown, ServerConfig};
    use boswell_router::config::{InstanceConfig, RouterConfig};
    use boswell_router::start_server_with_shutdown as start_router_with_shutdown;
    use boswell_store::SqliteStore;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tokio::sync::oneshot;

    /// How long to wait for a listener to start accepting. Generous: a cold CI
    /// runner binds slowly, and the cost of over-waiting is nothing, while the
    /// cost of under-waiting is a flake.
    const READY_TIMEOUT: Duration = Duration::from_secs(10);

    /// Poll interval while waiting for a listener.
    const READY_POLL: Duration = Duration::from_millis(10);

    /// A gRPC instance and a router that has it registered, both running on
    /// loopback in this process. Dropping the stack stops both.
    pub struct Stack {
        router_endpoint: String,
        shutdown: Vec<oneshot::Sender<()>>,
    }

    impl Stack {
        /// Start both servers and return once each is accepting connections.
        ///
        /// # Panics
        /// Panics if either server fails to come up inside [`READY_TIMEOUT`] —
        /// there is nothing a test can do with a half-built stack.
        pub async fn start() -> Self {
            let grpc_port = free_port();
            let router_port = free_port();

            let store = Arc::new(Mutex::new(
                SqliteStore::new(":memory:", false, 0).expect("an in-memory store should open"),
            ));

            let (grpc_tx, grpc_rx) = oneshot::channel();
            tokio::spawn(async move {
                if let Err(e) = start_server_with_shutdown(
                    ServerConfig::new("127.0.0.1", grpc_port),
                    store,
                    None,
                    None,
                    async move {
                        let _ = grpc_rx.await;
                    },
                )
                .await
                {
                    eprintln!("gRPC instance stopped with error: {e}");
                }
            });

            let config = RouterConfig {
                bind_address: "127.0.0.1".to_string(),
                bind_port: router_port,
                jwt_secret: "e2e-test-secret".to_string(),
                token_expiry_secs: 3600,
                instances: vec![InstanceConfig {
                    id: "e2e".to_string(),
                    endpoint: format!("http://127.0.0.1:{grpc_port}"),
                    expertise: vec!["*".to_string()],
                }],
            };

            let (router_tx, router_rx) = oneshot::channel();
            tokio::spawn(async move {
                if let Err(e) = start_router_with_shutdown(config, async move {
                    let _ = router_rx.await;
                })
                .await
                {
                    eprintln!("router stopped with error: {e}");
                }
            });

            // Wait on both, so a test that fails does so on its own assertion
            // rather than on a connection refused by a server still binding.
            await_listener("gRPC instance", grpc_port).await;
            await_listener("router", router_port).await;

            Self {
                router_endpoint: format!("http://127.0.0.1:{router_port}"),
                shutdown: vec![grpc_tx, router_tx],
            }
        }

        /// The router URL to hand [`boswell_sdk::BoswellClient::new`].
        pub fn router_endpoint(&self) -> &str {
            &self.router_endpoint
        }
    }

    impl Drop for Stack {
        fn drop(&mut self) {
            // `send` is synchronous, which is what makes this possible in
            // `drop`. Both servers shut down gracefully on their own tasks; the
            // test process does not wait for them, and does not need to.
            for tx in self.shutdown.drain(..) {
                let _ = tx.send(());
            }
        }
    }

    /// Claim an ephemeral port from the OS and release it, so a server can bind
    /// it a moment later.
    ///
    /// There is a race here — the same one `boswell-grpc`'s own server tests
    /// live with. Neither `tonic` nor the router's `start_server` takes a
    /// pre-bound listener, so the port has to be chosen before the bind. The
    /// window is microseconds and the ports are ephemeral.
    fn free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0")
            .expect("the loopback interface should hand out an ephemeral port")
            .local_addr()
            .expect("a bound listener has a local address")
            .port()
    }

    /// Block until something is accepting on `port`, or panic after
    /// [`READY_TIMEOUT`].
    async fn await_listener(what: &str, port: u16) {
        let deadline = tokio::time::Instant::now() + READY_TIMEOUT;
        while tokio::time::Instant::now() < deadline {
            if tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .is_ok()
            {
                return;
            }
            tokio::time::sleep(READY_POLL).await;
        }
        panic!("{what} never started accepting on port {port}");
    }
}

#[tokio::test]
async fn test_sdk_not_connected_error() {
    let mut client = BoswellClient::new("http://localhost:9999");

    // Try to assert without connecting
    let result = client
        .assert(
            "test",
            "subject",
            "predicate",
            "object",
            Some((0.9, 0.9)),
            None,
        )
        .await;

    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), SdkError::NotConnected));
}

#[tokio::test]
async fn test_sdk_connection_failure() {
    let mut client = BoswellClient::new("http://localhost:9999");

    // Try to connect to non-existent server
    let result = client.connect().await;

    assert!(result.is_err());
    // Should be a connection error or router error
    match result.unwrap_err() {
        SdkError::ConnectionError(_) | SdkError::RouterError(_) => {
            // Expected error types
        }
        other => panic!("Unexpected error type: {:?}", other),
    }
}

#[tokio::test]
async fn test_sdk_query_not_connected() {
    let mut client = BoswellClient::new("http://localhost:8080");

    let result = client
        .query(QueryFilter {
            namespace: Some("test".to_string()),
            ..Default::default()
        })
        .await;

    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), SdkError::NotConnected));
}

#[tokio::test]
async fn test_sdk_learn_not_connected() {
    let mut client = BoswellClient::new("http://localhost:8080");

    let result = client.learn(vec![]).await;

    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), SdkError::NotConnected));
}

#[tokio::test]
async fn test_sdk_forget_not_connected() {
    let mut client = BoswellClient::new("http://localhost:8080");

    let result = client.forget(vec![]).await;

    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), SdkError::NotConnected));
}

// ============================================================================
// Full-stack tests: SDK -> Router -> gRPC -> store, all in-process.
// ============================================================================

#[tokio::test]
async fn test_e2e_full_flow() {
    let stack = Stack::start().await;
    let mut client = BoswellClient::new(stack.router_endpoint());

    // Connect to router
    client.connect().await.expect("Failed to connect to router");

    // Assert a claim
    let claim_id = client
        .assert(
            "test_e2e",
            "Alice",
            "knows",
            "Rust",
            Some((0.95, 0.95)),
            Some(Tier::Permanent),
        )
        .await
        .expect("Failed to assert claim");

    assert!(!claim_id.to_string().is_empty());

    // Query the claim back
    let claims = client
        .query(QueryFilter {
            namespace: Some("test_e2e".to_string()),
            ..Default::default()
        })
        .await
        .expect("Failed to query claims");

    assert!(!claims.is_empty());

    // Find our claim
    let our_claim = claims.iter().find(|c| c.id == claim_id);
    assert!(our_claim.is_some());

    let claim = our_claim.unwrap();
    assert_eq!(claim.namespace, "test_e2e");
    assert_eq!(claim.subject, "Alice");
    assert_eq!(claim.predicate, "knows");
    assert_eq!(claim.object, "Rust");

    // Clean up - forget the claim
    let success = client
        .forget(vec![claim_id])
        .await
        .expect("Failed to forget claim");

    assert!(success);
}

#[tokio::test]
async fn test_e2e_batch_operations() {
    let stack = Stack::start().await;
    let mut client = BoswellClient::new(stack.router_endpoint());
    client.connect().await.expect("Failed to connect");

    // Assert multiple claims
    let id1 = client
        .assert(
            "test_batch",
            "A",
            "type",
            "one",
            Some((0.9, 0.9)),
            Some(Tier::Task),
        )
        .await
        .expect("Failed to assert claim 1");

    let id2 = client
        .assert(
            "test_batch",
            "B",
            "type",
            "two",
            Some((0.8, 0.8)),
            Some(Tier::Task),
        )
        .await
        .expect("Failed to assert claim 2");

    // Query all
    let claims = client
        .query(QueryFilter {
            namespace: Some("test_batch".to_string()),
            ..Default::default()
        })
        .await
        .expect("Failed to query claims");

    assert!(claims.len() >= 2);

    // Clean up
    client.forget(vec![id1, id2]).await.ok();
}

#[tokio::test]
async fn test_e2e_confidence_filtering() {
    let stack = Stack::start().await;
    let mut client = BoswellClient::new(stack.router_endpoint());
    client.connect().await.expect("Failed to connect");

    // Assert claims with different confidence
    let id_high = client
        .assert(
            "test_conf",
            "high",
            "confidence",
            "0.95",
            Some((0.95, 0.95)),
            None,
        )
        .await
        .expect("Failed to assert high");

    let id_low = client
        .assert(
            "test_conf",
            "low",
            "confidence",
            "0.55",
            Some((0.55, 0.55)),
            None,
        )
        .await
        .expect("Failed to assert low");

    // Query with confidence threshold
    let high_conf = client
        .query(QueryFilter {
            namespace: Some("test_conf".to_string()),
            min_confidence: Some(0.8),
            ..Default::default()
        })
        .await
        .expect("Failed to query high confidence");

    // Should get the high-confidence claim and NOT the low-confidence one —
    // asserting both presence and exclusion so a broken min_confidence filter
    // (returning everything) actually fails the test.
    let high_subjects: Vec<_> = high_conf.iter().map(|c| c.subject.as_str()).collect();
    assert!(
        high_subjects.contains(&"high"),
        "high-confidence claim missing"
    );
    assert!(
        !high_subjects.contains(&"low"),
        "low-confidence claim should have been filtered out"
    );

    // Clean up
    client.forget(vec![id_high, id_low]).await.ok();
}
