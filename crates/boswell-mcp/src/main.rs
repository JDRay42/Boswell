//! Boswell MCP Server - Main entry point

use boswell_mcp::McpServer;
use std::env;
use tracing::Level;
use tracing_subscriber;

fn main() {
    // Initialize tracing (log to stderr)
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_max_level(Level::INFO)
        .init();

    // Get router URL from environment or use default
    let router_url = env::var("BOSWELL_ROUTER")
        .unwrap_or_else(|_| "http://localhost:8080".to_string());

    // Create and start MCP server
    let mut server = match McpServer::new(router_url.clone()) {
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
