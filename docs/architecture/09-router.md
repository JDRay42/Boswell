# Boswell — Router

The Router is the session management and instance registry authority for all Boswell deployments. It is **always present**, even in single-instance configurations, where it adds minimal overhead (<1MB memory, near-zero CPU). The Router responds to session requests from authorized clients with a list of registered instances, their endpoints, capabilities, health status, and instance-specific tokens.

> **Status.** Most of this document is design, not description. What exists today is a small
> axum HTTP service: it parses a plaintext TOML file, answers `POST /session/establish` with
> one HS256 JWT and the configured instance list, and answers `GET /health` from registry
> state that nothing ever updates. The encrypted config, the health monitor, per-instance
> tokens and mTLS are unbuilt, and
> [ADR-021](../ADRs/021-gateway-is-the-security-boundary.md) has since moved authentication
> to the HTTP gateway. Sections describing unbuilt behavior are marked below.
> [`docs/development/roadmap.md`](../development/roadmap.md) is the source of truth for
> status, and [`10-security.md`](10-security.md) is normative for the security model.

## Responsibility

The Router is the single source of truth for:

- **Instance registry:** Maintains the list of registered instances with their cryptographic fingerprints, endpoints, capabilities, and health states.
- **Session token issuance:** Issues one token per instance in response to authenticated session requests. Each token is scoped to a specific instance.
- **Health tracking:** Reports each instance's health status in session responses. *Designed to poll; does not yet — see [Health Monitor](#health-monitor).*
- **Configuration management:** Holds the configuration file containing all registry data. *Plaintext TOML today; the encrypted portable form is unbuilt.*

**What the Router is NOT:**

- **Not a proxy.** After session establishment, clients route all operations directly to instances. The Router is not in the hot path.
- **Not automatic discovery.** Instance registration is manual and deliberate. Adding a new instance requires explicit administrative action.
- **Not a centralized data store.** The Router holds no claim data — only metadata about instances.

## Architecture

### Session Establishment Flow

```mermaid
sequenceDiagram
    participant Client as Client SDK / MCP Server
    participant Router
    participant InstanceA as Instance A
    participant InstanceB as Instance B

    Note over Router: Encrypted config loaded in memory<br/>(instance registry, keypairs)

    Client->>Router: SessionRequest<br/>(mTLS authentication)
    Router->>Router: Verify client identity
    Router->>Router: Generate tokens:<br/>- token_A for Instance A<br/>- token_B for Instance B
    
    Router-->>Client: SessionResponse {<br/>  instances: [<br/>    {id: "A", endpoint: "host:9001", capabilities: [...], token: "token_A", health: "healthy"},<br/>    {id: "B", endpoint: "host:9002", capabilities: [...], token: "token_B", health: "degraded"}<br/>  ]<br/>}
    
    Note over Client: Client now has direct access info<br/>for all registered instances

    Client->>InstanceA: Assert(claim, token_A)
    InstanceA->>InstanceA: Validate token_A
    InstanceA-->>Client: Success

    Client->>InstanceB: Query(params, token_B)
    InstanceB->>InstanceB: Validate token_B
    InstanceB-->>Client: Results
```

### Health Monitoring Flow

```mermaid
sequenceDiagram
    participant Router
    participant InstanceA as Instance A
    participant InstanceB as Instance B
    participant InstanceC as Instance C

    loop Every 60s (configurable)
        Router->>InstanceA: Health check (gRPC)
        InstanceA-->>Router: OK (healthy)
        
        Router->>InstanceB: Health check (gRPC)
        InstanceB-->>Router: Slow response (degraded)
        
        Router->>InstanceC: Health check (gRPC)
        Note over Router,InstanceC: Timeout / Connection refused
        Router->>Router: Mark C as unreachable
    end
    
    Note over Router: Health states included in<br/>next SessionResponse
```

## Not in the Hot Path

The Router is **not a proxy for routine operations**. After the session handshake, clients route operations directly to instances using the instance-specific tokens and endpoint information they received. The Router is only contacted for:

1. **Session establishment:** Initial authentication and token issuance for all registered instances.
2. **Token refresh:** When tokens expire (default: 1 hour), clients request a new SessionResponse to get fresh tokens.
3. **Registry updates:** When the client needs to check for new instances or updated health states.

Everything else — Assert, Query, Learn, Extract, Challenge, Promote, Forget — goes directly from client to instance with no Router involvement.

## Instance Registry

The registry is the Router's core data structure. It tracks all registered instances.

```rust
pub struct InstanceEntry {
    pub instance_id: String,
    pub endpoint: String,               // gRPC endpoint (host:port)
    pub fingerprint: Vec<u8>,           // Public key fingerprint for mTLS verification
    pub capabilities: Vec<String>,      // Supported operations (e.g., ["assert", "query", "learn"])
    pub health: InstanceHealth,         // Current health state
    pub last_health_check: DateTime,
}

pub enum InstanceHealth {
    Healthy,      // Responding normally
    Degraded,     // Responding slowly or with partial errors
    Unreachable,  // Not responding to health checks
}
```

### Manual Registration

*Partly built.* Registration is manual, but a registered instance is a `[[instances]]` table
of `id`, `endpoint` and `expertise` — there is no fingerprint field and no capability
declaration.

**Instance registration is manual and deliberate.** There is no automatic discovery mechanism. Adding a new instance requires:

1. **Administrative action:** Editing the Router's encrypted configuration file.
2. **Cryptographic identity:** The instance's public key fingerprint must be added to the registry.
3. **Endpoint configuration:** One or more network endpoints (LAN IP, VPN address, etc.).
4. **Capability declaration:** The set of operations this instance supports.

This deliberate process ensures that only trusted instances join the network. Automatic discovery would create security and trust management challenges.

### Multiple Endpoints

*Not built.* `InstanceConfig::endpoint` is a single string. One instance, one endpoint.

An instance may have multiple endpoints registered to support different network contexts:
- **LAN address** for when the client is on the same local network
- **VPN address** for remote access
- **Public endpoint** (if appropriate for the deployment)

The client SDK can try endpoints in order based on reachability and network context.

### Health States and Transitions

*Not built.* `HealthStatus` has the three variants, and `InstanceRegistry::update_health`
would apply a transition, but nothing calls it outside tests. Every instance is `Healthy`
from `from_config` onward, so `GET /health` reports `healthy` whether or not any instance
is running.

```mermaid
stateDiagram-v2
    [*] --> Healthy: Instance registered
    Healthy --> Degraded: Slow response or partial failure
    Healthy --> Unreachable: 2 consecutive health check failures
    Degraded --> Healthy: 2 consecutive successful checks
    Degraded --> Unreachable: 2 consecutive health check failures
    Unreachable --> Healthy: 2 consecutive successful checks
    note right of Unreachable: Instance remains in registry<br/>Clients may retry
```

| State | Meaning | Included in SessionResponse | Client Behavior |
|---|---|---|---|
| Healthy | Responding normally within timeout | Yes | Route operations normally |
| Degraded | Responding slowly or with partial errors | Yes | Route with caution; expect higher latency |
| Unreachable | Not responding to health checks | Yes | Client should handle gracefully (retry, skip, notify user) |

**Transition rules:**

- **Consecutive check requirement:** Two consecutive failures before marking `Unreachable`, two consecutive successes before marking `Healthy`. This prevents flapping on transient network issues.
- **Degraded detection:** Single slow response (>80% of timeout) or partial gRPC error triggers `Degraded` state.
- **All states are reported:** Even `Unreachable` instances remain in the registry and are included in SessionResponse. Clients decide how to handle unreachable instances.

Health state transitions are fully automatic based on health check results. No manual intervention is required unless an administrator wants to remove an instance from the registry entirely.

## Health Monitor

*Not built.* There is no polling task, no health-check client and no configuration for
either. This section describes the intended monitor, not a component that runs.

The Health Monitor periodically pings each registered instance:

- **Check interval:** Configurable (default: 60 seconds).
- **Check method:** Lightweight gRPC health check (standard `grpc.health.v1.Health` service).
- **Transition logic:** Two consecutive failures → `Unreachable`. Recovery requires two consecutive successes → back to `Healthy`. This prevents flapping on transient network issues.

Health states are reflected in the topology returned to clients. When a client re-fetches topology (new session request), it gets current health information.

## Token Issuance and Validation

*Not built as described.* The Router issues **one** token per session, not one per instance:
`generate_token` signs `{user_id, exp, iat}` with HS256 over the shared `jwt_secret`. No
instance validates it — ADR-021 made the gRPC instance loopback-only and authentication-free,
so the JWT is topology-discovery bookkeeping and nothing more. See
[ADR-019](../ADRs/019-stateless-sessions.md) and [`10-security.md`](10-security.md).

The Router issues **one token per instance** in response to each SessionRequest.

### Token Structure

Each token is:
- **Instance-specific:** Scoped to a single instance_id and cannot be used with other instances
- **Short-lived:** Default expiration is 1 hour (configurable)
- **Signed by Router:** Instances validate tokens against the Router's signing key

### Token Lifecycle Flow

```mermaid
sequenceDiagram
    participant Client
    participant Router
    participant Instance

    Client->>Router: SessionRequest (mTLS)
    Router->>Router: Generate token_A for Instance A
    Router-->>Client: SessionResponse {instances: [{..., token: "token_A"}]}
    
    Note over Client: Store token_A
    
    Client->>Instance: Assert(claim, token_A)
    Instance->>Instance: Validate token_A signature<br/>Check expiration
    Instance-->>Client: Success
    
    Note over Client: ~1 hour later, token expires
    
    Client->>Instance: Query(params, token_A)
    Instance->>Instance: Token expired
    Instance-->>Client: UNAUTHENTICATED error
    
    Client->>Router: SessionRequest (refresh)
    Router-->>Client: SessionResponse {new tokens}
```

### Security Properties

- **Token compromise is instance-scoped:** If one token leaks, only that single instance is affected. The client must re-authenticate to get fresh tokens.
- **No shared secrets between instances:** Each instance validates tokens independently using the Router's public key.
- **Instances never communicate with each other:** All trust relationships are mediated through the Router's registry and mTLS verification.

## Portable Encrypted Configuration

*Not built.* `RouterConfig::from_file` reads plaintext TOML. Whether an `age`-encrypted
portable config survives ADR-021 is an open question, tracked in
[`10-security.md`](10-security.md) and on the roadmap; it is not resolved here.

The Router's configuration — the instance registry, keypairs, and settings — is stored in a single encrypted file.

**Format:**
- **Inner layer:** TOML (human-readable when decrypted, easy to inspect and hand-edit).
- **Outer layer:** Encrypted with `age` (modern, Rust-native, passphrase-based encryption).

**Startup Flow:**

```mermaid
sequenceDiagram
    participant Admin
    participant Router
    participant Filesystem

    Admin->>Router: boswell-router --config ./router.enc
    Router->>Filesystem: Read router.enc
    Router->>Admin: Prompt for passphrase
    Admin->>Router: Enter passphrase
    Router->>Router: Decrypt in memory<br/>(never writes plaintext to disk)
    Router->>Router: Parse TOML registry
    Router->>Router: Load instance entries
    Note over Router: Router starts serving<br/>SessionRequests
```

**Properties:**

- **Never decrypted on disk:** Decryption happens in memory at startup. The plaintext never touches the filesystem.
- **Modifications are re-encrypted:** If the admin modifies the registry (adds/removes instances), the Router re-encrypts before writing.
- **Versioned config:** Contains a sequence number to detect which copy is most recent when copies exist in multiple locations.
- **Manual sync:** No automatic synchronization. The admin deliberately copies the config between storage locations (iCloud, USB drive, etc.) to prevent propagation of a compromised config.

**Disaster recovery:** With the passphrase and the encrypted config file, you can reconstruct your entire instance network from any machine running the Router binary. The actual knowledge lives on the instances; the Router config just contains the registry metadata.

## Trait Interface

```rust
pub trait Router {
    /// Authenticate a client and issue instance-specific session tokens
    fn create_session(&self, identity: &ClientIdentity) -> Result<SessionResponse, RouterError>;
    
    /// Return current instance registry with health states and endpoints
    fn get_registry(&self) -> Result<Vec<InstanceInfo>, RouterError>;
    
    /// Manually register a new instance (admin operation)
    fn register_instance(&self, entry: InstanceEntry) -> Result<(), RouterError>;
    
    /// Remove an instance from the registry (admin operation)
    fn remove_instance(&self, instance_id: &str) -> Result<(), RouterError>;
    
    /// Update an instance's endpoints or capabilities (admin operation)
    fn update_instance(&self, instance_id: &str, updates: InstanceUpdates) -> Result<(), RouterError>;
}

pub struct SessionResponse {
    pub instances: Vec<InstanceInfo>,
}

pub struct InstanceInfo {
    pub instance_id: String,
    pub endpoint: String,
    pub capabilities: Vec<String>,
    pub token: String,            // Instance-specific session token
    pub health: InstanceHealth,
}
```

Note that the Router trait does not include routing, classification, or query operations. Those are handled by the client SDK and instances directly.

## Configuration

The Router reads a plaintext TOML file named by `--config`. `--config` and `--help` are the
only flags. With neither, it warns on stderr and runs `RouterConfig::default_test_config()`
— loopback, port 8080, a hard-coded secret, one instance at `http://localhost:50051`. That
fallback exists for tests; it is not a deployment default.

### Built: the keys `RouterConfig` parses

| Key | Type | Default | Description |
|---|---|---|---|
| `bind_address` | string | *required* | Interface to bind, e.g. `127.0.0.1`. Unvalidated — unlike the gRPC instance, the Router does not refuse a routable address. |
| `bind_port` | integer | *required* | Port to bind, e.g. `8080`. |
| `jwt_secret` | string | *required* | Symmetric HS256 secret for session tokens. Rejected at load if empty. |
| `token_expiry_secs` | integer | `3600` | Lifetime of an issued session token, in seconds. |
| `instances` | array of tables | `[]` | Registered instances. An empty array parses; `POST /session/establish` then returns 500. |
| `instances[].id` | string | *required* | Instance identifier, e.g. `default`. |
| `instances[].endpoint` | string | *required* | The instance's gRPC endpoint. Exactly one — see *Multiple Endpoints*. |
| `instances[].expertise` | array of strings | `[]` | Namespaces this instance handles. Passed through to the session response; the Router never reads it. |

```toml
bind_address = "127.0.0.1"
bind_port = 8080
jwt_secret = "..."
token_expiry_secs = 3600

[[instances]]
id = "default"
endpoint = "http://localhost:50051"
expertise = ["*"]
```

The file is read once at startup. There is no reload, no write-back, and no environment-variable
override.

### Not built: what this table used to promise

These settings were specified here before the Router was written. None appears in
`RouterConfig`, and with one exception the behavior each would configure does not exist.

| Old setting | What it configured | Where it stands |
|---|---|---|
| `config_path` (`./router.enc`) | The `age`-encrypted portable config | Unbuilt; the file is plaintext TOML. Whether encryption survives ADR-021 is undecided — see [`10-security.md`](10-security.md). |
| `listen_address` (`0.0.0.0:9000`) | A gRPC listener on every interface | Wrong twice: the Router speaks HTTP, and ADR-021 puts it on loopback behind the gateway. `bind_address` and `bind_port` replace it. |
| `health_check_interval`, `health_check_timeout`, `failure_threshold`, `recovery_threshold`, `degraded_threshold` | The Health Monitor | Unbuilt. Nothing polls instances, so nothing consumes an interval or a threshold. |
| `token_ttl` (`1h`) | Session token lifetime | Built, under the name `token_expiry_secs`, same default. |
| `signing_key_path` | A private key for signing per-instance tokens | Unbuilt. Signing is HS256 over the shared `jwt_secret`, and no instance verifies the result. |

## Deployment

The Router is a single static binary with no runtime dependencies other than its config file. It runs on any machine that has network access to at least one registered instance.

### Single-Instance Mode

**The Router is present even in single-instance deployments.** This ensures consistent session management and security patterns across all deployment models.

- **Minimal overhead:** <1MB memory footprint, near-zero CPU usage
- **Simple registry:** Contains only one instance entry
- **Consistent API:** Clients use the same SessionRequest/SessionResponse flow regardless of deployment size

### Multi-Instance Mode

The Router scales efficiently to multiple instances:

- **Memory usage:** Approximately 100KB per registered instance (mostly for health state and endpoint tracking)
- **CPU usage:** Health checks every 60 seconds per instance (lightweight gRPC ping)
- **Network usage:** Minimal — only health checks and session establishment

### Recommended Deployment

**Run the Router on your primary trusted machine** (not a remote server). The Router holds:
- The instance registry and cryptographic identities
- The token-issuing signing key
- The encrypted configuration (highest-value target in the system)

Running it on your own machine (desktop, laptop) gives you direct control over the most security-sensitive component.

### Resource Requirements

- **Memory:** <1MB for single instance, ~1MB + (100KB × number of instances)
- **CPU:** Near-zero except during session establishment and health checks
- **Disk:** Only the config file (~10-50KB depending on registry size)
- **Network:** Outbound connections to instances for health checks; inbound gRPC listener for session requests

The Router holds **no claim data** — only metadata about instances. Memory and CPU usage are independent of the number of claims in your system.
