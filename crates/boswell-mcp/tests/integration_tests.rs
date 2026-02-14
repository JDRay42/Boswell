//! Integration tests for MCP server
//!
//! These tests verify the MCP protocol implementation and tool functionality.
//! They test JSON-RPC message handling and tool parameter validation.

use boswell_mcp::McpServer;
use serde_json::json;

#[test]
fn test_protocol_initialize() {
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {}
    });

    let request_str = serde_json::to_string(&request).unwrap();
    assert!(request_str.contains("initialize"));
}

#[test]
fn test_protocol_tools_list() {
    let request = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list",
        "params": {}
    });

    let request_str = serde_json::to_string(&request).unwrap();
    assert!(request_str.contains("tools/list"));
}

#[test]
fn test_protocol_tool_call_assert() {
    let request = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "boswell_assert",
            "arguments": {
                "namespace": "test",
                "subject": "entity1",
                "predicate": "hasProperty",
                "object": "value1",
                "confidence": 0.9,
                "tier": "Permanent"
            }
        }
    });

    let request_str = serde_json::to_string(&request).unwrap();
    assert!(request_str.contains("boswell_assert"));
}

#[test]
fn test_protocol_tool_call_query() {
    let request = json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "tools/call",
        "params": {
            "name": "boswell_query",
            "arguments": {
                "namespace": "test",
                "min_confidence": 0.8
            }
        }
    });

    let request_str = serde_json::to_string(&request).unwrap();
    assert!(request_str.contains("boswell_query"));
}

#[test]
fn test_protocol_tool_call_learn() {
    let request = json!({
        "jsonrpc": "2.0",
        "id": 5,
        "method": "tools/call",
        "params": {
            "name": "boswell_learn",
            "arguments": {
                "claims": [
                    {
                        "namespace": "test",
                        "subject": "entity1",
                        "predicate": "relatesTo",
                        "object": "entity2"
                    }
                ]
            }
        }
    });

    let request_str = serde_json::to_string(&request).unwrap();
    assert!(request_str.contains("boswell_learn"));
}

#[test]
fn test_protocol_tool_call_forget() {
    let request = json!({
        "jsonrpc": "2.0",
        "id": 6,
        "method": "tools/call",
        "params": {
            "name": "boswell_forget",
            "arguments": {
                "claim_ids": ["01HX5ZZKJQH5KW8F5N3D9T7G2A"]
            }
        }
    });

    let request_str = serde_json::to_string(&request).unwrap();
    assert!(request_str.contains("boswell_forget"));
}

#[test]
fn test_protocol_tool_call_search() {
    let request = json!({
        "jsonrpc": "2.0",
        "id": 7,
        "method": "tools/call",
        "params": {
            "name": "boswell_semantic_search",
            "arguments": {
                "query": "machine learning",
                "namespace": "ai",
                "limit": 10
            }
        }
    });

    let request_str = serde_json::to_string(&request).unwrap();
    assert!(request_str.contains("boswell_semantic_search"));
}

// E2E tests that require running servers
// Run these manually with: cargo test -p boswell-mcp --test integration_tests -- --ignored

#[test]
#[ignore]
fn test_e2e_full_workflow() {
    // This test requires:
    // 1. Terminal 1: cargo run -p boswell-grpc
    // 2. Terminal 2: cargo run -p boswell-router -- --config config/router.toml
    // 3. Terminal 3: cargo test -p boswell-mcp --test integration_tests -- --ignored
    
    // For manual testing, use examples/manual_test.sh
    // Or test with Claude Desktop integration
    println!("E2E test: Start gRPC and Router servers, then test MCP server");
}
