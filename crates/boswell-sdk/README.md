# Boswell SDK

Rust client library for interacting with Boswell instances via the Router.

## Features

- **Session Management**: Automatic session establishment with Router
- **gRPC Operations**: Wraps gRPC calls with type-safe Rust APIs
- **Error Handling**: Comprehensive error types for different failure modes
- **Async API**: Fully asynchronous using tokio runtime
- **Connection Pooling**: HTTP connection pooling for efficient Router communication
- **Auto-Reconnection**: Automatic session renewal on authentication failures

## Usage

```rust
use boswell_sdk::{BoswellClient, QueryFilter};
use boswell_domain::Tier;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create and connect client
    let mut client = BoswellClient::new("http://localhost:8080");
    client.connect().await?;

    // Assert a claim
    let claim_id = client.assert(
        "workspace",
        "document.pdf",
        "contains",
        "financial_data",
        Some(0.92),
        Some(Tier::Project),
    ).await?;

    // Query claims
    let claims = client.query(QueryFilter {
        namespace: Some("workspace".to_string()),
        ..Default::default()
    }).await?;

    // Learn multiple claims in batch
    let response = client.learn(claims).await?;

    // Forget claims
    client.forget(vec![claim_id]).await?;

    Ok(())
}
```

## Architecture

The SDK follows this flow:

1. **Session Establishment**: Client POSTs to Router `/session/establish` to get JWT token and instance topology
2. **gRPC Connection**: Client connects to assigned instance endpoint
3. **Operations**: Client includes token in all gRPC request `auth_token` fields
4. **Error Handling**: Maps gRPC errors to user-friendly SDK errors

## Error Handling

- `SdkError::NotConnected`: connect() must be called first
- `SdkError::RouterError`: Router API or network error  
- `SdkError::GrpcError`: gRPC service error
- `SdkError::AuthError`: Authentication or session expiry
- `SdkError::ConnectionError`: Network connectivity issue

## Testing

```bash
cargo test -p boswell-sdk
```

## Phase 2C Implementation

This SDK implements Phase 2C requirements:
- ✅ BoswellClient with session management
- ✅ Synchronous assert/query/learn/forget operations
- ✅ HTTP Router communication
- ✅ gRPC instance integration
- ✅ Unit tests with error validation

Full end-to-end integration tests will be added in Phase 3 with async SDK support.
