# Plan: Boswell Cognitive Memory System - Development Roadmap

Boswell is a claim-based cognitive memory system for AI agents, built in Rust with Clean Architecture principles. This plan delivers a functional single-instance system incrementally, with 11+ components organized into 5 phases. Each phase has clear deliverables, validation criteria, and parallelizable work streams for 2-3 subagent contributors.

**Critical Decisions:**
- Single-instance MVP first; federation deferred to future phases
- Real LLM providers (Ollama) from Phase 1 for authentic testing
- Synthesizer deferred to Phase 4+ (non-critical to core operations)
- Conservative 2-3 contributor parallelization minimizes coordination overhead

---

## PHASE 1: FOUNDATION

**Goal:** Establish domain core, storage layer, and LLM integration without external API surface

**Contributors Assigned:** 2 parallel streams

### Contributor A: Domain Core (`boswell-domain`)

1. Create crate with zero external dependencies (per ADR-004)
2. Implement core value objects:
   - `ClaimId` (ULID-based, per ADR-011)
   - `ConfidenceInterval` with `[lower, upper]` bounds (ADR-003)
   - `ProvenanceEntry` with source/timestamp/rationale
   - `Relationship` with pairwise constraint (ADR-002)
   - `Namespace` with slash-delimited validation (ADR-006)
3. Define `Claim` struct with all fields from [02-claim-model.md](../architecture/02-claim-model.md)
4. Define trait interfaces:
   - `ClaimStore` trait
   - `LlmProvider` trait with capability methods (ADR-015)
   - `Extractor`, `Synthesizer`, `Gatekeeper` traits
5. Implement deterministic confidence formula (ADR-007):
   - Recursive support network traversal
   - Source diversity weighting
   - Circular relationship protection
6. Add comprehensive property-based tests using `proptest`:
   - ULID ordering properties
   - Confidence interval invariants
   - Namespace depth validation
   - Formula convergence properties
7. XML documentation for all public types and methods

**Deliverable:** `boswell-domain` crate compiles with 100% doc coverage, passes property tests

---

### Contributor B: Storage Layer (`boswell-store`)

1. Create SQLite schema (fill documentation gap):
   - `claims` table with ULID primary key
   - `relationships` table (pairwise only)
   - `provenance` table
   - `confidence_cache` table for fast-path values
2. Implement `ClaimStore` trait using `rusqlite`:
   - `assert_claim()` with duplicate detection
   - `query_claims()` with structured filters
   - `get_by_id()` for direct retrieval
   - Temporal queries via ULID range scans (ADR-011)
3. Integrate HNSW vector index (ADR-005):
   - Use `hnswlib-rs` or equivalent
   - Memory-mapped index for performance
   - Separate index file alongside SQLite database
4. Implement local embedding pipeline (ADR-013):
   - ONNX runtime integration with `tract` or `ort`
   - Ship with `bge-small-en-v1.5` model (384 dims)
   - Document dimension configuration
5. Implement embedding-based duplicate detection:
   - Cosine similarity threshold (document default)
   - Pre-insert check against existing claims
6. Add migration framework using `rusqlite` migrations
7. Integration tests with in-memory SQLite database

**Deliverable:** `boswell-store` crate with full CRUD operations, vector search, and passing integration tests

---

### Phase 1 Sync Point: LLM Provider Layer (`boswell-llm`)

Both contributors collaborate once their streams complete:

1. Define `LlmProvider` trait implementation structure
2. Create `MockProvider` for deterministic testing
3. Implement `OllamaProvider` (local, no API keys needed):
   - HTTP client for Ollama API
   - Streaming response handling
   - Error handling and retries
4. Configuration structure for per-subsystem providers (ADR-015)
5. Unit tests with mock, integration tests with Ollama (conditional on availability)

**Deliverable:** `boswell-llm` crate with mock and Ollama providers

---

**Phase 1 Validation:**
- [ ] Domain core compiles with zero warnings
- [ ] All property tests pass (confidence formula, ULID ordering, namespace validation)
- [ ] Store can assert and query claims with confidence computation
- [ ] Embedding pipeline produces consistent vectors
- [ ] Vector search returns semantically similar claims
- [ ] Ollama provider successfully calls local LLM
- [ ] Full test suite runs in <10 seconds

---

## PHASE 2: CORE OPERATIONS

**Goal:** Expose gRPC API for basic operations, enable client interactions

**Contributors Assigned:** 3 parallel streams

### Contributor A: gRPC Service Layer (`boswell-grpc`)

1. Define `.proto` files for API surface (fill documentation gap):
   - `AssertRequest/Response` with optional tier targeting
   - `QueryRequest/Response` with fast/deliberate mode flag
   - `LearnRequest/Response` for bulk loading (ADR-012)
   - `ForgetRequest/Response` for eviction marking
   - `HealthCheckRequest/Response`
2. Generate Rust code using `tonic-build`
3. Implement service handlers:
   - Route to `ClaimStore` for storage operations
   - Handle provenance recording
   - Apply tier-based validation
4. Add authentication stubs (token validation placeholder)
5. Error mapping from domain errors to gRPC status codes
6. Server initialization with TLS configuration (ADR-017)

**Deliverable:** `boswell-grpc` crate with functional gRPC server

---

### Contributor B: Router (`boswell-router`)

1. Implement session management (ADR-019):
   - Session token generation (signed JWT or similar)
   - Stateless session validation
   - Token expiry handling
2. Create instance registry:
   - Single-instance mode (self-registration)
   - Health status tracking (stub for now)
   - Encrypted configuration storage
3. Implement session establishment endpoint:
   - Accept domain hints from clients
   - Return instance endpoints and tokens
4. Add health check aggregation
5. Configuration file parsing (TOML) - fill documentation gap
6. CLI bootstrapping command (initial instance setup)

**Deliverable:** `boswell-router` crate with single-instance session management

---

### Contributor C: Client SDK (`boswell-sdk`)

1. Create Rust SDK wrapping gRPC calls:
   - Session establishment via Router
   - Direct instance communication after session
   - Automatic token inclusion in requests
2. Implement API methods:
   - `assert()`, `query()`, `learn()`, `forget()`
   - Synchronous interface (async can come later)
3. Error handling with typed errors
4. Connection pooling for gRPC channels
5. Example code for common operations
6. Integration tests against running server

**Deliverable:** `boswell-sdk` crate with full API coverage

---

**Phase 2 Validation:**
- [ ] gRPC server starts and responds to health checks
- [ ] Router issues valid session tokens
- [ ] SDK successfully establishes session and routes to instance
- [ ] End-to-end test: SDK → Router → gRPC → Store → Response
- [ ] Claims can be asserted and queried via SDK
- [ ] Bulk `learn()` operation works with batches of pre-formatted claims
- [ ] TLS connections succeed with test certificates

---

## PHASE 3: INTELLIGENT OPERATIONS

**Goal:** Add LLM-backed operations (Extractor, Gatekeeper)

**Contributors Assigned:** 2 parallel streams

### Contributor A: Extractor (`boswell-extractor`)

1. Design LLM prompts for text → claims conversion (fill documentation gap):
   - System prompt defining claim structure
   - Examples of good claim extraction
   - Instructions for splitting compound statements
2. Implement `Extractor` trait:
   - Accept unstructured text blocks
   - Call LLM provider with prompt
   - Parse LLM response into `Claim` structs
   - Handle malformed responses gracefully
3. Integrate with `ClaimStore` duplicate detection:
   - Check embeddings before insertion
   - Decide merge vs. new claim policy
4. Record provenance linking to source text
5. Add Extract operation to gRPC API:
   - `ExtractRequest` with text and optional namespace/tier
   - Synchronous blocking operation
6. Unit tests with mock LLM, integration tests with Ollama
7. Measure performance: claims extracted per second

**Deliverable:** `boswell-extractor` crate with gRPC integration

---

### Contributor B: Gatekeeper (`boswell-gatekeeper`)

1. Define tier promotion evaluation prompts (fill documentation gap):
   - Per-tier criteria (ephemeral→task, task→project, project→permanent)
   - Advocacy tuple structure
   - Expected response format (accept/downgrade/reject + rationale)
2. Implement `Gatekeeper` trait:
   - Accept promotion requests with advocacy tuples
   - Call LLM with claim content + relationships + tier target
   - Parse decision and reasoning
3. Record gatekeeper reasoning as provenance (ADR-008)
4. Implement promotion request queue (deferred evaluation):
   - Background thread polling queue
   - Rate limiting to prevent LLM overload
5. Add Promote operation to gRPC API:
   - `PromoteRequest` with claim ID and target tier
   - Async response model (request accepted, decision later)
6. Create tier-specific configuration:
   - Different LLM models per tier boundary
   - Configurable evaluation criteria
7. Integration tests covering accept/downgrade/reject paths

**Deliverable:** `boswell-gatekeeper` crate with gRPC integration

---

**Phase 3 Validation:**
- [ ] Extractor converts sample text into structured claims
- [ ] Extracted claims have proper provenance linking to source
- [ ] Duplicate detection prevents redundant storage
- [ ] Gatekeeper evaluates promotion requests within 5 seconds (with Ollama)
- [ ] Gatekeeper reasoning is stored and retrievable
- [ ] Promotion rejections preserve claims in original tier
- [ ] End-to-end test: Extract text → Assert to ephemeral → Promote to task → Gatekeeper accepts

---

## PHASE 4: BACKGROUND PROCESSES

**Goal:** Add automated maintenance (Janitors) and optional synthesis

**Contributors Assigned:** 2-3 parallel streams

### Contributor A: Core Janitors (`boswell-janitor`)

1. Create janitor framework:
   - Background thread with configurable interval
   - Graceful shutdown handling
   - Per-janitor enable/disable configuration
2. Implement **Staleness Janitor**:
   - Apply confidence decay based on half-life model (ADR-009)
   - Update `confidence_cache` table
   - No LLM required (deterministic)
3. Implement **GC Janitor**:
   - Query forgotten claims past retention period
   - Hard delete from SQLite and HNSW index
   - Batch deletions for efficiency
4. Implement **Confidence Recomputation Janitor**:
   - Identify claims with stale cached confidence
   - Recompute via support network traversal
   - Update cache
5. Integration with gRPC server: janitors start with server
6. Observability: log janitor runs and metrics (claims processed, deleted)

**Deliverable:** `boswell-janitor` crate with 3 core janitors operational

---

### Contributor B (Optional): Advanced Janitors

1. Implement **Contradiction Janitor** (high complexity):
   - Query pairs of claims with overlapping namespaces/subjects
   - Call LLM for semantic contradiction detection
   - Record Challenge relationships
   - Rate-limited to prevent LLM overload
2. Implement **Tier Migration Janitor**:
   - Identify demotion candidates (low confidence, no recent access)
   - Downgrade tiers automatically or queue for review
3. Configuration for janitor aggressiveness
4. Integration tests with mock LLM for contradiction detection

**Deliverable:** `boswell-janitor` crate with 5 janitor types

---

### Contributor C (Phase 4b): Synthesizer (`boswell-synthesizer`)

*Deferred to Phase 4b - start only after Phase 4a validation*

1. Design synthesis prompts (fill documentation gap):
   - Instruct LLM to identify patterns across claim clusters
   - Generate higher-order derived claims
   - Specify `derived_from` relationship format
2. Implement `Synthesizer` trait:
   - Cluster claims by namespace/semantic similarity
   - Sample clusters for LLM analysis
   - Parse emergent insights into new claims
3. Create derived claims with proper provenance:
   - `derived_from` relationships pointing to source claims
   - Confidence inherited/computed from sources
4. Background scheduling (configurable, not triggered by API)
5. Prevent synthesis loops:
   - Max derivation depth
   - Skip already-synthesized clusters
6. Integration tests with small claim sets

**Deliverable:** `boswell-synthesizer` crate with scheduled synthesis

---

**Phase 4 Validation:**
- [ ] Staleness janitor reduces confidence over time according to half-life
- [ ] GC janitor deletes forgotten claims after retention period
- [ ] Confidence recomputation janitor keeps cache accurate
- [ ] Contradiction janitor identifies known contradictory claim pairs
- [ ] Tier migration janitor demotes stale low-confidence claims
- [ ] Synthesizer produces at least one derived claim from test data
- [ ] Janitors run on schedule without blocking gRPC operations
- [ ] System remains responsive under janitor load

---

## PHASE 5: CLIENT INTEGRATION & POLISH

**Goal:** Expose via MCP, add CLI tooling, optimize performance

**Contributors Assigned:** 2 parallel streams

### Contributor A: MCP Server (`boswell-mcp`)

1. Implement MCP protocol server:
   - Expose all operations as MCP tools
   - Tool schemas for each operation (Assert, Query, Extract, Learn, Promote, Forget, Reflect)
   - Session management wrapper using SDK
2. Create tool definitions:
   - Clear descriptions for LLM consumption
   - Parameter schemas with validation
   - Example invocations
3. Add Reflect operation:
   - LLM-backed narrative synthesis from query results
   - Configurable depth/breadth tradeoffs
4. Error handling and user-friendly messages
5. Integration with Claude Desktop, Cursor, etc.
6. Documentation for MCP setup

**Deliverable:** `boswell-mcp` crate with full MCP server implementation

---

### Contributor B: CLI & Operations Tooling (`boswell-cli`)

1. Create admin commands:
   - `boswell init` - bootstrap new instance
   - `boswell serve` - start server with config
   - `boswell reindex` - rebuild HNSW index (ADR-014)
   - `boswell health` - instance health check
   - `boswell backup` / `boswell restore` - SQLite operations
2. Interactive claim exploration:
   - `boswell query` - query with filters
   - `boswell inspect <claim-id>` - view claim details + relationships
   - `boswell graph <claim-id>` - visualize support network
3. Bulk operations:
   - `boswell import` - load from JSON/CSV
   - `boswell export` - dump claims
4. Configuration validation:
   - `boswell config check` - validate TOML
5. CLI help text and man pages
6. Shell completion scripts (bash/zsh)

**Deliverable:** `boswell-cli` crate with comprehensive admin tooling

---

### Contributor C (Optimization - ongoing):

1. Performance benchmarking:
   - Assert throughput (claims/sec)
   - Query latency (p50/p95/p99)
   - Vector search recall@k
   - Confidence computation time
2. Identify and optimize hot paths:
   - Profile with `cargo flamegraph`
   - Optimize SQL queries (EXPLAIN ANALYZE)
   - Tune HNSW parameters (M, efConstruction)
   - Cache frequently-accessed claims
3. Add observability:
   - Structured logging with `tracing`
   - Prometheus metrics export
   - Trace sampling for distributed tracing
4. Memory profiling and leak detection
5. Document performance characteristics and tuning guide

**Deliverable:** Performance report and optimization recommendations

---

**Phase 5 Validation:**
- [ ] MCP server connects to Claude Desktop and exposes all tools
- [ ] CLI can bootstrap instance, assert claims, query, and inspect
- [ ] Backup/restore preserves all data including vector index
- [ ] Reindex operation successfully rebuilds HNSW index
- [ ] Performance benchmarks meet targets from [01-architecture.md](../architecture/01-architecture.md)
- [ ] System handles 1000+ claims without degradation
- [ ] Metrics exported and viewable in Prometheus

---

## CROSS-PHASE REQUIREMENTS

All contributors must adhere to:

### 1. Testing Standards

- Unit tests for all business logic
- Integration tests for component boundaries
- Property-based tests for invariants
- BDD/Gherkin tests for user-facing operations (using `cucumber-rust`)
- Minimum 80% code coverage

### 2. Documentation Standards

- Rustdoc comments for all public items
- Module-level documentation explaining purpose
- Examples in docs for common operations
- Architecture decision updates when deviating from ADRs

### 3. Code Quality

- No file exceeds 300 lines (refactor if needed)
- All `clippy` lints pass at `warn` level
- Run `cargo fmt` before commits
- No `unwrap()` or `panic!()` in production code paths

### 4. Security

- Never commit secrets or test certificates to git
- Use environment variables for sensitive config
- Validate all inputs at API boundaries
- Follow Rust memory safety guidelines

### 5. Coordination

- Daily sync on completed work and blockers
- Update shared task board with progress
- Create GitHub issues for discovered gaps/ambiguities
- Document breaking changes in CHANGELOG.md

---

## RISK MITIGATION

### 1. LLM Quality Risk

- Maintain prompt versioning
- A/B test prompt variations
- Collect failure cases for refinement
- Support multiple LLM providers for fallback

### 2. Confidence Formula Risk

- Start with conservative parameters
- Instrument heavily for debugging
- Create visualization tools for support networks
- Plan for formula versioning and migration

### 3. Performance Risk

- Benchmark early and often
- Profile before optimizing
- Document hardware requirements
- Plan for horizontal scaling (Phase 6+)

### 4. Coordination Risk

- Clear phase gates prevent premature dependencies
- Each contributor owns complete vertical slices
- Sync points minimize integration conflicts

---

## SUCCESS CRITERIA

**Phase 1:** Foundation components compile, tests pass  
**Phase 2:** End-to-end claim lifecycle (assert → query → retrieve)  
**Phase 3:** Intelligent operations (extract text, evaluate promotions)  
**Phase 4:** Automated maintenance (decay, GC, synthesis)  
**Phase 5:** Production-ready deployment (MCP, CLI, monitoring)

**Final Deliverable:** A single-instance Boswell deployment that:
- Accepts unstructured text and extracts claims
- Stores claims with embeddings and confidence intervals
- Queries semantically with vector search
- Applies tier-based lifecycle management
- Evaluates promotion requests via gatekeeper
- Synthesizes emergent insights in background
- Exposes all functionality via MCP and CLI
- Processes 100+ assertions/sec, queries <100ms p95
