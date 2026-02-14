//! Error types for MCP server operations.

use thiserror::Error;

/// MCP server error types
#[derive(Error, Debug)]
pub enum McpError {
    /// Invalid request format or parameters
    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    /// Tool not found
    #[error("Tool not found: {0}")]
    ToolNotFound(String),

    /// Boswell SDK error
    #[error("Boswell error: {0}")]
    BoswellError(String),

    /// JSON serialization/deserialization error
    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),

    /// IO error
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

impl McpError {
    /// Convert to JSON-RPC error code
    pub fn error_code(&self) -> i32 {
        match self {
            McpError::InvalidRequest(_) => -32600,
            McpError::ToolNotFound(_) => -32601,
            McpError::BoswellError(_) => -32000,
            McpError::JsonError(_) => -32700,
            McpError::IoError(_) => -32000,
        }
    }
}
