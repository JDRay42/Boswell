//! Boswell MCP Server
//!
//! Model Context Protocol server for integrating Boswell with AI clients
//! (Claude Desktop, Cline, etc.).
//!
//! Claim tools:
//! - `boswell_assert` - Assert new claims
//! - `boswell_query` - Query claims with filters
//! - `boswell_learn` - Batch insert claims
//! - `boswell_forget` - Remove claims
//! - `boswell_semantic_search` - Semantic search with embeddings
//!
//! Procedural memory tools (design 15):
//! - `boswell_query_goals` - Find goals by namespace or intent
//! - `boswell_get_goal` - Fetch one goal
//! - `boswell_expand_goal` - One traversal hop into a goal's children
//! - `boswell_query_procedures` - Retrieve how-tos; issues execution receipts
//! - `boswell_get_procedure` - Fetch one procedure; issues a receipt
//! - `boswell_report_outcome` - Answer an outstanding receipt
//!
//! Retrieving a procedure creates an obligation to report on it, and the
//! principal on that obligation is the server's — see
//! [`McpServer::with_principal`]. Goal traversal issues no receipt.
//!
//! # Example
//!
//! ```no_run
//! use boswell_mcp::McpServer;
//!
//! let mut server = McpServer::new("http://localhost:8080".to_string()).unwrap();
//! server.connect().unwrap();
//! server.run().unwrap();
//! ```

#![warn(missing_docs)]

mod error;
mod protocol;
mod server;
mod tools;

pub use error::McpError;
pub use server::{McpServer, DEFAULT_PRINCIPAL};
