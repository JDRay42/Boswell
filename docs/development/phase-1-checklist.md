# Phase 1: Foundation - Implementation Checklist

## Overview
Establish domain core, storage layer, and LLM integration without external API surface.

**Status:** 🟡 In Progress  
**Contributors:** 2 parallel streams

---

## Contributor A: Domain Core (`boswell-domain`)

### ✅ Completed
- [x] Create crate with zero external dependencies
- [x] Implement basic value objects (ClaimId, ConfidenceInterval, Namespace, ProvenanceEntry, Relationship, Tier)
- [x] Define trait interfaces (ClaimStore, LlmProvider, Extractor)
- [x] Add initial unit tests (10 tests passing)
- [x] XML documentation for public types

### 🔲 Remaining Tasks

#### ULID Integration
- [ ] Add `ulid` crate dependency
- [ ] Update `ClaimId` to use proper ULID generation
- [ ] Add `ClaimId::new()` for generating new IDs
- [ ] Add `ClaimId::from_string()` for parsing ULID strings
- [ ] Format `ClaimId` Display as proper ULID (not hex)
- [ ] Add property tests for ULID chronological ordering

#### Confidence Formula Implementation (ADR-007)
- [ ] Create `confidence_computation.rs` module
- [ ] Implement recursive support network traversal
- [ ] Add source diversity weighting logic
- [ ] Add circular relationship detection and prevention
- [ ] Define `BOOST_FACTOR` and `PENALTY_FACTOR` constants
- [ ] Implement caching strategy for computed confidence
- [ ] Add comprehensive unit tests for formula edge cases

#### Property-Based Tests
- [ ] Add `proptest` tests for confidence interval invariants
- [ ] Add `proptest` tests for namespace depth validation
- [ ] Add `proptest` tests for relationship strength bounds
- [ ] Add `proptest` tests for confidence formula convergence
- [ ] Verify ULID ordering properties hold under all conditions

#### Documentation
- [ ] Ensure 100% Rustdoc coverage (run `cargo doc --no-deps`)
- [ ] Add module-level documentation for all modules
- [ ] Add usage examples in docs for key types
- [ ] Document confidence formula with examples

**Deliverable:** `boswell-domain` crate compiles with 100% doc coverage, passes property tests

---

## Contributor B: Storage Layer (`boswell-store`)

### 🔲 All Tasks Remaining

#### SQLite Schema Design
- [ ] Design and document complete SQLite schema
  - [ ] `claims` table with ULID primary key
  - [ ] `relationships` table (pairwise only, per ADR-002)
  - [ ] `provenance` table
  - [ ] `confidence_cache` table for fast-path values
  - [ ] Indexes for common query patterns
- [ ] Create schema SQL file in `src/schema.sql`

#### Database Implementation
- [ ] Add `rusqlite` dependency with "bundled" feature
- [ ] Create `SqliteStore` struct implementing `ClaimStore` trait
- [ ] Implement connection pooling or thread-local storage
- [ ] Add migration framework using `rusqlite` migrations
- [ ] Implement `assert_claim()` with duplicate detection
- [ ] Implement `get_claim()` for direct retrieval
- [ ] Implement `query_claims()` with structured filters
- [ ] Implement temporal queries via ULID range scans (ADR-011)
- [ ] Implement `add_relationship()` and `get_relationships()`

#### HNSW Vector Index (ADR-005)
- [ ] Research and select HNSW library (`hnsw_rs` or alternative)
- [ ] Add chosen library as dependency
- [ ] Create `VectorIndex` wrapper struct
- [ ] Implement memory-mapped index storage
- [ ] Add separate index file alongside SQLite database
- [ ] Implement index rebuild functionality
- [ ] Document HNSW parameters (M, efConstruction)

#### Embedding Pipeline (ADR-013)
- [ ] Add ONNX runtime dependency (`ort` or `tract`)
- [ ] Download `bge-small-en-v1.5` model (384 dims)
- [ ] Create `EmbeddingModel` struct
- [ ] Implement text → vector conversion
- [ ] Add model loading from filesystem
- [ ] Document embedding dimension configuration
- [ ] Handle model loading errors gracefully

#### Duplicate Detection
- [ ] Implement cosine similarity calculation
- [ ] Define and document similarity threshold (default: 0.95?)
- [ ] Implement pre-insert duplicate check
- [ ] Add tests for duplicate detection edge cases

#### Integration Tests
- [ ] Create in-memory SQLite database for testing
- [ ] Test full claim CRUD cycle
- [ ] Test relationship storage and retrieval
- [ ] Test duplicate detection with near-identical claims
- [ ] Test vector search returns semantically similar results
- [ ] Test temporal queries using ULID ordering
- [ ] Benchmark insertion performance

**Deliverable:** `boswell-store` crate with full CRUD operations, vector search, and passing integration tests

---

## Phase 1 Sync Point: LLM Provider Layer (`boswell-llm`)

### 🔲 All Tasks Remaining (Both contributors collaborate)

#### Provider Trait Implementation
- [ ] Define complete `LlmProvider` trait with error types
- [ ] Add configuration structure for per-subsystem providers (ADR-015)
- [ ] Document trait methods and expected behavior

#### Mock Provider
- [ ] Create `MockProvider` for deterministic testing
- [ ] Implement configurable responses
- [ ] Add delay simulation for realistic testing
- [ ] Add error injection for failure testing

#### Ollama Provider
- [ ] Add `reqwest` dependency for HTTP client
- [ ] Create `OllamaProvider` struct
- [ ] Implement connection to local Ollama API
- [ ] Handle streaming response parsing
- [ ] Add retry logic with exponential backoff
- [ ] Add timeout configuration
- [ ] Document required Ollama models (qwen2.5:7b, llama3.2:3b)

#### Testing
- [ ] Unit tests with `MockProvider`
- [ ] Integration tests with Ollama (conditional on availability)
- [ ] Test error handling and retries
- [ ] Document testing requirements in README

**Deliverable:** `boswell-llm` crate with mock and Ollama providers

---

## Phase 1 Validation Criteria

**All criteria must pass before proceeding to Phase 2:**

- [ ] Domain core compiles with zero warnings
- [ ] All property tests pass (confidence formula, ULID ordering, namespace validation)
- [ ] Store can assert and query claims with confidence computation
- [ ] Embedding pipeline produces consistent vectors
- [ ] Vector search returns semantically similar claims
- [ ] Ollama provider successfully calls local LLM
- [ ] Full test suite runs in <10 seconds
- [ ] `cargo clippy` passes with no warnings
- [ ] `cargo doc --no-deps` generates complete documentation
- [ ] All integration tests pass

---

## Getting Started (Next Session)

### Quick Commands

```bash
# Check current status
cargo check

# Run all tests
cargo test

# Run tests with output
cargo test -- --nocapture

# Run specific crate tests
cargo test -p boswell-domain

# Watch mode (auto-rebuild)
cargo watch -x test

# Lint
cargo clippy

# Generate documentation
cargo doc --no-deps --open
```

### Recommended Starting Point

**Contributor A:** Start with ULID integration in `boswell-domain/src/claim.rs`
**Contributor B:** Start with SQLite schema design in `boswell-store/src/schema.sql`

### Resources

- [ULID crate docs](https://docs.rs/ulid)
- [rusqlite docs](https://docs.rs/rusqlite)
- [ONNX Runtime docs](https://docs.rs/ort)
- ADR-005: SQLite + HNSW architecture
- ADR-007: Hybrid confidence computation
- ADR-011: ULID over UUID
- ADR-013: Local embedding models

---

**Last Updated:** February 14, 2026  
**Next Review:** After Phase 1 completion
