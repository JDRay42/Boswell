//! MCP protocol types (JSON-RPC 2.0)

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// JSON-RPC request
#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    /// JSON-RPC version (must be "2.0")
    pub jsonrpc: String,
    /// Request ID
    pub id: Option<Value>,
    /// Method name
    pub method: String,
    /// Method parameters
    #[serde(default)]
    pub params: Value,
}

/// JSON-RPC response (success)
#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    /// JSON-RPC version (must be "2.0")
    pub jsonrpc: String,
    /// Request ID
    pub id: Option<Value>,
    /// Result data
    pub result: Value,
}

/// JSON-RPC error response
#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    /// JSON-RPC version (must be "2.0")
    pub jsonrpc: String,
    /// Request ID
    pub id: Option<Value>,
    /// Error details
    pub error: ErrorDetail,
}

/// Error detail structure
#[derive(Debug, Serialize)]
pub struct ErrorDetail {
    /// Error code
    pub code: i32,
    /// Error message
    pub message: String,
}

impl JsonRpcResponse {
    /// Create a new success response
    pub fn new(id: Option<Value>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result,
        }
    }
}

impl JsonRpcError {
    /// Create a new error response
    pub fn new(id: Option<Value>, code: i32, message: String) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            error: ErrorDetail { code, message },
        }
    }
}

/// MCP tool list response
#[derive(Debug, Serialize)]
pub struct ToolListResponse {
    /// Available tools
    pub tools: Vec<ToolDefinition>,
}

/// Tool definition
#[derive(Debug, Serialize)]
pub struct ToolDefinition {
    /// Tool name
    pub name: String,
    /// Tool description
    pub description: String,
    /// Input schema (JSON Schema)
    pub inputSchema: Value,
}

/// MCP server info
#[derive(Debug, Serialize)]
pub struct ServerInfo {
    /// Server name
    pub name: String,
    /// Server version
    pub version: String,
}

/// Initialize response
#[derive(Debug, Serialize)]
pub struct InitializeResponse {
    /// Protocol version
    #[serde(rename = "protocolVersion")]
    pub protocol_version: String,
    /// Server info
    #[serde(rename = "serverInfo")]
    pub server_info: ServerInfo,
    /// Capabilities
    pub capabilities: Capabilities,
}

/// Server capabilities
#[derive(Debug, Serialize)]
pub struct Capabilities {
    /// Tools capability
    pub tools: ToolsCapability,
}

/// Tools capability
#[derive(Debug, Serialize)]
pub struct ToolsCapability {
    /// Whether tools are supported
    pub supported: bool,
}
