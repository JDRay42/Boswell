//! Boswell Router CLI
//!
//! Starts the Router HTTP server for session management and instance routing.

use boswell_router::{config::RouterConfig, start_server, RouterError};
use std::env;
use std::process;

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("Error: {}", e);
        process::exit(1);
    }
}

async fn run() -> Result<(), RouterError> {
    // Parse command-line arguments
    let args: Vec<String> = env::args().collect();

    let config = if args.len() > 2 && args[1] == "--config" {
        // Load from specified config file
        let config_path = &args[2];
        RouterConfig::from_file(config_path)?
    } else if args.len() > 1 && args[1] == "--help" {
        print_help();
        process::exit(0);
    } else {
        // Use default test configuration
        eprintln!("Warning: No config file specified, using default test configuration");
        eprintln!("Usage: boswell-router --config <path-to-config.toml>");
        eprintln!();
        RouterConfig::default_test_config()
    };

    // Start the server
    start_server(config).await?;

    Ok(())
}

fn print_help() {
    println!("Boswell Router - Session Management and Instance Registry");
    println!();
    println!("USAGE:");
    println!("    boswell-router --config <path-to-config.toml>");
    println!();
    println!("OPTIONS:");
    println!("    --config <file>    Load configuration from TOML file");
    println!("    --help             Print this help message");
    println!();
    println!("EXAMPLE:");
    println!("    boswell-router --config config/router.toml");
    println!();
    println!("CONFIGURATION:");
    println!("    The TOML config file should contain:");
    println!("    - bind_address: IP address to bind (e.g., '127.0.0.1')");
    println!("    - bind_port: Port number (e.g., 8080)");
    println!("    - jwt_secret: Secret key for JWT token signing");
    println!("    - token_expiry_secs: Token expiry in seconds (default: 3600)");
    println!("    - instances: Array of registered gRPC service instances");
    println!();
}
