//! End-to-End Integration Tests for Boswell SDK
//!
//! Full E2E tests require manually starting Router and gRPC servers.
//! These tests focus on SDK integration behavior.
//!
//! To run full manual E2E tests:
//! 1. Start gRPC server: `cargo run -p boswell-grpc`
//! 2. Start Router: `cargo run -p boswell-router --config config/router.toml`
//! 3. Run tests: `cargo test -p boswell-sdk --test e2e_tests -- --ignored`

use boswell_sdk::{BoswellClient, QueryFilter, SdkError};
use boswell_domain::Tier;

#[tokio::test]
async fn test_sdk_not_connected_error() {
    let mut client = BoswellClient::new("http://localhost:9999");

    // Try to assert without connecting
    let result = client
        .assert("test", "subject", "predicate", "object", Some(0.9), None)
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
// Manual E2E Tests (require servers to be running)
// Run with: cargo test -p boswell-sdk --test e2e_tests -- --ignored
// ============================================================================

#[tokio::test]
#[ignore] // Requires manually started servers
async fn test_e2e_full_flow() {
    // Expects Router on localhost:8080 and gRPC instance registered
    let mut client = BoswellClient::new("http://localhost:8080");

    // Connect to router
    client.connect().await.expect("Failed to connect - is Router running?");

    // Assert a claim
    let claim_id = client
        .assert(
            "test_e2e",
            "Alice",
            "knows",
            "Rust",
            Some(0.95),
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
#[ignore] // Requires manually started servers
async fn test_e2e_batch_operations() {
    let mut client = BoswellClient::new("http://localhost:8080");
    client.connect().await.expect("Failed to connect");

    // Assert multiple claims
    let id1 = client
        .assert("test_batch", "A", "type", "one", Some(0.9), Some(Tier::Task))
        .await
        .expect("Failed to assert claim 1");

    let id2 = client
        .assert("test_batch", "B", "type", "two", Some(0.8), Some(Tier::Task))
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
#[ignore] // Requires manually started servers
async fn test_e2e_confidence_filtering() {
    let mut client = BoswellClient::new("http://localhost:8080");
    client.connect().await.expect("Failed to connect");

    // Assert claims with different confidence
    let id_high = client
        .assert("test_conf", "high", "confidence", "0.95", Some(0.95), None)
        .await
        .expect("Failed to assert high");

    let id_low = client
        .assert("test_conf", "low", "confidence", "0.55", Some(0.55), None)
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

    // Should only get the high confidence claim
    let high_subjects: Vec<_> = high_conf.iter().map(|c| c.subject.as_str()).collect();
    assert!(high_subjects.contains(&"high"));

    // Clean up
    client.forget(vec![id_high, id_low]).await.ok();
}
