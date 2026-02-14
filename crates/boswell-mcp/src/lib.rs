//! Boswell MCP Server
//!
//! Model Context Protocol server for integrating Boswell with AI clients
//! (Claude Desktop, Cline, etc.).
//!
//! Provides 5 MCP tools:
//! - `boswell_assert` - Assert new claims
//! - `boswell_query` - Query claims with filters
//! - `boswell_learn` - Batch insert claims
//! - `boswell_forget` - Remove claims
//! - `boswell_semantic_search` - Semantic search with embeddings
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
pub use server::McpServer;
