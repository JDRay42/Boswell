# Phase 2: Core Operations - Implementation Checklist

## Overview
Expose gRPC API for basic operations, enable client interactions through Router and SDK.

**Status:** 🟡 In Progress  
**Start Date:** February 14, 2026  
**Contributors:** 3 parallel streams

---

## Stream A: gRPC Service Layer (`boswell-grpc`)

### ✅ Protocol Definitions
- [x] Create `.proto` files for API surface ✅ DONE
  - [x] `AssertRequest/Response` with optional tier targeting
  - [x] `QueryRequest/Response` with fast/deliberate mode flag
  - [x] `LearnRequest/Response` for bulk loading (ADR-012)
  - [x] `ForgetRequest/Response` for eviction marking
  - [x] `HealthCheckRequest/Response`
- [x] Add `tonic` and `tonic-build` dependencies ✅ DONE
- [x] Configure build.rs for proto code generation ✅ DONE
- [x] Generate Rust code and verify compilation ✅ DONE

### ✅ Service Implementation  
- [x] Implement `BosWellService` trait ✅ DONE
- [x] Add service handlers: ✅ DONE
  - [x] `assert()` - Route to ClaimStore, record provenance
  - [x] `query()` - Handle fast/deliberate modes (basic implementation)
  - [x] `learn()` - Batch claim insertion
  - [x] `forget()` - Mark claims for eviction (stub)
  - [x] `health_check()` - Return instance status
- [x] Implement error mapping (domain errors → gRPC status codes) ✅ DONE
- [x] Add authentication token validation (placeholder) ✅ DONE
- [ ] Apply tier-based validation per ADR ⏸️ Deferred

### ✅ Server Infrastructure
- [x] Server initialization with TLS support (ADR-017) ✅ DONE (placeholder)
- [x] Configuration loading (endpoint, port, TLS certs) ✅ DONE
- [ ] Graceful shutdown handling ⏸️ Deferred
- [ ] Logging and metrics instrumentation ⏸️ Deferred
- [x] Integration tests with test client ✅ DONE (8 tests passing)

**Deliverable:** ✅ `boswell-grpc` crate with functional gRPC server (basic implementation complete)

---

## Stream B: Router (`boswell-router`)

### 🔲 Session Management (ADR-019)
- [ ] Implement session token generation (JWT or signed tokens)
- [ ] Stateless session validation
- [ ] Token expiry handling
- [ ] Token refresh mechanism

### 🔲 Instance Registry
- [ ] Single-instance mode (self-registration)
- [ ] Health status tracking (stub for Phase 2)
- [ ] Encrypted configuration storage
- [ ] Instance endpoint resolution

### 🔲 Router Service
- [ ] Session establishment endpoint
  - [ ] Accept domain hints from clients
  - [ ] Return instance endpoints and tokens
- [ ] Health check aggregation
- [ ] Configuration file parsing (TOML)
- [ ] CLI bootstrapping command (initial instance setup)

### 🔲 Infrastructure
- [ ] HTTP server setup (lightweight, not gRPC)
- [ ] TLS configuration
- [ ] Logging and error handling
- [ ] Integration tests

**Deliverable:** `boswell-router` crate with single-instance session management

---

## Stream C: Client SDK (`boswell-sdk`)

### 🔲 Core SDK Implementation
- [ ] Create Rust SDK wrapping gRPC calls
- [ ] Session establishment via Router
- [ ] Direct instance communication after session
- [ ] Automatic token inclusion in requests
- [ ] Connection pooling for gRPC channels

### 🔲 API Methods
- [ ] `assert()` - Assert claim with optional tier
- [ ] `query()` - Query claims with filters
- [ ] `learn()` - Bulk claim insertion
- [ ] `forget()` - Mark claim for eviction
- [ ] Synchronous interface (async deferred to Phase 3)

### 🔲 Infrastructure
- [ ] Error handling with typed errors
- [ ] Configuration (router endpoint, timeouts)
- [ ] Retry logic with exponential backoff
- [ ] Example code for common operations
- [ ] Integration tests against running server

**Deliverable:** `boswell-sdk` crate with full API coverage

---

## Phase 2 Validation Criteria

**All criteria must pass before proceeding to Phase 3:**

- [ ] gRPC server starts and responds to health checks
- [ ] Router issues valid session tokens
- [ ] SDK successfully establishes session and routes to instance
- [ ] End-to-end test: SDK → Router → gRPC → Store → Response
- [ ] Claims can be asserted and queried via SDK
- [ ] Bulk `learn()` operation works with batches of pre-formatted claims
- [ ] TLS connections succeed with test certificates
- [ ] All tests pass: `cargo test --workspace`
- [ ] Zero clippy warnings: `cargo clippy --workspace -- -D warnings`
- [ ] 100% documentation coverage for public APIs

---

## Quick Commands

```bash
# Run all tests
cargo test --workspace

# Run specific crate tests
cargo test -p boswell-grpc
cargo test -p boswell-router
cargo test -p boswell-sdk

# Build proto files
cargo build -p boswell-grpc

# Start gRPC server (manual testing)
cargo run -p boswell-grpc

# Start Router (manual testing)
cargo run -p boswell-router

# Lint
cargo clippy --workspace -- -D warnings

# Generate documentation
cargo doc --no-deps --open
```

---

**Last Updated:** February 14, 2026  
**Next Review:** After Phase 2 completion
