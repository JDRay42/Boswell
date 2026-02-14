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
- [x] Add `ulid` crate dependency
- [x] Update `ClaimId` to use proper ULID generation
- [x] Add `ClaimId::new()` for generating new IDs
- [x] Add `ClaimId::from_string()` for parsing ULID strings
- [x] Format `ClaimId` Display as proper ULID (not hex)
- [x] Add property tests for ULID chronological ordering
- [x] Create `confidence_computation.rs` module
- [x] Implement recursive support network traversal
- [x] Add source diversity weighting logic
- [x] Add circular relationship detection and prevention (via stale confidence only)
- [x] Define `BOOST_FACTOR` and `PENALTY_FACTOR` constants
- [x] Implement caching strategy for computed confidence
- [x] Add comprehensive unit tests for formula edge cases
- [x] Add `proptest` tests for confidence interval invariants
- [x] Add `proptest` tests for namespace depth validation
- [x] Add `proptest` tests for relationship strength bounds
- [x] Add `proptest` tests for confidence formula convergence
- [x] Verify ULID ordering properties hold under all conditions
- [x] Ensure 100% Rustdoc coverage (run `cargo doc --no-deps`)
- [x] Add module-level documentation for all modules
- [x] Add usage examples in docs for key types
- [x] Document confidence formula with examples
- [x] All clippy warnings resolved

### 🔲 Remaining Tasks

#### None for Contributor A - Domain Core Complete! ✨

**Deliverable:** `boswell-domain` crate compiles with 100% doc coverage, passes property tests

---

## Contributor B: Storage Layer (`boswell-store`)

### ✅ Completed
- [x] Design and document complete SQLite schema
  - [x] `claims` table with ULID primary key
  - [x] `relationships` table (pairwise only, per ADR-002)
  - [x] `provenance` table
  - [x] `confidence_cache` table for fast-path values
  - [x] Indexes for common query patterns
- [x] Create schema SQL file in `src/schema.sql`
- [x] Add `rusqlite` dependency with "bundled" feature
- [x] Create `SqliteStore` struct implementing `ClaimStore` trait
- [x] Implement connection pooling or thread-local storage
- [x] Implement `assert_claim()` with duplicate detection
- [x] Implement `get_claim()` for direct retrieval
- [x] Implement `query_claims()` with structured filters
- [x] Implement `add_relationship()` and `get_relationships()`
- [x] Add comprehensive integration tests (12 tests passing)
- [x] Test full claim CRUD cycle
- [x] Test relationship storage and retrieval
- [x] Test temporal queries using ULID ordering

### 🔲 Remaining Tasks

#### Database Implementation
- [x] ~~Add `rusqlite` dependency with "bundled" feature~~ DONE
- [x] ~~Create `SqliteStore` struct implementing `ClaimStore` trait~~ DONE
- [x] ~~Implement connection pooling or thread-local storage~~ DONE
- [x] ~~Add migration framework using `rusqlite` migrations~~ ✅ Schema versioning table added
- [x] ~~Implement `assert_claim()` with duplicate detection~~ DONE
- [x] ~~Implement `get_claim()` for direct retrieval~~ DONE
- [x] ~~Implement `query_claims()` with structured filters~~ DONE
- [x] ~~Implement temporal queries via ULID range scans (ADR-011)~~ ✅ Implemented via query_claims
- [x] ~~Implement `add_relationship()` and `get_relationships()`~~ DONE

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
- [x] ~~Create in-memory SQLite database for testing~~ DONE
- [x] ~~Test full claim CRUD cycle~~ DONE
- [x] ~~Test relationship storage and retrieval~~ DONE
- [ ] Test duplicate detection with near-identical claims (requires embedding)
- [ ] Test vector search returns semantically similar results (requires HNSW)
- [x] ~~Test temporal queries using ULID ordering~~ DONE
- [ ] Benchmark insertion performance

**Deliverable:** `boswell-store` crate with full CRUD operations ✅ **DONE**, vector search pending

---

## Phase 1 Sync Point: LLM Provider Layer (`boswell-llm`)

### ✅ Completed Tasks

#### Provider Trait Implementation
- [x] Define complete `LlmProvider` trait with error types
- [x] Add configuration structure for per-subsystem providers (ADR-015)
- [x] Document trait methods and expected behavior

#### Mock Provider
- [x] Create `MockProvider` for deterministic testing
- [x] Implement configurable responses
- [x] Add delay simulation for realistic testing (via call count tracking)
- [x] Add error injection for failure testing

#### Testing
- [x] Unit tests with `MockProvider` (6 tests passing)
- [x] Test error handling and retries
- [x] Document testing requirements in README

### 🔲 Remaining Tasks (Both contributors collaborate)

#### Ollama Provider
- [ ] Add `reqwest` dependency for HTTP client
- [ ] Create `OllamaProvider` struct
- [ ] Implement connection to local Ollama API
- [ ] Handle streaming response parsing
- [ ] Add retry logic with exponential backoff
- [ ] Add timeout configuration
- [ ] Document required Ollama models (qwen2.5:7b, llama3.2:3b)
- [ ] Integration tests with Ollama (conditional on availability)

**Deliverable:** `boswell-llm` crate with mock ✅ **DONE** and Ollama providers (pending)

---

## Phase 1 Validation Criteria

**All criteria must pass before proceeding to Phase 2:**

- [x] ~~Domain core compiles with zero warnings~~ ✅ DONE
- [x] ~~All property tests pass (confidence formula, ULID ordering, namespace validation)~~ ✅ DONE
- [x] ~~Store can assert and query claims with confidence computation~~ ✅ DONE
- [ ] Embedding pipeline produces consistent vectors
- [ ] Vector search returns semantically similar claims
- [ ] Ollama provider successfully calls local LLM
- [x] ~~Full test suite runs in <10 seconds~~ ✅ DONE (0.03s for store, 0.00s for llm)
- [x] ~~`cargo clippy` passes with no warnings~~ ✅ DONE (domain, store, llm)
- [x] ~~`cargo doc --no-deps` generates complete documentation~~ ✅ DONE
- [x] ~~All integration tests pass~~ ✅ DONE (18 tests passing: 10 domain, 6 llm, 12 store)

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
