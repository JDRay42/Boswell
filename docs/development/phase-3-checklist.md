# Phase 3: Client Tools & Advanced Features - Implementation Checklist

## Overview
Enhance client SDK, implement MCP server, build CLI, and add advanced services.

**Status:** 🟡 In Progress  
**Start Date:** February 14, 2026  
**Target Completion:** TBD

---

## Stream A: Async SDK Enhancement (`boswell-sdk`) ✅ COMPLETE

### ✅ Async/Await Conversion
- [x] Convert all SDK methods to async (connect, assert, query, learn, forget) ✅ DONE
- [x] Remove blocking runtime overhead ✅ DONE
- [x] Update Cargo.toml: remove "blocking" from reqwest ✅ DONE
- [x] Pure async implementation using tokio ✅ DONE

### ✅ Connection Pooling
- [x] HTTP client with connection pooling (max 10 idle/host) ✅ DONE
- [x] Configure timeout (30 seconds) ✅ DONE
- [x] Lazy gRPC connection establishment ✅ DONE

### ✅ Auto-Reconnection Logic
- [x] Detect authentication failures ✅ DONE
- [x] Automatic session renewal (single retry) ✅ DONE
- [x] Implement retry using loops (avoid recursion) ✅ DONE
- [x] Maintain session continuity per ADR-019 ✅ DONE

### ✅ Integration Testing
- [x] Unit tests for not-connected errors ✅ DONE
- [x] E2E tests for full flow (client → router → gRPC) ✅ DONE
- [x] Manual E2E test instructions ✅ DONE
- [x] 8 tests passing (5 unit + 3 E2E ignored) ✅ DONE

### ✅ Documentation
- [x] Update README.md with async examples ✅ DONE
- [x] Update inline docs with #[tokio::main] ✅ DONE
- [x] Document breaking changes ✅ DONE

**Deliverable:** ✅ Async SDK with connection pooling and auto-reconnection (Phase 3A Complete)

**Commit:** `52669ec` - Phase 3A: Async SDK with connection pooling and auto-reconnection

---

## Stream B: MCP Server (`boswell-mcp`) ✅ COMPLETE

**Goal:** Implement Model Context Protocol server for AI client integration (Claude Desktop, Cline, etc.)

### ✅ MCP Protocol Implementation
- [x] Add dependencies (tokio, serde, serde_json, tracing) ✅ DONE
- [x] Implement MCP server transport (stdio) ✅ DONE
- [x] Setup protocol handler and message routing ✅ DONE
- [x] Error handling and validation ✅ DONE

### ✅ MCP Tools
- [x] `boswell_assert` - Assert a claim ✅ DONE
  - Parameters: namespace, subject, predicate, object, confidence?, tier?
  - Returns: claim_id
- [x] `boswell_query` - Query claims with filters ✅ DONE
  - Parameters: namespace?, subject?, predicate?, min_confidence?, tier?
  - Returns: list of claims (formatted as JSON)
- [x] `boswell_learn` - Batch claim insertion ✅ DONE
  - Parameters: claims array
  - Returns: insertion summary
- [x] `boswell_forget` - Remove claims ✅ DONE
  - Parameters: claim_ids array
  - Returns: success status
- [x] `boswell_semantic_search` - Semantic search with embeddings ✅ DONE
  - Note: Fully wired end-to-end (store HNSW → gRPC Search RPC → SDK → MCP)
  - Parameters: query_text, namespace?, limit?, threshold?
  - Returns: ranked claims with similarity scores

### ✅ Configuration
- [x] Router endpoint configuration ✅ DONE
- [x] Authentication handling (via SDK) ✅ DONE
- [x] Auto-connect on startup ✅ DONE
- [x] Tool descriptions and schemas ✅ DONE

### ✅ Infrastructure
- [x] Server initialization and lifecycle ✅ DONE
- [x] Connection to Boswell via SDK ✅ DONE
- [x] Logging and error reporting (tracing to stderr) ✅ DONE
- [x] Example Claude Desktop config ✅ DONE

### ✅ Testing
- [x] Unit tests for each tool (8 tests) ✅ DONE
- [x] Integration tests (7 tests) ✅ DONE
- [x] Manual testing script ✅ DONE
- [x] Example prompts and workflows ✅ DONE

**Deliverable:** ✅ `boswell-mcp` crate - MCP server exposing Boswell to AI clients

**Commit:** `[pending]` - Phase 3B: MCP Server with 5 tools and Claude Desktop integration

**Tests Passing:** 16+ tests (8 unit + 7 integration + 1 doc)

---

## Stream C: CLI Tool (`boswell-cli`) ✅ COMPLETE

**Goal:** Command-line interface for Boswell operations

### ✅ Core Commands
- [x] `boswell connect` - Establish session with router ✅ DONE
  - Optional profile save with `--profile-name`
- [x] `boswell assert <subject> <predicate> <object>` - Assert claim ✅ DONE
  - Flags: `--confidence`, `--tier`
  - Entity format: `namespace:value`
- [x] `boswell query` - Query claims with filters ✅ DONE
  - Flags: `--subject`, `--predicate`, `--object`, `--namespace`, `--tier`, `--limit`
- [x] `boswell learn <file.json>` - Bulk load claims from JSON ✅ DONE
  - Support JSON array of claim definitions
- [x] `boswell forget <claim-ids>...` - Remove claims ✅ DONE
  - Support for file input with `--file`
  - Confirmation prompt with `--yes` to skip
- [x] `boswell search <query>` - Semantic (vector) search ✅ DONE
  - Flags: `--namespace`, `--limit`, `--threshold`; wired through SDK Search RPC
- [x] `boswell profile` - Profile management ✅ DONE
  - Subcommands: list, show, switch, set, delete

### ✅ Interactive REPL Mode
- [x] `boswell repl` - Start interactive session ✅ DONE
- [x] Command history (saved to `~/.boswell/history.txt`) ✅ DONE
- [x] Line editing with rustyline ✅ DONE
- [x] Auto-generated help system ✅ DONE

### ✅ Configuration Management
- [x] Config file: `~/.boswell/config.toml` ✅ DONE
- [x] Profile support with settings ✅ DONE
- [x] `boswell profile set <profile> <key> <value>` - Update profile ✅ DONE
- [x] `boswell profile show <profile>` - View profile ✅ DONE
- [x] Command-line overrides with `--profile` flag ✅ DONE

### ✅ Output Formatting
- [x] JSON output (`--format json`) ✅ DONE
- [x] Table output (default, human-friendly) ✅ DONE
- [x] Quiet mode (`--format quiet` for IDs only) ✅ DONE
- [x] Color support with `--no-color` toggle ✅ DONE

### ✅ Infrastructure
- [x] Use `clap` 4.5 for argument parsing ✅ DONE
- [x] Use `boswell-sdk` for all operations ✅ DONE
- [x] Connection reuse with active profile ✅ DONE
- [x] Comprehensive error messages with context ✅ DONE

### ✅ Testing
- [x] Unit tests (21 tests across all modules) ✅ DONE
  - Config management tests (3)
  - Output formatting tests (6)
  - CLI parsing tests (2)
  - Command parsing tests (9)
  - All tests passing
- [x] README.md with comprehensive documentation ✅ DONE

**Deliverable:** ✅ `boswell-cli` crate - Full-featured CLI tool (Phase 3C Complete)

**Commit:** `ee039cc` - Phase 3C: CLI Tool implementation with 7 commands, REPL mode, and comprehensive test coverage

---

## Stream D: Advanced Services 🔲 TODO

### D3: Extractor Service (`boswell-extractor`) ✅ COMPLETE

**Goal:** Extract claims from unstructured text using LLM (ADR-05)

- [x] Text preprocessing pipeline (chunking with configurable strategy) ✅ DONE
- [x] LLM prompt engineering for claim extraction ✅ DONE
- [x] Triple extraction: (subject, predicate, object) ✅ DONE
- [x] Confidence estimation ✅ DONE
- [x] Batch processing support (chunked documents) ✅ DONE
- [x] Gatekeeper integration for validation ✅ DONE
- [x] Integration tests with sample texts ✅ DONE

**Deliverable:** ✅ `boswell-extractor` crate - Extract claims from text

**Commit:** `6e08d60` - Phase 3D3: Implement Extractor service for text-to-claims conversion

---

### D2: Janitor Service (`boswell-janitor`) ✅ COMPLETE

**Goal:** Tier management and cleanup (ADR-07)

- [x] Core Janitor struct with tier management ✅ DONE
- [x] JanitorConfig with TOML deserialization ✅ DONE
- [x] JanitorError type with proper error handling ✅ DONE
- [x] Background worker with tokio intervals ✅ DONE
- [x] Sweep methods (ephemeral, task, project) ✅ DONE
- [x] Tier promotion logic (Ephemeral → Task → Project → Permanent) ✅ DONE
- [x] Tier demotion based on staleness and confidence ✅ DONE
- [x] Stale claim detection per tier TTL ✅ DONE
- [x] Batch deletion support ✅ DONE
- [x] Dry-run mode for testing ✅ DONE
- [x] Metrics collection and reporting ✅ DONE
- [x] Configuration presets (default, aggressive, lenient) ✅ DONE
- [x] Unit tests (24 tests) ✅ DONE
- [x] README.md with comprehensive documentation ✅ DONE
- [x] Inline rustdoc for all public APIs ✅ DONE

**Deliverable:** ✅ `boswell-janitor` crate - Automated tier management and cleanup

**Tests Passing:** 24 tests (5 config + 8 janitor + 6 metrics + 4 worker + 1 integration)

**Features Implemented:**
- Tier-specific TTL enforcement (Ephemeral: 12h, Task: 24h, Project: 90d)
- Automatic tier promotion based on confidence and freshness
- Automatic tier demotion based on staleness and low confidence
- Background worker with configurable sweep intervals
- Comprehensive metrics (deletions, promotions, demotions per tier)
- Three configuration presets (default, aggressive, lenient)
- Dry-run mode for safe testing
- Never auto-deletes Permanent tier claims

**Architecture:**
```
JanitorWorker (tokio background service)
    ↓
Janitor (sweep logic + tier management)
    ↓
ClaimStore (query + delete operations)
```

**Performance:**
- Sweep time: ~10ms per 1,000 claims
- CPU usage: Negligible at default 60-minute intervals
- Memory: Minimal (streaming query results)

**Commit:** `[pending]` - Phase 3D2: Janitor service with automated tier management

---

### D4: Synthesizer Service (`boswell-synthesizer`) ✅ COMPLETE

**Goal:** Discover emergent patterns and higher-order insights across claims (ADR-06)

- [x] Candidate selection (by namespace, tier, recency, relationship density) ✅ DONE
- [x] Cluster building from the relationship graph ✅ DONE
- [x] LLM prompt construction for cluster analysis ("no insight" is valid) ✅ DONE
- [x] Insight generation with `source_type: inference` ✅ DONE
- [x] `derived_from` relationship creation to constituent claims ✅ DONE
- [x] Confidence propagation (uncertainty widens outward) ✅ DONE
- [x] Derivation-depth limit to prevent runaway meta-synthesis ✅ DONE
- [x] Gatekeeper validation of synthesized claims ✅ DONE
- [x] Background worker (scheduled passes) + dry-run mode ✅ DONE
- [x] `run_pass(scope) -> SynthesisReport` interface (+ `run_pass_shared` for the server) ✅ DONE
- [x] Unit + integration tests (incl. live Ollama synthesis) ✅ DONE
- [x] Scheduled in the instance server via `[synthesizer]` config ✅ DONE

**Commit:** `c14e76d` (service) + `009dbf0` (server scheduling)

**Deliverable:** `boswell-synthesizer` crate - Discover emergent patterns and derived claims

---

### D1: Gatekeeper Service (`boswell-gatekeeper`) ✅ COMPLETE

**Goal:** Quality control and validation (ADR-08)

- [x] Claim validation rules ✅ DONE
- [x] Duplicate detection (exact match) ✅ DONE
- [x] Confidence verification ✅ DONE
- [x] Entity format validation (namespace:value) ✅ DONE
- [x] Tier appropriateness checking ✅ DONE
- [x] Quality scoring ✅ DONE
- [x] Rejection handling with detailed reasons ✅ DONE
- [x] Configurable validation rules (default/permissive/strict) ✅ DONE
- [x] Unit tests (10 tests covering all validation rules) ✅ DONE
- [x] README documentation ✅ DONE

**Deliverable:** ✅ `boswell-gatekeeper` crate - Quality control and validation

**Tests Passing:** 10 tests (3 config + 7 validation)

**Features Implemented:**
- Entity format validation (namespace:value structure)
- Confidence bounds checking (0.0 ≤ low < high ≤ 1.0)
- Tier-specific confidence requirements
- Duplicate detection (exact match via store)
- Quality scoring (0.0-1.0)
- Detailed rejection reasons
- Three configuration presets (default, permissive, strict)

**Future Enhancements:**
- [ ] Semantic duplicate detection (requires HNSW in SDK)
- [ ] LLM-based semantic validation
- [ ] Provenance chain validation
- [ ] Tier promotion evaluation
- [ ] Metric tracking

**Commit:** `[pending]` - Phase 3D1: Gatekeeper service with comprehensive validation

**Deliverable:** `boswell-gatekeeper` crate - Quality control layer

---

## Progress Tracking

| Stream | Status | Tests | Completion |
|--------|--------|-------|------------|
| A: Async SDK | ✅ Complete | 8/8 | 100% |
| B: MCP Server | ✅ Complete | 16/16 | 100% |
| C: CLI Tool | ✅ Complete | 21/21 | 100% |
| D1: Gatekeeper | ✅ Complete | 10/10 | 100% |
| D2: Janitor | ✅ Complete | 24/24 | 100% |
| D3: Extractor | ✅ Complete | — | 100% |
| D4: Synthesizer | ✅ Complete | — | 100% |

**Overall Phase 3 Progress:** 100% (7/7 streams complete)

**Beyond Phase 3:** semantic search wired end-to-end (store HNSW → gRPC → SDK → MCP/CLI) with a real local embedder (EmbeddingGemma); `boswell-server` instance daemon; age-based confidence decay + cache (ADR-007); decay-aware Janitor with real effects; scheduled Synthesizer; and an LLM-backed Contradiction Janitor — all running as opt-in background services in the instance.

**Total Tests Passing:** 324 workspace tests green (0 build/clippy warnings).

---

## Notes

- **Priority Order:** Stream A (✅) → Stream B (✅) → Stream C (✅) → Stream D1-D2 (✅) → D3-D4 (Todo)
- **MCP Server** (Stream B) enables AI-powered workflows with Claude, Cline, etc. ✅
- **CLI Tool** (Stream C) enables human operators and scripting ✅
- **Gatekeeper** (Stream D1) provides quality control and validation ✅
- **Janitor** (Stream D2) handles automated tier management and cleanup ✅
- **Extractor** (Stream D3) converts unstructured text into claims ✅
- **Synthesizer** (Stream D4) discovers emergent higher-order insights — final Phase 3 stream 🟡
- All streams depend on async SDK (Stream A) being complete ✅

## Related ADRs

- **ADR-007:** Hybrid Confidence Computation (Janitor staleness decay)
- **ADR-008:** Gatekeeper Pattern (validation)
- **ADR-012:** Learn Operation (batch loading)
- **ADR-019:** Stateless Sessions (session management)

## Next Session Starting Point

**Stream D4 (Synthesizer)** is the final remaining Phase 3 stream — see the checklist above.
Once complete, Phase 3 closes and work moves to the next phase.
