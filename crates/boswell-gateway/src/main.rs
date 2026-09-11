//! Boswell public HTTP API gateway CLI.
//!
//! Serves the authenticated `/v1` HTTP/JSON API on localhost, front-ending the
//! private gRPC instance via the in-repo SDK.

use std::env;
use std::process;

use boswell_gateway::config::STARTER_TOML;
use boswell_gateway::{run, GatewayConfig, GatewayError};

#[tokio::main]
async fn main() {
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

async fn dispatch() -> Result<(), GatewayError> {
    let args: Vec<String> = env::args().collect();

    match args.get(1).map(String::as_str) {
        Some("--help") | Some("-h") => {
            print_help();
            Ok(())
        }
        Some("init") => {
            let path = args
                .get(2)
                .map(String::as_str)
                .unwrap_or("config/gateway.toml");
            init_config(path)
        }
        Some("keygen") => {
            keygen();
            Ok(())
        }
        Some("--config") => {
            let path = args.get(2).ok_or_else(|| {
                GatewayError::Serve("--config requires a path argument".to_string())
            })?;
            let config = GatewayConfig::from_file(path)?;
            run(config).await
        }
        None => {
            eprintln!("Warning: no --config specified; using built-in defaults");
            eprintln!("         (127.0.0.1:8081, router http://127.0.0.1:8080, no API keys)");
            eprintln!("         Run `boswell-gateway --help` for options.\n");
            run(GatewayConfig::default()).await
        }
        Some(other) => {
            eprintln!("Unknown argument: {other}\n");
            print_help();
            process::exit(2);
        }
    }
}

/// Write a starter config file, refusing to overwrite an existing one.
fn init_config(path: &str) -> Result<(), GatewayError> {
    if std::path::Path::new(path).exists() {
        return Err(GatewayError::Serve(format!(
            "refusing to overwrite existing file: {path}"
        )));
    }
    if let Some(parent) = std::path::Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| GatewayError::Serve(format!("failed to create {parent:?}: {e}")))?;
        }
    }
    std::fs::write(path, STARTER_TOML)
        .map_err(|e| GatewayError::Serve(format!("failed to write {path}: {e}")))?;
    println!("Wrote starter configuration to {path}");
    println!("Edit it (add API-key hashes), then run: boswell-gateway --config {path}");
    Ok(())
}

/// Print a fresh Ed25519 root key pair for the `[tokens]` section.
///
/// To stdout and nowhere else — the gateway does not write the key into a config
/// file, because the file it would write to is the one an operator is most
/// likely to commit.
fn keygen() {
    let keypair = biscuit_auth::KeyPair::new_with_algorithm(biscuit_auth::Algorithm::Ed25519);
    println!("# Attenuable-token root key (ADR-022). Keep the private half secret:");
    println!("# anything holding it can mint a token for any principal.");
    println!("[tokens]");
    println!(
        "root_private_key = \"{}\"",
        keypair.private().to_bytes_hex()
    );
    println!();
    println!("# Public half, for reference. A verifier needs only this, and it cannot mint.");
    println!("# public_key = \"{}\"", keypair.public().to_bytes_hex());
}

fn print_help() {
    println!("Boswell Gateway - public HTTP/JSON API in front of the private gRPC instance");
    println!();
    println!("USAGE:");
    println!("    boswell-gateway --config <path>  Start the gateway with a config file");
    println!("    boswell-gateway init [path]      Write a starter config (default: config/gateway.toml)");
    println!("    boswell-gateway keygen           Print a new attenuable-token root key pair");
    println!("    boswell-gateway                  Start with built-in defaults (no API keys)");
    println!("    boswell-gateway --help           Print this help");
    println!();
    println!("CONFIGURATION (TOML):");
    println!("    bind_address, bind_port          HTTP listen address (default 127.0.0.1:8081)");
    println!("    router_endpoint                  Router URL for the SDK (default http://127.0.0.1:8080)");
    println!("    max_body_bytes, request_timeout_secs, rate_limit_per_minute   hardening knobs");
    println!("    [[api_keys]] id, key_hash, namespace, scopes                  bearer keys (hashes only)");
    println!("    [oidc] issuer, principals                                     identity-provider tokens");
    println!("    [tokens] root_private_key, default_ttl_secs, max_ttl_secs     attenuable tokens");
    println!();
    println!("SECURITY:");
    println!("    Serves plain HTTP on localhost. Put a reverse proxy or tunnel in front for TLS");
    println!("    and public reach; keep the gRPC instance bound to 127.0.0.1.");
}
