//! End-to-end tests that serve the real gRPC stack over a TCP socket and drive
//! it with a tonic client — the same path a client (via the router) uses.
//!
//! The default test uses the offline mock embedder so it runs in CI. A second,
//! `#[ignore]`d test exercises the real Ollama embedder end-to-end.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use boswell_grpc::proto::bos_well_service_client::BosWellServiceClient;
use boswell_grpc::proto::bos_well_service_server::BosWellServiceServer;
use boswell_grpc::proto::{AssertRequest, ConfidenceInterval, SearchRequest, Tier};
use boswell_grpc::BosWellServiceImpl;
use boswell_server::{
    build_store, EmbeddingBackend, EmbeddingConfig, InstanceConfig, StorageConfig,
};
use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::transport::Channel;

/// Start a server on an OS-assigned port and return its base URL.
async fn spawn_server(backend: EmbeddingBackend, model: &str) -> String {
    let config = InstanceConfig {
        bind_address: "127.0.0.1".to_string(),
        bind_port: 0, // unused: we bind the listener ourselves to learn the port
        storage: StorageConfig {
            db_path: ":memory:".to_string(),
        },
        embedding: EmbeddingConfig {
            backend,
            model: model.to_string(),
            mock_dimension: 64,
            ..EmbeddingConfig::default()
        },
        janitor: Default::default(),
        synthesizer: Default::default(),
    };

    let store = Arc::new(Mutex::new(build_store(&config).expect("build_store failed")));
    let service = BosWellServiceServer::new(BosWellServiceImpl::new(store));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(service)
            .serve_with_incoming(TcpListenerStream::new(listener))
            .await
            .unwrap();
    });

    format!("http://{addr}")
}

/// Connect a client, retrying briefly while the server task starts accepting.
async fn connect(url: &str) -> BosWellServiceClient<Channel> {
    for _ in 0..40 {
        if let Ok(client) = BosWellServiceClient::connect(url.to_string()).await {
            return client;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("could not connect to test server at {url}");
}

fn assert_request(auth: &str) -> AssertRequest {
    AssertRequest {
        namespace: "lang".to_string(),
        subject: "rust".to_string(),
        predicate: "is_a".to_string(),
        object: "programming_language".to_string(),
        confidence: Some(ConfidenceInterval {
            lower: 0.9,
            upper: 0.95,
        }),
        tier: Tier::Permanent as i32,
        provenance: vec![],
        auth_token: auth.to_string(),
    }
}

#[tokio::test]
async fn test_assert_then_search_over_tcp_mock_backend() {
    let url = spawn_server(EmbeddingBackend::Mock, "unused").await;
    let mut client = connect(&url).await;

    let asserted = client
        .assert(assert_request("test-token"))
        .await
        .expect("assert failed")
        .into_inner();
    assert!(!asserted.claim_id.is_empty());

    // Mock embeds identical text deterministically, so exact-text search returns
    // the claim with near-perfect similarity.
    let search = client
        .search(SearchRequest {
            query_text: "rust is_a programming_language".to_string(),
            namespace: None,
            limit: 10,
            min_similarity: 0.5,
            auth_token: "test-token".to_string(),
        })
        .await
        .expect("search failed")
        .into_inner();

    assert_eq!(search.results.len(), 1);
    let hit = &search.results[0];
    assert_eq!(hit.claim.as_ref().unwrap().subject, "rust");
    assert!(hit.similarity > 0.9, "similarity was {}", hit.similarity);
}

#[tokio::test]
async fn test_search_missing_auth_is_rejected() {
    let url = spawn_server(EmbeddingBackend::Mock, "unused").await;
    let mut client = connect(&url).await;

    let status = client
        .search(SearchRequest {
            query_text: "rust".to_string(),
            namespace: None,
            limit: 10,
            min_similarity: 0.5,
            auth_token: String::new(),
        })
        .await
        .expect_err("expected an error for missing auth");
    assert_eq!(status.code(), tonic::Code::Unauthenticated);
}

#[tokio::test]
#[ignore = "requires a local Ollama with embeddinggemma"]
async fn test_assert_then_search_over_tcp_real_embedder() {
    let url = spawn_server(EmbeddingBackend::Ollama, "embeddinggemma").await;
    let mut client = connect(&url).await;

    client
        .assert(assert_request("test-token"))
        .await
        .expect("assert failed");

    // A conceptual query that never matches the literal claim text.
    let search = client
        .search(SearchRequest {
            query_text: "a systems programming language".to_string(),
            namespace: None,
            limit: 5,
            min_similarity: 0.0,
            auth_token: "test-token".to_string(),
        })
        .await
        .expect("search failed")
        .into_inner();

    assert!(!search.results.is_empty(), "expected a semantic match");
    assert_eq!(search.results[0].claim.as_ref().unwrap().subject, "rust");
}
