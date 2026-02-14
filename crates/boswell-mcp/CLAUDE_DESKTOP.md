# Claude Desktop Configuration for Boswell

This file provides example configurations for integrating Boswell MCP server with Claude Desktop.

## macOS Configuration Location

```
~/Library/Application Support/Claude/claude_desktop_config.json
```

## Basic Configuration

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

## Development Configuration (Debug Build)

For development with verbose logging:

```json
{
  "mcpServers": {
    "boswell": {
      "command": "/path/to/boswell/target/debug/boswell-mcp",
      "args": [],
      "env": {
        "BOSWELL_ROUTER": "http://localhost:8080",
        "RUST_LOG": "debug"
      }
    }
  }
}
```

## Remote Router Configuration

If your Boswell router is running on a different machine:

```json
{
  "mcpServers": {
    "boswell": {
      "command": "/path/to/boswell/target/release/boswell-mcp",
      "args": [],
      "env": {
        "BOSWELL_ROUTER": "http://your-server.example.com:8080"
      }
    }
  }
}
```

## Multiple MCP Servers

Claude Desktop supports multiple MCP servers:

```json
{
  "mcpServers": {
    "boswell": {
      "command": "/path/to/boswell/target/release/boswell-mcp",
      "args": [],
      "env": {
        "BOSWELL_ROUTER": "http://localhost:8080"
      }
    },
    "other-mcp-server": {
      "command": "/path/to/other-server",
      "args": []
    }
  }
}
```

## Setup Steps

1. **Build Boswell MCP server:**
   ```bash
   cd /path/to/boswell
   cargo build -p boswell-mcp --release
   ```

2. **Start Boswell stack:**
   ```bash
   # Terminal 1: gRPC server
   cargo run -p boswell-grpc
   
   # Terminal 2: Router
   cargo run -p boswell-router -- --config config/router.toml
   ```

3. **Update Claude config:**
   - Open/create `~/Library/Application Support/Claude/claude_desktop_config.json`
   - Add Boswell configuration (use full path to binary)
   - Save file

4. **Restart Claude Desktop:**
   - Completely quit Claude Desktop (Cmd+Q)
   - Reopen Claude Desktop

5. **Verify integration:**
   - Open Claude Desktop
   - Look for Boswell tools in the command palette
   - Try a simple command: "Assert a claim that Rust is fast"

## Troubleshooting

### Tools Not Appearing

1. **Check config file syntax:**
   ```bash
   cat ~/Library/Application\ Support/Claude/claude_desktop_config.json | jq
   ```
   If this errors, you have invalid JSON.

2. **Verify binary path:**
   ```bash
   ls -la /path/to/boswell/target/release/boswell-mcp
   ```
   Make sure the file exists and is executable.

3. **Check Claude logs:**
   ```bash
   tail -f ~/Library/Logs/Claude/mcp*.log
   ```

### Connection Errors

If Boswell tools appear but fail with connection errors:

1. **Verify Boswell stack is running:**
   ```bash
   # Check gRPC server
   curl -v http://localhost:50051  # Should connect (won't return valid HTTP)
   
   # Check Router
   curl http://localhost:8080/health  # Should return 200 OK
   ```

2. **Check BOSWELL_ROUTER environment variable:**
   Make sure it matches your Router's actual address.

### Debug Mode

To see detailed MCP server logs:

1. Update Claude config with debug logging:
   ```json
   {
     "env": {
       "RUST_LOG": "debug"
     }
   }
   ```

2. Check Claude's MCP logs:
   ```bash
   tail -f ~/Library/Logs/Claude/mcp-boswell.log
   ```

## Example Prompts for Claude

Once configured, try these prompts in Claude Desktop:

**Asserting claims:**
```
"Remember that Rust is a systems programming language with 95% confidence"
"Store the fact that Python is popular for machine learning"
```

**Querying:**
```
"What do you know about Rust?"
"Show me all claims in the 'programming' namespace"
"Find claims with confidence above 90%"
```

**Batch learning:**
```
"Learn these facts about Kubernetes: it's a container orchestrator, 
it was created by Google, and it's written in Go"
```

**Forgetting:**
```
"Forget claim ID 01HX5ZZKJQH5KW8F5N3D9T7G2A"
```

## Security Notes

- The MCP server connects to Boswell via the Router (HTTP)
- The Router handles authentication via JWT tokens (automatic via SDK)
- For production use, consider TLS/SSL for Router connections
- Boswell has no built-in authentication beyond the Router's JWT system

## Windows Configuration

On Windows, the config location is:
```
%APPDATA%\Claude\claude_desktop_config.json
```

Example:
```json
{
  "mcpServers": {
    "boswell": {
      "command": "C:\\path\\to\\boswell\\target\\release\\boswell-mcp.exe",
      "args": [],
      "env": {
        "BOSWELL_ROUTER": "http://localhost:8080"
      }
    }
  }
}
```

## Linux Configuration

On Linux, the config location is:
```
~/.config/Claude/claude_desktop_config.json
```

Configuration is the same as macOS.
