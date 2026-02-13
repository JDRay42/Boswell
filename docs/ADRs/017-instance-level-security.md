# ADR-017: Instance-Level Security Independent of Router

## Status

Accepted

## Context

The initial architecture framed security primarily in the context of the Router — mTLS between Router and instances, token issuance, manual registration. This implied that single-instance deployments (without a Router) had a weaker or undefined security posture. It also left a gap in how the MCP server authenticates to instances.

A Boswell instance is a network service that may run on a separate machine from the agents consuming it. Without authentication, any process on the network could read or write to the memory store.

## Decision

**Security is enforced at the instance level, not the Router level.** Every Boswell instance requires mTLS on every inbound connection, regardless of deployment mode. Any client — MCP server, Router, direct SDK call — must present a client certificate registered with the instance. There are no unauthenticated access modes.

The Router adds topology management and federation, but it is not the security boundary. Each instance is its own security boundary.

## Consequences

- Single-instance deployments are fully secured. No Router needed for authentication.
- The MCP server holds its own keypair and registers its fingerprint with each instance (or with the Router in multi-instance mode). It is a trusted client, not an anonymous proxy.
- Client registration uses the same manual trust process regardless of client type: generate keypair, register fingerprint, authenticate via mTLS.
- Every trust relationship is explicit, manually established, and independently revocable. Compromising one client doesn't compromise others.
- A rogue agent on the network without a registered certificate is rejected at the TLS handshake. No application-level authentication bypass is possible.
- The Router's security role is additive (token issuance, federated query coordination) rather than foundational. Removing the Router from a deployment doesn't remove security.
