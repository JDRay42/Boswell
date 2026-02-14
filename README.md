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

## Development Setup

### Prerequisites

- Rust 1.88+ (install via [rustup](https://rustup.rs/) or Homebrew)
- Protocol Buffers compiler (`brew install protobuf`)
- Ollama for local LLM testing (`brew install ollama`)

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

## Project Status

🚧 **In Development** - Phase 1: Foundation

See [docs/development/plan-boswellDevelopment.prompt.md](docs/development/plan-boswellDevelopment.prompt.md) for the complete development roadmap.

## Documentation

- [Architecture Documentation](docs/architecture/) - System design and component specifications
- [Architecture Decision Records](docs/ADRs/) - Key technical decisions and rationale
- [Development Plan](docs/development/plan-boswellDevelopment.prompt.md) - Phased implementation roadmap

## License

MIT OR Apache-2.0
