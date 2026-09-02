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

### Runtime Binaries
- `boswell-server` - Instance gRPC daemon (serves the claim store + embedder)
- `boswell-router` - Session management and instance registry (HTTP)

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
HTTP-hook transport and how to serve Boswell securely when it must be publicly
reachable.

## Documentation

- [Architecture Documentation](docs/architecture/) - System design and component specifications
- [Architecture Decision Records](docs/ADRs/) - Key technical decisions and rationale
- [Development Plan](docs/development/roadmap.md) - Phased implementation roadmap
- [Importing Personal Memory](docs/importing-personal-memory.md) - Seed an instance with facts about yourself
- [Claude Code Hooks Integration](docs/integrations/claude-code-hooks.md) - Wire an agent's lifecycle into Boswell's memory (local examples + secure public-serving design)

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
