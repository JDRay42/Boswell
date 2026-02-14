//! MCP server implementation

use boswell_sdk::BoswellClient;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use tokio::runtime::Runtime;
use tracing::{debug, error, info, warn};

use crate::error::McpError;
use crate::protocol::*;
use crate::tools;

/// MCP Server
///
/// Handles Model Context Protocol requests via stdio transport.
pub struct McpServer {
    client: BoswellClient,
    runtime: Runtime,
}

impl McpServer {
    /// Create a new MCP server
    ///
    /// # Arguments
    ///
    /// * `router_url` - URL of the Boswell router
    ///
    /// # Returns
    ///
    /// Result containing the server or an error
    pub fn new(router_url: String) -> Result<Self, McpError> {
        let runtime = Runtime::new().map_err(|e| {
            McpError::IoError(std::io::Error::new(std::io::ErrorKind::Other, e))
        })?;

        let client = BoswellClient::new(&router_url);

        Ok(Self { client, runtime })
    }

    /// Connect to Boswell router
    pub fn connect(&mut self) -> Result<(), McpError> {
        self.runtime
            .block_on(self.client.connect())
            .map_err(|e| McpError::BoswellError(e.to_string()))?;
        Ok(())
    }

    /// Run the MCP server (stdio transport)
    ///
    /// Reads JSON-RPC requests from stdin and writes responses to stdout.
    pub fn run(&mut self) -> Result<(), McpError> {
        info!("MCP server started");

        let stdin = std::io::stdin();
        let reader = BufReader::new(stdin);
        let mut stdout = std::io::stdout();

        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }

            debug!("Received request: {}", line);

            // Parse request
            let request: JsonRpcRequest = match serde_json::from_str(&line) {
                Ok(req) => req,
                Err(e) => {
                    error!("Failed to parse request: {}", e);
                    let error_response = JsonRpcError::new(
                        None,
                        -32700,
                        format!("Parse error: {}", e),
                    );
                    let error_value = serde_json::to_value(&error_response).unwrap();
                    self.write_response(&mut stdout, &error_value)?;
                    continue;
                }
            };

            // Handle request
            let response = self.handle_request(request);
            self.write_response(&mut stdout, &response)?;
        }

        info!("MCP server stopped");
        Ok(())
    }

    /// Handle a JSON-RPC request
    fn handle_request(&mut self, request: JsonRpcRequest) -> Value {
        let id = request.id.clone();

        match request.method.as_str() {
            "initialize" => self.handle_initialize(id),
            "tools/list" => self.handle_tools_list(id),
            "tools/call" => self.handle_tool_call(id, request.params),
            _ => {
                let error = JsonRpcError::new(
                    id,
                    -32601,
                    format!("Method not found: {}", request.method),
                );
                serde_json::to_value(error).unwrap()
            }
        }
    }

    /// Handle initialize request
    fn handle_initialize(&self, id: Option<Value>) -> Value {
        let response = InitializeResponse {
            protocol_version: "0.1.0".to_string(),
            server_info: ServerInfo {
                name: "boswell-mcp".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
            capabilities: Capabilities {
                tools: ToolsCapability { supported: true },
            },
        };

        let json_response = JsonRpcResponse::new(id, serde_json::to_value(response).unwrap());
        serde_json::to_value(json_response).unwrap()
    }

    /// Handle tools/list request
    fn handle_tools_list(&self, id: Option<Value>) -> Value {
        let tools = vec![
            self.tool_definition_assert(),
            self.tool_definition_query(),
            self.tool_definition_learn(),
            self.tool_definition_forget(),
            self.tool_definition_search(),
        ];

        let response = ToolListResponse { tools };
        let json_response = JsonRpcResponse::new(id, serde_json::to_value(response).unwrap());
        serde_json::to_value(json_response).unwrap()
    }

    /// Handle tools/call request
    fn handle_tool_call(&mut self, id: Option<Value>, params: Value) -> Value {
        let tool_name = match params.get("name").and_then(|v| v.as_str()) {
            Some(name) => name,
            None => {
                let error = JsonRpcError::new(id, -32602, "Missing tool name".to_string());
                return serde_json::to_value(error).unwrap();
            }
        };

        let tool_params = match params.get("arguments") {
            Some(args) => args.clone(),
            None => json!({}),
        };

        // Route to appropriate tool handler
        let result = match tool_name {
            "boswell_assert" => self.call_assert_tool(tool_params),
            "boswell_query" => self.call_query_tool(tool_params),
            "boswell_learn" => self.call_learn_tool(tool_params),
            "boswell_forget" => self.call_forget_tool(tool_params),
            "boswell_semantic_search" => self.call_search_tool(tool_params),
            _ => {
                let error = JsonRpcError::new(
                    id,
                    -32601,
                    format!("Tool not found: {}", tool_name),
                );
                return serde_json::to_value(error).unwrap();
            }
        };

        match result {
            Ok(value) => {
                let response = JsonRpcResponse::new(id, value);
                serde_json::to_value(response).unwrap()
            }
            Err(e) => {
                let error = JsonRpcError::new(id, e.error_code(), e.to_string());
                serde_json::to_value(error).unwrap()
            }
        }
    }

    /// Call assert tool
    fn call_assert_tool(&mut self, params: Value) -> Result<Value, McpError> {
        let params: tools::AssertParams = serde_json::from_value(params)?;
        let result = self.runtime.block_on(tools::handle_assert(&mut self.client, params))?;
        Ok(serde_json::to_value(result)?)
    }

    /// Call query tool
    fn call_query_tool(&mut self, params: Value) -> Result<Value, McpError> {
        let params: tools::QueryParams = serde_json::from_value(params)?;
        let result = self.runtime.block_on(tools::handle_query(&mut self.client, params))?;
        Ok(serde_json::to_value(result)?)
    }

    /// Call learn tool
    fn call_learn_tool(&mut self, params: Value) -> Result<Value, McpError> {
        let params: tools::LearnParams = serde_json::from_value(params)?;
        let result = self.runtime.block_on(tools::handle_learn(&mut self.client, params))?;
        Ok(serde_json::to_value(result)?)
    }

    /// Call forget tool
    fn call_forget_tool(&mut self, params: Value) -> Result<Value, McpError> {
        let params: tools::ForgetParams = serde_json::from_value(params)?;
        let result = self.runtime.block_on(tools::handle_forget(&mut self.client, params))?;
        Ok(serde_json::to_value(result)?)
    }

    /// Call search tool
    fn call_search_tool(&mut self, params: Value) -> Result<Value, McpError> {
        let params: tools::SearchParams = serde_json::from_value(params)?;
        let result = self.runtime.block_on(tools::handle_search(&mut self.client, params))?;
        Ok(serde_json::to_value(result)?)
    }

    /// Write response to stdout
    fn write_response<W: Write>(&self, writer: &mut W, response: &Value) -> Result<(), McpError> {
        let response_str = serde_json::to_string(response)?;
        writeln!(writer, "{}", response_str)?;
        writer.flush()?;
        debug!("Sent response: {}", response_str);
        Ok(())
    }

    // Tool definitions for tools/list response
    fn tool_definition_assert(&self) -> ToolDefinition {
        ToolDefinition {
            name: "boswell_assert".to_string(),
            description: "Assert a new claim into Boswell with optional confidence and tier".to_string(),
            inputSchema: json!({
                "type": "object",
                "properties": {
                    "namespace": {"type": "string", "description": "Namespace for the claim"},
                    "subject": {"type": "string", "description": "Subject (entity or concept)"},
                    "predicate": {"type": "string", "description": "Predicate (relationship or attribute)"},
                    "object": {"type": "string", "description": "Object (value or related entity)"},
                    "confidence": {"type": "number", "description": "Confidence score (0.0-1.0)", "minimum": 0.0, "maximum": 1.0},
                    "tier": {"type": "string", "enum": ["Transient", "Session", "Permanent"], "description": "Persistence tier"}
                },
                "required": ["namespace", "subject", "predicate", "object"]
            }),
        }
    }

    fn tool_definition_query(&self) -> ToolDefinition {
        ToolDefinition {
            name: "boswell_query".to_string(),
            description: "Query claims from Boswell with optional filters".to_string(),
            inputSchema: json!({
                "type": "object",
                "properties": {
                    "namespace": {"type": "string", "description": "Filter by namespace"},
                    "subject": {"type": "string", "description": "Filter by subject"},
                    "predicate": {"type": "string", "description": "Filter by predicate"},
                    "object": {"type": "string", "description": "Filter by object"},
                    "min_confidence": {"type": "number", "description": "Minimum confidence threshold"},
                    "tier": {"type": "string", "enum": ["Transient", "Session", "Permanent"]}
                }
            }),
        }
    }

    fn tool_definition_learn(&self) -> ToolDefinition {
        ToolDefinition {
            name: "boswell_learn".to_string(),
            description: "Batch insert multiple claims into Boswell".to_string(),
            inputSchema: json!({
                "type": "object",
                "properties": {
                    "claims": {
                        "type": "array",
                        "description": "Array of claims to insert",
                        "items": {
                            "type": "object",
                            "properties": {
                                "namespace": {"type": "string"},
                                "subject": {"type": "string"},
                                "predicate": {"type": "string"},
                                "object": {"type": "string"},
                                "confidence": {"type": "number"},
                                "tier": {"type": "string", "enum": ["Transient", "Session", "Permanent"]}
                            },
                            "required": ["namespace", "subject", "predicate", "object"]
                        }
                    }
                },
                "required": ["claims"]
            }),
        }
    }

    fn tool_definition_forget(&self) -> ToolDefinition {
        ToolDefinition {
            name: "boswell_forget".to_string(),
            description: "Remove claims from Boswell by their IDs".to_string(),
            inputSchema: json!({
                "type": "object",
                "properties": {
                    "claim_ids": {
                        "type": "array",
                        "description": "Array of claim IDs (ULIDs) to remove",
                        "items": {"type": "string"}
                    }
                },
                "required": ["claim_ids"]
            }),
        }
    }

    fn tool_definition_search(&self) -> ToolDefinition {
        ToolDefinition {
            name: "boswell_semantic_search".to_string(),
            description: "Perform semantic search to find claims similar to a query".to_string(),
            inputSchema: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Search query text"},
                    "namespace": {"type": "string", "description": "Filter by namespace"},
                    "limit": {"type": "integer", "description": "Maximum results (default: 10)", "default": 10},
                    "threshold": {"type": "number", "description": "Similarity threshold (default: 0.7)", "default": 0.7}
                },
                "required": ["query"]
            }),
        }
    }
}
