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
  - Note: Returns error indicating feature not yet in SDK
  - Parameters: query_text, namespace?, limit?, threshold?
  - Returns: error message with workaround

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
- [x] `boswell search <query>` - Semantic search placeholder ✅ DONE
  - Awaits SDK HNSW exposure
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

### D1: Extractor Service (`boswell-extractor`)

**Goal:** Extract claims from unstructured text using LLM (ADR-05)

- [ ] Text preprocessing pipeline
- [ ] LLM prompt engineering for claim extraction
- [ ] Triple extraction: (subject, predicate, object)
- [ ] Confidence estimation
- [ ] Batch processing support
- [ ] Integration tests with sample texts

**Deliverable:** `boswell-extractor` crate - Extract claims from text

---

### D2: Synthesizer Service (`boswell-synthesizer`)

**Goal:** Generate summaries and answer questions (ADR-06)

- [ ] Context retrieval from claim store
- [ ] LLM prompt construction
- [ ] Summary generation
- [ ] Question answering
- [ ] Citation support (claim provenance)
- [ ] Streaming responses

**Deliverable:** `boswell-synthesizer` crate - Generate summaries and answers

---

### D3: Janitor Service (`boswell-janitor`)

**Goal:** Tier management and cleanup (ADR-07)

- [ ] Tier promotion logic (Ephemeral → Task → Project → Permanent)
- [ ] Tier demotion based on usage
- [ ] Stale claim detection
- [ ] Garbage collection for Ephemeral tier
- [ ] Scheduled background jobs
- [ ] Metrics and reporting

**Deliverable:** `boswell-janitor` crate - Automated tier management

---

### D4: Gatekeeper Service (`boswell-gatekeeper`)

**Goal:** Quality control and validation (ADR-08)

- [ ] Claim validation rules
- [ ] Duplicate detection (enhanced)
- [ ] Confidence verification
- [ ] Provenance validation
- [ ] Quality scoring
- [ ] Rejection handling

**Deliverable:** `boswell-gatekeeper` crate - Quality control layer

---

## Progress Tracking

| Stream | Status | Tests | Completion |
|--------|--------|-------|------------|
| A: Async SDK | ✅ Complete | 8/8 | 100% |
| B: MCP Server | 🔲 Todo | 0 | 0% |
| C: CLI Tool | 🔲 Todo | 0 | 0% |
| D1: Extractor | 🔲 Todo | 0 | 0% |
| D2: Synthesizer | 🔲 Todo | 0 | 0% |
| D3: Janitor | 🔲 Todo | 0 | 0% |
| D4: Gatekeeper | 🔲 Todo | 0 | 0% |

**Overall Phase 3 Progress:** 14% (1/7 streams complete)

---

## Notes

- **Priority Order:** Stream A (✅) → Stream B → Stream C → Stream D (any order)
- **MCP Server** (Stream B) enables AI-powered workflows with Claude, Cline, etc.
- **CLI Tool** (Stream C) enables human operators and scripting
- **Advanced Services** (Stream D) can be built in parallel after B & C
- All streams depend on async SDK (Stream A) being complete ✅

## Related ADRs

- **ADR-012:** Learn Operation (batch loading)
- **ADR-019:** Stateless Sessions (session management)
- **ADR-005:** Extractor Design
- **ADR-006:** Synthesizer Design
- **ADR-007:** Janitor Design
- **ADR-008:** Gatekeeper Pattern

## Next Session Starting Point

**Start with Stream B (MCP Server)** - enables immediate value for AI-assisted workflows.

See `HANDOFF_PHASE3.md` for detailed continuation instructions.
