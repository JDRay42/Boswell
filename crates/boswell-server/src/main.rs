//! Boswell instance server CLI.
//!
//! Starts the gRPC instance server that serves the Boswell API, backed by a
//! SQLite store and (by default) a local Ollama embedder.

use std::env;
use std::process;

use boswell_server::{config::STARTER_TOML, run, InstanceConfig, ServerError};

#[tokio::main]
async fn main() {
    // Respect RUST_LOG; default to info-level output.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    if let Err(e) = dispatch().await {
        eprintln!("Error: {}", e);
        process::exit(1);
    }
}

async fn dispatch() -> Result<(), ServerError> {
    let args: Vec<String> = env::args().collect();

    match args.get(1).map(String::as_str) {
        Some("--help") | Some("-h") => {
            print_help();
            Ok(())
        }
        Some("init") => {
            let path = args.get(2).map(String::as_str).unwrap_or("config/instance.toml");
            init_config(path)
        }
        Some("--config") => {
            let path = args.get(2).ok_or_else(|| {
                ServerError::Serve("--config requires a path argument".to_string())
            })?;
            let config = InstanceConfig::from_file(path)?;
            run(config).await
        }
        None => {
            eprintln!("Warning: no --config specified; using built-in defaults");
            eprintln!("         (Ollama embedder 'embeddinggemma', 127.0.0.1:50051)");
            eprintln!("         Run `boswell-server --help` for options.\n");
            run(InstanceConfig::default()).await
        }
        Some(other) => {
            eprintln!("Unknown argument: {other}\n");
            print_help();
            process::exit(2);
        }
    }
}

/// Write a starter config file, refusing to overwrite an existing one.
fn init_config(path: &str) -> Result<(), ServerError> {
    if std::path::Path::new(path).exists() {
        return Err(ServerError::Serve(format!(
            "refusing to overwrite existing file: {path}"
        )));
    }
    if let Some(parent) = std::path::Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| ServerError::Serve(format!("failed to create {parent:?}: {e}")))?;
        }
    }
    std::fs::write(path, STARTER_TOML)
        .map_err(|e| ServerError::Serve(format!("failed to write {path}: {e}")))?;
    println!("Wrote starter configuration to {path}");
    println!("Edit it, then run: boswell-server --config {path}");
    Ok(())
}

fn print_help() {
    println!("Boswell Instance Server - serves the Boswell gRPC API");
    println!();
    println!("USAGE:");
    println!("    boswell-server --config <path>   Start the server with a config file");
    println!("    boswell-server init [path]       Write a starter config (default: config/instance.toml)");
    println!("    boswell-server                   Start with built-in defaults");
    println!("    boswell-server --help            Print this help");
    println!();
    println!("CONFIGURATION (TOML):");
    println!("    bind_address, bind_port          gRPC listen address (default 127.0.0.1:50051)");
    println!("    [storage] db_path                SQLite path (default boswell.db; ':memory:' for ephemeral)");
    println!("    [embedding] backend              'ollama' | 'mock' | 'none' (default ollama)");
    println!("    [embedding] model, endpoint      Ollama model + endpoint (default embeddinggemma @ localhost:11434)");
    println!();
    println!("NOTE:");
    println!("    The 'ollama' backend requires a running Ollama with the model pulled:");
    println!("        ollama pull embeddinggemma");
}
