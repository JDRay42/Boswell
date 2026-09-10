//! MCP server implementation

use boswell_sdk::BoswellClient;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use tokio::runtime::Runtime;
use tracing::{debug, error, info};

use crate::error::McpError;
use crate::protocol::*;
use crate::tools;

/// MCP Server
///
/// Handles Model Context Protocol requests via stdio transport.
pub struct McpServer {
    client: BoswellClient,
    runtime: Runtime,
    /// The principal named on every execution receipt this server takes out.
    ///
    /// Server-side, never a tool argument: a caller that names itself on a
    /// receipt is not accountable for answering it (design 15 §3.3).
    principal: String,
}

/// The principal used when none is configured.
///
/// A receipt with nobody accountable for reporting is not a contract, so this
/// is a real name rather than an empty string — but it is a weak one, and
/// `BOSWELL_MCP_PRINCIPAL` exists to replace it.
pub const DEFAULT_PRINCIPAL: &str = "mcp";

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
        let runtime = Runtime::new().map_err(|e| McpError::IoError(std::io::Error::other(e)))?;

        let client = BoswellClient::new(&router_url);

        Ok(Self {
            client,
            runtime,
            principal: DEFAULT_PRINCIPAL.to_string(),
        })
    }

    /// Name the principal this server issues execution receipts to.
    ///
    /// An empty name is refused: it would put the receipt on nobody.
    pub fn with_principal(mut self, principal: impl Into<String>) -> Result<Self, McpError> {
        let principal = principal.into();
        if principal.trim().is_empty() {
            return Err(McpError::InvalidRequest(
                "principal must not be empty".to_string(),
            ));
        }
        self.principal = principal;
        Ok(self)
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
                    let error_response =
                        JsonRpcError::new(None, -32700, format!("Parse error: {}", e));
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
                let error =
                    JsonRpcError::new(id, -32601, format!("Method not found: {}", request.method));
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
            self.tool_definition_query_goals(),
            self.tool_definition_get_goal(),
            self.tool_definition_expand_goal(),
            self.tool_definition_query_procedures(),
            self.tool_definition_get_procedure(),
            self.tool_definition_report_outcome(),
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
            "boswell_query_goals" => self.call_query_goals_tool(tool_params),
            "boswell_get_goal" => self.call_get_goal_tool(tool_params),
            "boswell_expand_goal" => self.call_expand_goal_tool(tool_params),
            "boswell_query_procedures" => self.call_query_procedures_tool(tool_params),
            "boswell_get_procedure" => self.call_get_procedure_tool(tool_params),
            "boswell_report_outcome" => self.call_report_outcome_tool(tool_params),
            _ => {
                let error = JsonRpcError::new(id, -32601, format!("Tool not found: {}", tool_name));
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
        let result = self
            .runtime
            .block_on(tools::handle_assert(&mut self.client, params))?;
        Ok(serde_json::to_value(result)?)
    }

    /// Call query tool
    fn call_query_tool(&mut self, params: Value) -> Result<Value, McpError> {
        let params: tools::QueryParams = serde_json::from_value(params)?;
        let result = self
            .runtime
            .block_on(tools::handle_query(&mut self.client, params))?;
        Ok(serde_json::to_value(result)?)
    }

    /// Call learn tool
    fn call_learn_tool(&mut self, params: Value) -> Result<Value, McpError> {
        let params: tools::LearnParams = serde_json::from_value(params)?;
        let result = self
            .runtime
            .block_on(tools::handle_learn(&mut self.client, params))?;
        Ok(serde_json::to_value(result)?)
    }

    /// Call forget tool
    fn call_forget_tool(&mut self, params: Value) -> Result<Value, McpError> {
        let params: tools::ForgetParams = serde_json::from_value(params)?;
        let result = self
            .runtime
            .block_on(tools::handle_forget(&mut self.client, params))?;
        Ok(serde_json::to_value(result)?)
    }

    /// Call search tool
    fn call_search_tool(&mut self, params: Value) -> Result<Value, McpError> {
        let params: tools::SearchParams = serde_json::from_value(params)?;
        let result = self
            .runtime
            .block_on(tools::handle_search(&mut self.client, params))?;
        Ok(serde_json::to_value(result)?)
    }

    /// Call query_goals tool
    fn call_query_goals_tool(&mut self, params: Value) -> Result<Value, McpError> {
        let params: tools::QueryGoalsParams = serde_json::from_value(params)?;
        let result = self
            .runtime
            .block_on(tools::handle_query_goals(&mut self.client, params))?;
        Ok(serde_json::to_value(result)?)
    }

    /// Call get_goal tool
    fn call_get_goal_tool(&mut self, params: Value) -> Result<Value, McpError> {
        let params: tools::GetGoalParams = serde_json::from_value(params)?;
        let result = self
            .runtime
            .block_on(tools::handle_get_goal(&mut self.client, params))?;
        Ok(serde_json::to_value(result)?)
    }

    /// Call expand_goal tool
    fn call_expand_goal_tool(&mut self, params: Value) -> Result<Value, McpError> {
        let params: tools::ExpandGoalParams = serde_json::from_value(params)?;
        let result = self
            .runtime
            .block_on(tools::handle_expand_goal(&mut self.client, params))?;
        Ok(serde_json::to_value(result)?)
    }

    /// Call query_procedures tool
    fn call_query_procedures_tool(&mut self, params: Value) -> Result<Value, McpError> {
        let params: tools::QueryProceduresParams = serde_json::from_value(params)?;
        let principal = self.principal.clone();
        let result = self.runtime.block_on(tools::handle_query_procedures(
            &mut self.client,
            params,
            &principal,
        ))?;
        Ok(serde_json::to_value(result)?)
    }

    /// Call get_procedure tool
    fn call_get_procedure_tool(&mut self, params: Value) -> Result<Value, McpError> {
        let params: tools::GetProcedureParams = serde_json::from_value(params)?;
        let principal = self.principal.clone();
        let result = self.runtime.block_on(tools::handle_get_procedure(
            &mut self.client,
            params,
            &principal,
        ))?;
        Ok(serde_json::to_value(result)?)
    }

    /// Call report_outcome tool
    fn call_report_outcome_tool(&mut self, params: Value) -> Result<Value, McpError> {
        let params: tools::ReportOutcomeParams = serde_json::from_value(params)?;
        let result = self
            .runtime
            .block_on(tools::handle_report_outcome(&mut self.client, params))?;
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
            description: "Assert a new claim into Boswell with optional confidence and tier"
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "namespace": {"type": "string", "description": "Namespace for the claim"},
                    "subject": {"type": "string", "description": "Subject (entity or concept)"},
                    "predicate": {"type": "string", "description": "Predicate (relationship or attribute)"},
                    "object": {"type": "string", "description": "Object (value or related entity)"},
                    "confidence": {"type": "number", "description": "Confidence score (0.0-1.0)", "minimum": 0.0, "maximum": 1.0},
                    "tier": {"type": "string", "enum": ["ephemeral", "task", "project", "permanent"], "description": "Persistence tier"}
                },
                "required": ["namespace", "subject", "predicate", "object"]
            }),
        }
    }

    fn tool_definition_query(&self) -> ToolDefinition {
        ToolDefinition {
            name: "boswell_query".to_string(),
            description: "Query claims from Boswell with optional filters".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "namespace": {"type": "string", "description": "Filter by namespace"},
                    "subject": {"type": "string", "description": "Filter by subject"},
                    "predicate": {"type": "string", "description": "Filter by predicate"},
                    "object": {"type": "string", "description": "Filter by object"},
                    "min_confidence": {"type": "number", "description": "Minimum confidence threshold"},
                    "tier": {"type": "string", "enum": ["ephemeral", "task", "project", "permanent"]}
                }
            }),
        }
    }

    fn tool_definition_learn(&self) -> ToolDefinition {
        ToolDefinition {
            name: "boswell_learn".to_string(),
            description: "Batch insert multiple claims into Boswell".to_string(),
            input_schema: json!({
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
                                "tier": {"type": "string", "enum": ["ephemeral", "task", "project", "permanent"]}
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
            input_schema: json!({
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
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Search query text"},
                    "namespace": {"type": "string", "description": "Filter by namespace"},
                    "limit": {"type": "integer", "description": "Maximum results (default: 10)", "default": 10},
                    "threshold": {"type": "number", "description": "Minimum similarity filter; 0.0 (default) returns the top `limit` ranked results", "default": 0.0}
                },
                "required": ["query"]
            }),
        }
    }

    // ---- Procedural memory (design 15 §3.2, §3.3, §4.1) ----
    //
    // The descriptions carry the obligation, not just the capability: a model
    // reading `tools/list` has to learn that retrieving a procedure creates a
    // receipt it owes a report on, because nothing else on this surface will
    // tell it.

    fn tool_definition_query_goals(&self) -> ToolDefinition {
        ToolDefinition {
            name: "boswell_query_goals".to_string(),
            description: "Find goals by namespace or intent — the entry hop into a decomposition. \
                          Traversal is free: no execution receipt is issued and nothing is owed."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "namespace": {"type": "string", "description": "Filter by namespace prefix"},
                    "intent_contains": {"type": "string", "description": "Case-insensitive substring of the goal's intent"},
                    "limit": {"type": "integer", "description": "Maximum number of results"}
                }
            }),
        }
    }

    fn tool_definition_get_goal(&self) -> ToolDefinition {
        ToolDefinition {
            name: "boswell_get_goal".to_string(),
            description: "Fetch one goal by id. Returns found=false when no such goal exists in \
                          scope. Issues no receipt."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": {"type": "string", "description": "Goal id"},
                    "namespace_scope": {"type": "string", "description": "Confine the lookup to a namespace prefix"}
                },
                "required": ["id"]
            }),
        }
    }

    fn tool_definition_expand_goal(&self) -> ToolDefinition {
        ToolDefinition {
            name: "boswell_expand_goal".to_string(),
            description: "Expand one goal into its ranked candidate children — a single traversal \
                          hop. Returns candidates whose preconditions currently hold, the \
                          decide-role procedures that help choose among them, and the claim \
                          readings behind the filtering. The store surfaces; you decide, and you \
                          hold the cursor: call this again on whichever child you pick."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "goal_id": {"type": "string", "description": "The goal to expand"},
                    "context_tags": {
                        "type": "array",
                        "description": "Situational tags matched against edge tags, e.g. [\"time:quick\"]",
                        "items": {"type": "string"}
                    },
                    "namespace_scope": {"type": "string", "description": "Confine the lookup to a namespace prefix"}
                },
                "required": ["goal_id"]
            }),
        }
    }

    fn tool_definition_query_procedures(&self) -> ToolDefinition {
        ToolDefinition {
            name: "boswell_query_procedures".to_string(),
            description: "Retrieve stored how-tos for a goal or intent. Every procedure returned \
                          carries an execution receipt: report the outcome with \
                          boswell_report_outcome before the receipt expires, or the run counts as \
                          unknown against the procedure. Do not retrieve procedures you do not \
                          intend to use."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "namespace": {"type": "string", "description": "Filter by namespace prefix"},
                    "goal": {"type": "string", "description": "Filter to a single goal grouping key"},
                    "intent_contains": {"type": "string", "description": "Case-insensitive substring of the procedure's intent"},
                    "include_superseded": {"type": "boolean", "description": "Include non-current versions", "default": false},
                    "limit": {"type": "integer", "description": "Maximum number of results"},
                    "task_id": {"type": "string", "description": "Correlation id stamped onto the issued receipts"},
                    "session_id": {"type": "string", "description": "Correlation id stamped onto the issued receipts"}
                }
            }),
        }
    }

    fn tool_definition_get_procedure(&self) -> ToolDefinition {
        ToolDefinition {
            name: "boswell_get_procedure".to_string(),
            description: "Fetch one procedure by id, issuing an execution receipt for it. Returns \
                          found=false when no such procedure exists in scope — and an \
                          out-of-scope lookup leaves no receipt behind. As with \
                          boswell_query_procedures, a returned procedure must be reported on."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": {"type": "string", "description": "Procedure id"},
                    "namespace_scope": {"type": "string", "description": "Confine the lookup to a namespace prefix"},
                    "task_id": {"type": "string", "description": "Correlation id stamped onto the issued receipt"},
                    "session_id": {"type": "string", "description": "Correlation id stamped onto the issued receipt"}
                },
                "required": ["id"]
            }),
        }
    }

    fn tool_definition_report_outcome(&self) -> ToolDefinition {
        ToolDefinition {
            name: "boswell_report_outcome".to_string(),
            description: "Answer an outstanding execution receipt. This is how a procedure's \
                          effectiveness is learned; silence is not success, so report failures \
                          and abandonments too."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "receipt_id": {"type": "string", "description": "The receipt this report answers"},
                    "outcome": {"type": "string", "enum": ["success", "failure", "abandoned"], "description": "What happened"},
                    "failure_mode": {
                        "type": "string",
                        "enum": ["preconditions_stale", "step_failed", "bad_result", "executor_error"],
                        "description": "Failure attribution; only valid when outcome is failure"
                    },
                    "failed_step": {"type": "string", "description": "The step that failed, when failure_mode is step_failed"},
                    "executor_confidence": {"type": "number", "description": "Your self-assessed confidence (0.0-1.0)", "minimum": 0.0, "maximum": 1.0},
                    "cost": {"type": "number", "description": "Reported cost; units are executor-defined"},
                    "notes": {"type": "string", "description": "Free-form notes"}
                },
                "required": ["receipt_id", "outcome"]
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `new()` builds a runtime + SDK client but performs no network I/O until
    /// `connect()`, so it is safe to construct offline for handler tests. These
    /// tests drive the real `handle_request` dispatch and handler code — no live
    /// Router/instance is needed for `initialize`, `tools/list`, or the error
    /// paths (those never touch the client).
    fn test_server() -> McpServer {
        McpServer::new("http://127.0.0.1:8080".to_string()).unwrap()
    }

    fn request(method: &str, params: Value) -> JsonRpcRequest {
        serde_json::from_value(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        }))
        .expect("valid JsonRpcRequest")
    }

    #[test]
    fn test_initialize_reports_protocol_and_server_info() {
        let mut server = test_server();
        let resp = server.handle_request(request("initialize", json!({})));
        let result = &resp["result"];
        assert_eq!(result["protocolVersion"], "0.1.0");
        assert_eq!(result["serverInfo"]["name"], "boswell-mcp");
        assert_eq!(result["capabilities"]["tools"]["supported"], true);
    }

    fn advertised_tools(server: &mut McpServer) -> Vec<Value> {
        let resp = server.handle_request(request("tools/list", json!({})));
        resp["result"]["tools"]
            .as_array()
            .expect("tools should be an array")
            .clone()
    }

    #[test]
    fn test_tools_list_advertises_every_tool() {
        let mut server = test_server();
        let tools = advertised_tools(&mut server);
        let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();

        for expected in [
            "boswell_assert",
            "boswell_query",
            "boswell_learn",
            "boswell_forget",
            "boswell_semantic_search",
            "boswell_query_goals",
            "boswell_get_goal",
            "boswell_expand_goal",
            "boswell_query_procedures",
            "boswell_get_procedure",
            "boswell_report_outcome",
        ] {
            assert!(names.contains(&expected), "missing tool: {}", expected);
        }
        assert_eq!(tools.len(), 11, "advertised: {:?}", names);
    }

    #[test]
    fn test_every_advertised_tool_is_dispatchable() {
        // A tool in `tools/list` that `tools/call` does not route is a surface
        // that lies. Each call below reaches its handler with empty arguments,
        // so it fails on parameters or on the absent connection — never with
        // "Tool not found".
        let mut server = test_server();
        let names: Vec<String> = advertised_tools(&mut server)
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect();

        for name in names {
            let resp = server.handle_request(request(
                "tools/call",
                json!({ "name": name, "arguments": {} }),
            ));
            let message = resp["error"]["message"].as_str().unwrap_or_default();
            assert!(
                !message.contains("Tool not found"),
                "{} is advertised but not routed",
                name
            );
        }
    }

    #[test]
    fn test_procedure_tools_do_not_take_a_principal() {
        // `issued_to` is the server's to name (design 15 §3.3). If it ever
        // appears in an advertised schema, a model can put someone else on the
        // hook for a receipt it took out.
        let mut server = test_server();
        for tool in advertised_tools(&mut server) {
            let properties = &tool["inputSchema"]["properties"];
            assert!(
                properties.get("issued_to").is_none(),
                "{} advertises issued_to",
                tool["name"]
            );
        }
    }

    #[test]
    fn test_retrieval_tools_advertise_the_reporting_obligation() {
        // The receipt is the whole point of procedure retrieval, and
        // `tools/list` is the only place a model learns about it.
        let mut server = test_server();
        let tools = advertised_tools(&mut server);
        let described = |name: &str| -> String {
            tools
                .iter()
                .find(|t| t["name"] == name)
                .expect("tool should be advertised")["description"]
                .as_str()
                .unwrap()
                .to_string()
        };

        assert!(described("boswell_query_procedures").contains("receipt"));
        assert!(described("boswell_get_procedure").contains("receipt"));
        assert!(described("boswell_query_goals").contains("no execution receipt"));
    }

    #[test]
    fn test_principal_defaults_and_can_be_named() {
        let server = test_server();
        assert_eq!(server.principal, DEFAULT_PRINCIPAL);

        let named = server.with_principal("agent:jd").unwrap();
        assert_eq!(named.principal, "agent:jd");
    }

    #[test]
    fn test_empty_principal_is_refused() {
        // A receipt issued to nobody is not a contract.
        match test_server().with_principal("   ") {
            Err(McpError::InvalidRequest(_)) => {}
            Err(e) => panic!("wrong error: {}", e),
            Ok(_) => panic!("an empty principal should be refused"),
        }
    }

    #[test]
    fn test_tools_list_uses_valid_lowercase_tier_enum() {
        // Regression guard: the advertised tier enum must be the real domain
        // tiers, not the old bogus "Transient"/"Session"/"Permanent" values.
        let mut server = test_server();
        let resp = server.handle_request(request("tools/list", json!({})));
        let serialized = resp.to_string();
        for tier in ["ephemeral", "task", "project", "permanent"] {
            assert!(
                serialized.contains(&format!("\"{}\"", tier)),
                "tier enum should advertise '{}'",
                tier
            );
        }
        assert!(!serialized.contains("Transient"));
        assert!(!serialized.contains("Session"));
        assert!(!serialized.contains("\"Permanent\""));
    }

    #[test]
    fn test_unknown_method_returns_method_not_found() {
        let mut server = test_server();
        let resp = server.handle_request(request("does/not/exist", json!({})));
        assert_eq!(resp["error"]["code"], -32601);
    }

    #[test]
    fn test_tool_call_unknown_tool_returns_not_found() {
        let mut server = test_server();
        let resp = server.handle_request(request(
            "tools/call",
            json!({ "name": "boswell_nonexistent", "arguments": {} }),
        ));
        assert_eq!(resp["error"]["code"], -32601);
        assert!(resp["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Tool not found"));
    }

    #[test]
    fn test_tool_call_missing_name_returns_invalid_params() {
        let mut server = test_server();
        let resp = server.handle_request(request("tools/call", json!({ "arguments": {} })));
        assert_eq!(resp["error"]["code"], -32602);
    }
}
