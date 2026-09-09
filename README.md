# Boswell

Boswell is a cognitive memory system designed as the long-term memory substrate for AI agents. It provides persistent, structured, semantically searchable memory that accumulates knowledge over time across tasks, projects, and domains.

## Core Philosophy

- **Claims, not facts** - Nothing is absolute truth; everything is a claim with confidence
- **Organic memory** - Memory works with layers, decay, and emergent insights
- **Gatekeeper pattern** - Agents advocate, gatekeepers decide what persists
- **Speed by default, depth on demand** - Fast deterministic paths with optional LLM-assisted depth
- **Local-first, network-capable** - Privacy and control with optional federation

## Language

[`CONTEXT.md`](./CONTEXT.md) is the project glossary: one word per concept, with the rejected
synonyms listed so they stay rejected. Read it before naming anything.

## Architecture

Boswell follows Clean Architecture principles with clear separation of concerns:

### Domain Layer (innermost)
- `boswell-domain` - Core business logic, value objects, and trait definitions (depends on nothing but `uuid`, for UUIDv7 identifiers per [ADR-011](docs/ADRs/011-ulid-over-uuid.md))

### Application Layer
- `boswell-extractor` - Converts unstructured text to structured claims
- `boswell-gatekeeper` - Evaluates tier promotion requests
- `boswell-janitor` - Automated maintenance (decay, contradiction detection, GC)
- `boswell-synthesizer` - Discovers emergent patterns and higher-order insights
- `boswell-router` - Session management and instance registry

### Infrastructure Layer
- `boswell-store` - Claim storage (SQLite + HNSW vector index)
- `boswell-llm` - LLM provider port. Two adapters ship: **Ollama** and a test mock. There is no hosted-provider adapter (no Anthropic, no OpenAI) — the abstraction is a trait shape, not a provider set.
- `boswell-grpc` - gRPC API surface

### Interface Layer
- `boswell-sdk` - Rust client SDK
- `boswell-mcp` - MCP (Model Context Protocol) server. **Claims only** — five tools (assert, query, learn, forget, semantic search). Goals and procedures are not exposed over MCP; use the CLI or the HTTP gateway for those.
- `boswell-cli` - Command-line interface
- `boswell-gateway` - Public, authenticated HTTP/JSON API (see [HTTP API guide](docs/integrations/http-api.md))

### Runtime Binaries
- `boswell-server` - Instance gRPC daemon (serves the claim store + embedder)
- `boswell-router` - Session management and instance registry (HTTP)
- `boswell-gateway` - Public HTTP/JSON API in front of the private gRPC instance

## Development Setup

### Prerequisites

- Rust 1.98 (install via [rustup](https://rustup.rs/) or Homebrew). `rust-toolchain.toml` pins the toolchain, so rustup will fetch it for you; the `rust-version` floor in `Cargo.toml` is 1.88 but the pinned build is what CI runs.
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

Claim embeddings are persisted with the claim, and the in-memory vector index is
rebuilt from them each time the instance starts, so semantic search survives a
restart. Claims that predate persistence (or that were written while the embedder
was unreachable) are embedded automatically on the next startup. After a
deliberate embedding-model change, re-embed everything with the offline reindex
(per [ADR-014](docs/ADRs/014-offline-reindexing.md)), which runs with the instance
down:

```bash
cargo run -p boswell-server -- reindex --config config/instance.toml
```

To run without Ollama (e.g. for offline development), set `backend = "mock"`
under `[embedding]` in the config. A `boswell-router` can then register the
instance at its `http://localhost:50051` endpoint (see `config/router.toml`).

To keep memory healthy automatically, enable the background Janitor under
`[janitor]` in the config (`enabled = true`). It runs decay-aware sweeps on a
schedule: stale claims past their tier TTL are garbage-collected, and claims
whose age-decayed confidence (ADR-007) has fallen below the demotion threshold
are demoted a tier. The same pass expires unanswered execution receipts, which is
what makes "silence is not success" true rather than aspirational (see
[procedural memory](#navigating-procedural-memory-from-the-cli) below). Set
`dry_run = true` to log intended changes without applying them.

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

It is the only component with real request authentication: SHA-256-hashed bearer
API keys, per-key scopes and rate limits, and namespace isolation. Its surface
covers claims (including batch writes and `learn`), relationships, search and
recall, extraction, hook ingest, and the goal/procedure/receipt endpoints.

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

### Navigating procedural memory from the CLI

Beyond claims, Boswell stores **procedures** (how-tos) grouped under **goals**
(how work decomposes). The CLI walks that structure.

The examples below invoke `boswell`, the CLI binary. Install it, or substitute
`cargo run -p boswell-cli --` for `boswell` throughout:

```bash
cargo install --path crates/boswell-cli
```

Traversal is stateless — you hold the cursor. Find an entry goal, expand one
level, pick a child, expand again, until a candidate is a procedure:

```bash
# Find where to start
boswell goal list --intent-contains breakfast

# One hop: ranked candidates, the procedures that help you choose, and the
# claims that were read to filter them
boswell goal expand <goal-id> --context time:quick,ldl:low
```

Each candidate's `Kind` column says what to do next: `goal` means expand again,
`procedure` means it is a runnable leaf. Expanding is free and creates no
obligation.

Retrieving a procedure is **not** free. Every procedure handed out carries an
execution receipt, and whoever it was issued to must answer it:

```bash
# Issues a receipt per procedure — you are now on the hook
boswell procedure list --goal goal:person:jd/cook-eggs

# Answer it when the run finishes
boswell procedure report <receipt-id> --outcome success
boswell procedure report <receipt-id> --outcome failure --failure-mode executor-error
```

An unanswered receipt counts as `unknown` against the procedure — silence is not
success, so an agent cannot game its stats by running something and staying quiet
about the failure. The `--failure-mode` matters: `executor-error` blames the
runner and leaves the procedure's counters alone, while `bad-result` and
`step-failed` count against the how-to itself.

Reports are gatekept. A negative report from a low-assurance reporter against a
shared, project-tier procedure is recorded but **quarantined** rather than
applied, so one executor cannot tank a how-to the whole team depends on. The CLI
tells you when that happens.

See [the procedural-memory design](docs/architecture/15-procedural-memory.md) for
the model, and the [HTTP API guide](docs/integrations/http-api.md) for the same
surface over HTTP.

## Project Status

🚧 **In Development** — core lifecycle complete end-to-end; **not yet secure for
multi-host deployment**, because the gRPC instance does not authenticate (see
[what's expected of implementers](#running-boswell--whats-expected-of-implementers)).

The full organic-memory loop runs across the two-process architecture (router +
instance): assert claims → semantic retrieval via a local embedder → age-based
confidence decay → decay-aware maintenance (tier demotion + GC) → LLM-backed
synthesis of emergent insights → LLM-backed contradiction detection. All
maintenance services run as opt-in background workers inside the instance server.
Alongside claims, procedural memory — goals, procedures, execution receipts and
the effectiveness loop — runs over the same stack, reachable from the CLI and the
HTTP gateway.

[docs/development/roadmap.md](docs/development/roadmap.md) is the single source of
truth for what is built, what is open, and what is deferred on purpose;
[docs/architecture/](docs/architecture/) holds the component specs.

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
  ship a production identity system. The only adapter in the repo is
  [`boswell-devauth`](crates/boswell-devauth/), a **development-only** stand-in with four
  sample identities; it refuses to start unless you opt in explicitly and have declared a
  non-production `BOSWELL_ENV` (an undeclared one counts as production). With no provider at
  all, writes are stamped with the lowest assurance and nothing is promoted. You decide which
  agents to run and what each may write, especially to higher (project/permanent) tiers. Run
  only agents you're willing to trust with the tier you grant them.
- **The gRPC instance does not authenticate. Keep it on `127.0.0.1`.** This is a hard
  requirement, not a preference. The instance checks only that a request carries a non-empty
  `auth_token`; it does not verify the router's signature, so any process that can reach the
  port can write to any tier. The router issues a properly signed JWT and the SDK carries it,
  but nothing on the instance side reads it yet — tracked in the
  [roadmap](docs/development/roadmap.md) under *Identity, trust and security*. Reach memory
  from remote agents only through [`boswell-gateway`](docs/integrations/http-api.md), which
  **does** authenticate: SHA-256-hashed bearer API keys, per-key scopes and rate limits, and
  namespace isolation. Put TLS in front of the gateway with a reverse proxy or tunnel — the
  gateway does not terminate it, and neither does the instance (setting `enable_tls` on the
  instance refuses to start rather than pretending). Rotate gateway API keys and the router
  `jwt_secret`; never ship the placeholder secrets. See the
  [security model](docs/architecture/10-security.md) and the
  [hooks integration guide](docs/integrations/claude-code-hooks.md).
- **Back up your memory, and test the restore.** Memory is durable state. Run regular (e.g.
  nightly) backups and periodically test restoring them; catastrophic poisoning or disk loss is
  recovered from backups plus provenance-targeted cleanup. **Boswell ships no backup tooling
  yet** — `boswell backup` / `boswell restore` are on the roadmap, so today this means copying
  the database file yourself while the instance is stopped. See
  [Backup & Recovery](docs/architecture/16-backup-recovery.md) for the strategy.
- **Maintenance behavior is opt-in.** The Janitor, Synthesizer, and Contradiction workers are
  off by default; enabling them changes how memory decays, is garbage-collected, and is
  reconciled. Turn them on deliberately.
- **Provide the runtime dependencies.** Semantic search needs Ollama with an embedding model
  (or `backend = "mock"` for offline/no-Ollama use); building needs a protobuf compiler.

## Data Store

Boswell ships **one** storage adapter: an **embedded SQLite store** with a local vector
index — zero-dependency, single-file, and the right choice for a local instance. There is
no second adapter today and therefore no decision to make.

Persistence sits behind a storage port (the `ClaimStore` trait), and a **Postgres + pgvector**
adapter for shared, multi-agent or hosted use is the intended growth path
([ADR-020](docs/ADRs/020-swappable-storage-backends.md)). Both it and a data-store migration
tool are open work, and the port itself needs hardening before a second implementation can sit
behind it — see the [roadmap](docs/development/roadmap.md) under *Storage portability* for
where that stands.

## Documentation

- [Architecture Documentation](docs/architecture/) - System design and component specifications
- [Architecture Decision Records](docs/ADRs/) - Key technical decisions and rationale
- [Roadmap](docs/development/roadmap.md) - The single source of truth for what is built, what is open, and what is deferred
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
