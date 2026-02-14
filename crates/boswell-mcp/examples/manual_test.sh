#!/bin/bash
# Manual test script for MCP server
#
# This script demonstrates how to test the MCP server manually by sending
# JSON-RPC requests via stdin and receiving responses via stdout.

set -e

echo "=== Boswell MCP Server Manual Test ==="
echo
echo "Prerequisites:"
echo "  1. Terminal 1: cargo run -p boswell-grpc"
echo "  2. Terminal 2: cargo run -p boswell-router -- --config config/router.toml"
echo
echo "Starting MCP server..."
echo

# Build the MCP server
cargo build -p boswell-mcp

# Test initialize
echo "Test 1: Initialize"
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' | \
  cargo run -p boswell-mcp 2>/dev/null
echo

# Test tools/list
echo "Test 2: List tools"
echo '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}' | \
  cargo run -p boswell-mcp 2>/dev/null
echo

# Test assert
echo "Test 3: Assert a claim"
echo '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"boswell_assert","arguments":{"namespace":"test","subject":"entity1","predicate":"hasProperty","object":"value1","confidence":0.9}}}' | \
  cargo run -p boswell-mcp 2>/dev/null
echo

# Test query
echo "Test 4: Query claims"
echo '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"boswell_query","arguments":{"namespace":"test"}}}' | \
  cargo run -p boswell-mcp 2>/dev/null
echo

echo "=== Tests complete ==="
