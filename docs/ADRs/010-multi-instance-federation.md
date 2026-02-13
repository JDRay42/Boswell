# ADR-010: Multi-Instance Federation with Manual Trust, Automatic Liveness

## Status

Accepted

## Context

Boswell is designed as a long-lived personal knowledge base. Users may want to isolate domains (development, personal, health, finance) into separate instances for performance, security, and operational independence. These instances may run on different machines (laptop, Mac Mini, VPS).

The question is how instances discover and trust each other.

## Decision

**Registration is manual and deliberate. Discovery of registered instances is automatic and resilient. Identity is cryptographic.**

- Adding a new instance is an explicit administrative action: generate keypair, register fingerprint with Router.
- No automatic discovery of unknown instances. The Router will never trust an instance it hasn't been explicitly told about.
- The Router continuously health-checks all registered instances and tracks availability (healthy, degraded, unreachable, untrusted).
- mTLS ensures mutual identity verification on every connection.

## Consequences

- A compromised or rogue instance cannot be silently added to the trust network.
- Network disruptions are handled gracefully. The Router operates with whatever instances are reachable and re-integrates instances when they become available. Agents see partial results with transparency ("3 of 5 instances unreachable").
- Exponential backoff on unreachable instances prevents hammering dead endpoints (ceiling ~5 minutes).
- Each instance carries a trust score (0.0-1.0, default 1.0) that can be degraded through a mechanism to be determined. Claims from lower-trust instances have their effective confidence scaled accordingly.
- Instances have scope permissions: federated query participation, cross-domain synthesis participation, or direct-access only. A health memory instance might be fully isolated.
- Endpoint flexibility: an instance can be registered with multiple endpoints (LAN IP, VPN address) for different network contexts.
- The Router's encrypted, portable config file enables spinning up a Router from any location with access to the config and passphrase.
