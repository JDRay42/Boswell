# Boswell MCP Server

Model Context Protocol (MCP) server for Boswell cognitive memory system. Enables AI clients like Claude Desktop and Cline to interact with Boswell's knowledge graph.

## Overview

The Boswell MCP server exposes five tools via the Model Context Protocol:

1. **boswell_assert** - Assert new claims into the knowledge graph
2. **boswell_query** - Query claims with flexible filters  
3. **boswell_learn** - Batch insert multiple claims
4. **boswell_forget** - Remove claims by ID
5. **boswell_semantic_search** - Semantic search (coming soon)

## Architecture

```
┌─────────────────┐
│  AI Client      │ (Claude Desktop, Cline, etc.)
│  (MCP Client)   │
└────────┬────────┘
         │ JSON-RPC over stdio
         ▼
┌─────────────────┐
│  boswell-mcp    │ MCP Server (this crate)
└────────┬────────┘
         │ Async SDK
         ▼
┌─────────────────┐
│ boswell-router  │ HTTP + JWT sessions
└────────┬────────┘
         │ gRPC
         ▼
┌─────────────────┐
│  boswell-grpc   │ gRPC Service
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│ boswell-store   │ SQLite + HNSW
└─────────────────┘
```

## Installation

### Build from source

```bash
cd /path/to/boswell
cargo build -p boswell-mcp --release
```

The binary will be at `target/release/boswell-mcp`.

## Usage

### Prerequisites

The MCP server requires a running Boswell stack:

1. **gRPC server** (localhost:50051)
2. **Router** (localhost:8080)

Start them in separate terminals:

```bash
# Terminal 1: gRPC server
cargo run -p boswell-grpc

# Terminal 2: Router
cargo run -p boswell-router -- --config config/router.toml
```

### Running the MCP Server

The MCP server communicates via stdio (standard input/output):

```bash
# Default router URL (http://localhost:8080)
cargo run -p boswell-mcp

# Custom router URL
BOSWELL_ROUTER=http://custom-host:8080 cargo run -p boswell-mcp
```

### Environment Variables

- `BOSWELL_ROUTER` - Router URL (default: `http://localhost:8080`)

## Claude Desktop Integration

### Configuration

Add Boswell to Claude Desktop's MCP configuration:

**macOS**: `~/Library/Application Support/Claude/claude_desktop_config.json`

```json
{
  "mcpServers": {
    "boswell": {
      "command": "/path/to/boswell/target/release/boswell-mcp",
      "args": [],
      "env": {
        "BOSWELL_ROUTER": "http://localhost:8080"
      }
    }
  }
}
```

### Usage in Claude Desktop

Once configured, Claude can use Boswell tools:

**Example prompts:**

```
"Assert a claim that Rust is a systems programming language with 95% confidence"

"Query all claims in the 'programming' namespace"

"Learn these facts about Python: it's interpreted, dynamically typed, and popular for ML"

"Search for claims related to 'machine learning frameworks'"
```

## MCP Tools Reference

### 1. boswell_assert

Assert a new claim into Boswell.

**Parameters:**
- `namespace` (string, required) - Namespace for organization
- `subject` (string, required) - Entity or concept
- `predicate` (string, required) - Relationship or attribute
- `object` (string, required) - Value or related entity
- `confidence` (number, optional) - Confidence score 0.0-1.0
- `tier` (string, optional) - Persistence tier: "Transient", "Session", "Permanent"

**Returns:**
- `claim_id` - Unique ULID identifier
- `message` - Success message

**Example:**
```json
{
  "namespace": "programming",
  "subject": "Rust",
  "predicate": "hasProperty",
  "object": "memory-safe",
  "confidence": 0.95,
  "tier": "Permanent"
}
```

### 2. boswell_query

Query claims with optional filters.

**Parameters:**
- `namespace` (string, optional) - Filter by namespace
- `subject` (string, optional) - Filter by subject
- `predicate` (string, optional) - Filter by predicate
- `object` (string, optional) - Filter by object
- `min_confidence` (number, optional) - Minimum confidence threshold
- `tier` (string, optional) - Filter by tier

**Returns:**
- `count` - Number of matching claims
- `claims` - Array of claim objects

**Example:**
```json
{
  "namespace": "programming",
  "min_confidence": 0.8
}
```

### 3. boswell_learn

Batch insert multiple claims.

**Parameters:**
- `claims` (array, required) - Array of claim objects (same fields as assert)

**Returns:**
- `success_count` - Number of claims successfully inserted
- `total_count` - Total claims attempted
- `claim_ids` - Array of created claim IDs
- `errors` - Array of error messages (if any)

**Example:**
```json
{
  "claims": [
    {
      "namespace": "programming",
      "subject": "Python",
      "predicate": "hasProperty",
      "object": "interpreted"
    },
    {
      "namespace": "programming",
      "subject": "Python",
      "predicate": "usedFor",
      "object": "machine-learning"
    }
  ]
}
```

### 4. boswell_forget

Remove claims by their IDs.

**Parameters:**
- `claim_ids` (array, required) - Array of claim ID strings (ULIDs)

**Returns:**
- `success_count` - Number of claims removed
- `total_count` - Total claims attempted
- `errors` - Array of error messages (if any)

**Example:**
```json
{
  "claim_ids": [
    "01HX5ZZKJQH5KW8F5N3D9T7G2A",
    "01HX5ZZKJQH5KW8F5N3D9T7G2B"
  ]
}
```

### 5. boswell_semantic_search

**Status:** Coming soon

Semantic search using embeddings. Currently returns an error indicating the feature is under development. The underlying HNSW vector search exists in the store layer but is not yet exposed via the gRPC API.

Use `boswell_query` for exact filter-based search in the meantime.

## Testing

### Unit Tests

```bash
cargo test -p boswell-mcp
```

Currently passing: **8 tests**

### Integration Tests

Integration tests verify JSON-RPC protocol handling:

```bash
cargo test -p boswell-mcp --test integration_tests
```

### Manual Testing

Use the provided script to test the server manually:

```bash
# Ensure gRPC and Router are running
chmod +x examples/manual_test.sh
./examples/manual_test.sh
```

Or use `echo` to pipe JSON-RPC requests:

```bash
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' | cargo run -p boswell-mcp
```

## Development

### Adding New Tools

1. Create a new module in `src/tools/`
2. Implement `handle_<tool_name>` function
3. Add tool definition in `src/server.rs`
4. Add route in `handle_tool_call`
5. Export from `src/tools/mod.rs`
6. Add tests

### Protocol Details

The server implements JSON-RPC 2.0 over stdio:
- Reads newline-delimited JSON from stdin
- Writes JSON responses to stdout
- Logs to stderr

Supported methods:
- `initialize` - Initialize MCP session
- `tools/list` - List available tools
- `tools/call` - Invoke a tool

## Troubleshooting

### Connection Errors

**Problem:** `Failed to connect to Boswell at http://localhost:8080`

**Solution:** Ensure gRPC server and Router are running:
```bash
# Start gRPC server
cargo run -p boswell-grpc

# Start Router  
cargo run -p boswell-router -- --config config/router.toml
```

### Claude Desktop Not Seeing Tools

**Problem:** Tools don't appear in Claude Desktop

**Solution:**
1. Verify config file path and JSON syntax
2. Restart Claude Desktop completely
3. Check Claude logs: `~/Library/Logs/Claude/`
4. Verify binary path is correct and executable

### Authentication Errors

**Problem:** `Authentication failed`

**Solution:** The SDK auto-reconnects on auth failures. If persistent:
1. Restart the Router
2. Check Router logs for session issues
3. Verify Router config

## Architecture Notes

- **Max file size**: 300 lines per file (refactor if exceeded)
- **Async runtime**: Uses Tokio via boswell-sdk
- **Error handling**: Uses `thiserror` for error types
- **Confidence intervals**: All claims have confidence intervals [lower, upper]
- **IDs**: Uses ULIDs (not UUIDs) per ADR-011

## Future Enhancements

- [ ] Full semantic search implementation
- [ ] MCP resources (read-only views)
- [ ] Streaming support for large queries
- [ ] Tool usage analytics
- [ ] Enhanced error messages with troubleshooting tips

## License

MIT OR Apache-2.0

## Related Documentation

- [MCP Specification](https://modelcontextprotocol.io/)
- [Boswell Architecture](../../docs/architecture/)
- [Phase 3 Checklist](../../docs/development/phase-3-checklist.md)
