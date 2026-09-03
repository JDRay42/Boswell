# Boswell

Boswell is a cognitive memory system designed as the long-term memory substrate for AI agents. It provides persistent, structured, semantically searchable memory that accumulates knowledge over time across tasks, projects, and domains.

## Core Philosophy

- **Claims, not facts** - Nothing is absolute truth; everything is a claim with confidence
- **Organic memory** - Memory works with layers, decay, and emergent insights
- **Gatekeeper pattern** - Agents advocate, gatekeepers decide what persists
- **Speed by default, depth on demand** - Fast deterministic paths with optional LLM-assisted depth
- **Local-first, network-capable** - Privacy and control with optional federation

## Architecture

Boswell follows Clean Architecture principles with clear separation of concerns:

### Domain Layer (innermost)
- `boswell-domain` - Core business logic, value objects, and trait definitions (zero external dependencies)

### Application Layer
- `boswell-extractor` - Converts unstructured text to structured claims
- `boswell-gatekeeper` - Evaluates tier promotion requests
- `boswell-janitor` - Automated maintenance (decay, contradiction detection, GC)
- `boswell-synthesizer` - Discovers emergent patterns and higher-order insights
- `boswell-router` - Session management and instance registry

### Infrastructure Layer
- `boswell-store` - Claim storage (SQLite + HNSW vector index)
- `boswell-llm` - Pluggable LLM provider abstractions
- `boswell-grpc` - gRPC API surface

### Interface Layer
- `boswell-sdk` - Rust client SDK
- `boswell-mcp` - MCP (Model Context Protocol) server
- `boswell-cli` - Command-line interface
- `boswell-gateway` - Public, authenticated HTTP/JSON API (see [HTTP API guide](docs/integrations/http-api.md))

### Runtime Binaries
- `boswell-server` - Instance gRPC daemon (serves the claim store + embedder)
- `boswell-router` - Session management and instance registry (HTTP)
- `boswell-gateway` - Public HTTP/JSON API in front of the private gRPC instance

## Development Setup

### Prerequisites

- Rust 1.88+ (install via [rustup](https://rustup.rs/) or Homebrew)
- Protocol Buffers compiler (`brew install protobuf`)
- Ollama for local LLM testing (`brew install ollama`)
  - Semantic search uses a local embedding model; pull it with `ollama pull embeddinggemma` (see [ADR-013](docs/ADRs/013-local-embedding-models.md))

### Building

```bash
# Build all crates
cargo build

# Build in release mode
cargo build --release

# Run tests
cargo test

# Run tests with output
cargo test -- --nocapture

# Run specific crate tests
cargo test -p boswell-domain
```

### Development Workflow

```bash
# Watch mode - auto-rebuild on changes
cargo watch -x test

# Format code
cargo fmt

# Lint
cargo clippy -- -D warnings

# Check without building
cargo check
```

### Running an instance

The instance server (`boswell-server`) serves the Boswell gRPC API backed by the
SQLite store and a local embedder. By default it uses the EmbeddingGemma model
via Ollama, so make sure Ollama is running and the model is pulled first:

```bash
ollama pull embeddinggemma

# Write a starter config you can edit
cargo run -p boswell-server -- init config/instance.toml

# Start the server (defaults to 127.0.0.1:50051)
cargo run -p boswell-server -- --config config/instance.toml
```

To run without Ollama (e.g. for offline development), set `backend = "mock"`
under `[embedding]` in the config. A `boswell-router` can then register the
instance at its `http://localhost:50051` endpoint (see `config/router.toml`).

To keep memory healthy automatically, enable the background Janitor under
`[janitor]` in the config (`enabled = true`). It runs decay-aware sweeps on a
schedule: stale claims past their tier TTL are garbage-collected, and claims
whose age-decayed confidence (ADR-007) has fallen below the demotion threshold
are demoted a tier. Set `dry_run = true` to log intended changes without
applying them.

To generate emergent insights, enable the background Synthesizer under
`[synthesizer]` (`enabled = true`; requires an Ollama chat model, e.g.
`ollama pull qwen2.5:7b`). On a schedule it clusters related claims, asks the LLM
whether each cluster implies a higher-order insight, and stores accepted
insights as new claims linked to their sources via `derived_from` (ADR-006).
LLM analysis runs without holding the store lock, so gRPC requests are not
blocked during a pass.

To surface conflicting knowledge, enable the Contradiction Janitor under
`[contradiction]` (`enabled = true`; also LLM-backed). It compares claims that
share a subject, asks the LLM whether each pair is incompatible, and records a
`Contradicts` relationship for genuine contradictions — which the confidence
computation (ADR-007) folds in as a penalty, lowering the effective confidence
of both claims. Pairs are rate-limited and already-related pairs are skipped.

### Running the HTTP gateway

The gateway (`boswell-gateway`) exposes the full memory lifecycle as a public,
authenticated HTTP/JSON API so remote agents (e.g. Claude on the web) can use
Boswell over HTTPS. It reuses the SDK internally and keeps the gRPC instance
private; serve TLS and public reach via a reverse proxy or tunnel in front of it.

```bash
# Write a starter config and add your API-key hashes (see the file's comments)
cargo run -p boswell-gateway -- init config/gateway.toml

# Start the gateway (defaults to 127.0.0.1:8081)
cargo run -p boswell-gateway -- --config config/gateway.toml
```

Server-side LLM extraction (`POST /v1/extract` and LLM-mode `/v1/hooks/ingest`)
requires enabling `[extraction]` in the instance config. See the full
[HTTP API guide](docs/integrations/http-api.md) for endpoints, auth scopes, the
claim DTO, and deployment.

## Project Status

🚧 **In Development** — core lifecycle complete end-to-end.

The full organic-memory loop runs across the two-process architecture (router +
instance): assert claims → semantic retrieval via a local embedder → age-based
confidence decay → decay-aware maintenance (tier demotion + GC) → LLM-backed
synthesis of emergent insights → LLM-backed contradiction detection. All
maintenance services run as opt-in background workers inside the instance server.

See [docs/development/roadmap.md](docs/development/roadmap.md) for the development roadmap and [docs/architecture/](docs/architecture/) for component specs.

## Integrations

### Claude Code hooks

Give a coding agent persistent memory by wiring [Claude Code
hooks](https://code.claude.com/docs/en/hooks) into Boswell: a `SessionStart` hook
recalls stored claims into the session, and a `UserPromptSubmit` hook captures new
ones. Runnable local examples (command hooks over `localhost`, no network exposure)
live in [`examples/claude-code-hooks/`](examples/claude-code-hooks/). The
[integration guide](docs/integrations/claude-code-hooks.md) also covers the native
HTTP-hook transport, now backed by the gateway's real `POST /v1/hooks/ingest`
endpoint.

### HTTP API

The [`boswell-gateway`](docs/integrations/http-api.md) serves a public,
authenticated HTTP/JSON API (`/v1`) over the full memory lifecycle — read, write,
search, recall, delete, relationships, extract, and hook ingest — so any external
agent can use Boswell over HTTPS.

## Running Boswell — what's expected of implementers

Boswell is **local-first**: instances typically run on your own hardware with your own
agents. That makes the runtime environment — and its trust boundary — **your
responsibility**. Boswell aims to be a *thorny hedge*, not an impenetrable wall: it raises
the cost of casual or careless memory poisoning and keeps damage recoverable, but it is not
designed to be immune to a determined actor who already controls the host. In practice, plan
for the following.

- **Identity & access are yours to govern.** Boswell provides provenance, tiers, gatekeeping,
  and (by design) an identity-provider port with assurance-gated write tiers — but it does not
  ship a production identity system. You decide which agents to run and what each may write,
  especially to higher (project/permanent) tiers. Run only agents you're willing to trust with
  the tier you grant them.
- **Don't expose the instance carelessly.** The gRPC instance is meant to stay bound to
  `127.0.0.1`. Reach it from remote agents only through the authenticated, TLS-fronted
  [`boswell-gateway`](docs/integrations/http-api.md); see the
  [security model](docs/architecture/10-security.md) and the
  [hooks integration guide](docs/integrations/claude-code-hooks.md). Rotate the router
  `jwt_secret` and gateway API keys; never ship the placeholder secrets to production.
- **Back up your memory, and test the restore.** Memory is durable state. Run regular (e.g.
  nightly) backups and periodically test restoring them; catastrophic poisoning or disk loss is
  recovered from backups plus provenance-targeted cleanup. See
  [Backup & Recovery](docs/architecture/16-backup-recovery.md).
- **Maintenance behavior is opt-in.** The Janitor, Synthesizer, and Contradiction workers are
  off by default; enabling them changes how memory decays, is garbage-collected, and is
  reconciled. Turn them on deliberately.
- **Provide the runtime dependencies.** Semantic search needs Ollama with an embedding model
  (or `backend = "mock"` for offline/no-Ollama use); building needs a protobuf compiler.

## Choosing Your Data Store

Boswell's persistence sits behind a single storage port (the `ClaimStore` trait), so the engine
underneath is a **swappable adapter** — the memory model, the API, and the gateway stay identical
regardless of what stores the data (see
[ADR-020](docs/ADRs/020-swappable-storage-backends.md)).

**Start simple.** Today Boswell ships one adapter: an **embedded SQLite store** with a local
vector index — zero-dependency, single-file, ideal for a local, single-agent instance. This is
where almost everyone should begin.

**Grow when you need to.** As you move toward shared, multi-agent, or hosted use, a **Postgres +
pgvector** adapter is the planned growth path (not yet shipped — see the
[backlog](docs/development/roadmap.md#backlog--future-work)): concurrent writers,
networked/shared access, point-in-time recovery, and standard database operations.

| If you have… | Prefer |
|---|---|
| One agent, one machine, getting started | **SQLite** (embedded, default) |
| Many concurrent agents / a shared or hosted instance | **Postgres + pgvector** (planned) |
| A hard "no external services" constraint | **SQLite** |
| Existing Postgres ops, replication, and backups you trust | **Postgres + pgvector** (planned) |

You aren't locked in: a planned **data-store migration tool** will move your memories from one
adapter to another when you outgrow the simple setup, so starting small costs you nothing later.

## Documentation

- [Architecture Documentation](docs/architecture/) - System design and component specifications
- [Architecture Decision Records](docs/ADRs/) - Key technical decisions and rationale
- [Development Plan](docs/development/roadmap.md) - Phased implementation roadmap
- [Importing Personal Memory](docs/importing-personal-memory.md) - Seed an instance with facts about yourself
- [Claude Code Hooks Integration](docs/integrations/claude-code-hooks.md) - Wire an agent's lifecycle into Boswell's memory (local examples + secure public-serving design)
- [Backup & Recovery](docs/architecture/16-backup-recovery.md) - Durability strategy: consistent snapshots of the store + vector index, and how to restore

## Contributing

See the [contributing guide](docs/architecture/14-contributing.md) and the
[Architecture Decision Records](docs/ADRs/) before opening a PR. All code should
build clean (`cargo build`), pass `cargo clippy -- -D warnings`, and pass
`cargo test`.

## License

Copyright © 2026 the Boswell authors.

Boswell is licensed under the **GNU Affero General Public License v3.0**
([AGPL-3.0](LICENSE)). You may use, modify, and redistribute it under those
terms — including running it as a network service — provided that you preserve
attribution and make your source (including any modifications) available to
users of that service under the same license.

The "Boswell" name is not licensed for use in a way that implies endorsement by
or affiliation with the project.
