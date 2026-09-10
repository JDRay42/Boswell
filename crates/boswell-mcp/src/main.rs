//! Boswell MCP Server - Main entry point

use boswell_mcp::{McpServer, DEFAULT_PRINCIPAL};
use std::env;
use tracing::Level;

fn main() {
    // Initialize tracing (log to stderr)
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_max_level(Level::INFO)
        .init();

    // Get router URL from environment or use default
    let router_url =
        env::var("BOSWELL_ROUTER").unwrap_or_else(|_| "http://localhost:8080".to_string());

    // Who is on the hook for the execution receipts this server takes out.
    // It is deliberately not a tool argument: a model that names itself on a
    // receipt is not accountable for answering it (design 15 §3.3).
    let principal =
        env::var("BOSWELL_MCP_PRINCIPAL").unwrap_or_else(|_| DEFAULT_PRINCIPAL.to_string());

    // Create and start MCP server
    let mut server =
        match McpServer::new(router_url.clone()).and_then(|s| s.with_principal(principal)) {
            Ok(server) => server,
            Err(e) => {
                eprintln!("Failed to create MCP server: {}", e);
                std::process::exit(1);
            }
        };

    // Connect to Boswell
    if let Err(e) = server.connect() {
        eprintln!("Failed to connect to Boswell at {}: {}", router_url, e);
        std::process::exit(1);
    }

    // Run server (blocks until stdin closes)
    if let Err(e) = server.run() {
        eprintln!("MCP server error: {}", e);
        std::process::exit(1);
    }
}
