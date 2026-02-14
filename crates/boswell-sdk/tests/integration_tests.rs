//! Integration tests for the Boswell SDK
//!
//! Note: Full end-to-end integration tests require starting Router + gRPC servers.
//! For Phase 2, we test the core SDK behavior with unit tests.
//! Full integration testing will be added in Phase 3 with async SDK.

use boswell_sdk::{BoswellClient, SdkError};

#[test]
fn test_sdk_not_connected_error() {
    let mut client = BoswellClient::new("http://localhost:8080");
    
    // Try to assert without connecting
    let result = client.assert(
        "test",
        "subject",
        "predicate",
        "object",
        Some(0.9),
        None,
    );

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(matches!(err, SdkError::NotConnected));
}

#[test]
fn test_sdk_client_creation() {
    let client = BoswellClient::new("http://localhost:8080");
    // Client should be created successfully
    // Connection happens on connect()
    std::mem::drop(client); // Silence unused variable warning
}


