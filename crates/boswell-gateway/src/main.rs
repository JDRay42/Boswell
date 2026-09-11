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
        Some("revocation-ids") => {
            let token = args.get(2).ok_or_else(|| {
                GatewayError::Serve("revocation-ids requires a token argument".to_string())
            })?;
            revocation_ids(token)
        }
        Some("revoke") => {
            let path = args.get(2).ok_or_else(|| {
                GatewayError::Serve(
                    "revoke requires a revocation-list path and a token or identifier".to_string(),
                )
            })?;
            let subject = args.get(3).ok_or_else(|| {
                GatewayError::Serve("revoke requires a token or identifier".to_string())
            })?;
            let note = args
                .get(4..)
                .filter(|rest| !rest.is_empty())
                .map(|rest| rest.join(" "));
            revoke(path, subject, note.as_deref())
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

/// Print every revocation identifier of a token, one per line.
///
/// The token is parsed but not verified, so this works without the root key —
/// an operator handed a token to revoke should not need the gateway's secret to
/// name it. The first line is the authority block: revoking it ends the root
/// grant and every token attenuated from it. Each line after it belongs to one
/// attenuation, and revoking that one ends that delegate alone.
fn revocation_ids(token: &str) -> Result<(), GatewayError> {
    let ids = boswell_gateway::tokens::revocation_ids(token.trim())
        .map_err(|e| GatewayError::Serve(format!("could not read the token: {e}")))?;

    for (index, id) in ids.iter().enumerate() {
        let role = if index == 0 {
            "root grant (revoking this ends every token attenuated from it)"
        } else {
            "attenuation block"
        };
        println!("{id}  # block {index}: {role}");
    }
    Ok(())
}

/// Append a token's revocation identifier to a revocation list file.
///
/// `subject` is either a token or a bare hex identifier. A token is parsed but
/// not verified — the same reasoning as `revocation-ids`: whoever has to revoke
/// a grant should not need the gateway's root key to name it.
///
/// Given a token, the identifier written is its **last** block. That is the one
/// unique to the token in hand, so revoking it ends that token and anything
/// attenuated from it, and leaves the parent it was narrowed from working. The
/// authority block would end the whole tree, which is a bigger decision than
/// "revoke this token" and so has to be asked for by id.
fn revoke(path: &str, subject: &str, note: Option<&str>) -> Result<(), GatewayError> {
    let subject = subject.trim();
    let path = std::path::Path::new(path);

    let (id, scope) = match boswell_gateway::tokens::revocation_ids(subject) {
        Ok(ids) => {
            let last = ids
                .last()
                .ok_or_else(|| GatewayError::Serve("the token has no blocks".to_string()))?
                .clone();
            let scope = if ids.len() == 1 {
                "the root grant, and every token attenuated from it".to_string()
            } else {
                format!(
                    "block {} of {}, and anything attenuated from it; \n\
                     to end the whole tree instead, revoke the root grant: {}",
                    ids.len() - 1,
                    ids.len(),
                    ids[0]
                )
            };
            (last, scope)
        }
        // Not a token, so take it for what the file holds: a bare identifier.
        // `append` rejects it if it is not hex either.
        Err(_) => (
            subject.to_string(),
            "the block carrying this id".to_string(),
        ),
    };

    // Lowercase so what is printed is what the file holds.
    let id = id.to_ascii_lowercase();

    match boswell_gateway::revocation::append(path, &id, note)
        .map_err(|e| GatewayError::Serve(e.to_string()))?
    {
        boswell_gateway::revocation::Appended::Added => {
            println!("Revoked {id}");
            println!("  in     {}", path.display());
            println!("  ends   {scope}");
            println!();
            println!("A running gateway picks this up within revocation_refresh_secs.");
            println!(
                "It only takes effect if this file is the one [tokens] revocation_list_path names."
            );
        }
        boswell_gateway::revocation::Appended::AlreadyPresent => {
            println!("Already revoked: {id}");
            println!("  in     {}", path.display());
        }
    }
    Ok(())
}

fn print_help() {
    println!("Boswell Gateway - public HTTP/JSON API in front of the private gRPC instance");
    println!();
    println!("USAGE:");
    println!("    boswell-gateway --config <path>  Start the gateway with a config file");
    println!("    boswell-gateway init [path]      Write a starter config (default: config/gateway.toml)");
    println!("    boswell-gateway keygen           Print a new attenuable-token root key pair");
    println!("    boswell-gateway revocation-ids <token>");
    println!("                                     Print a token's revocation identifiers");
    println!("    boswell-gateway revoke <list-path> <token|id> [note...]");
    println!("                                     Append a revocation to the list, creating it if absent");
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
    println!(
        "    [tokens] revocation_list_path, revocation_refresh_secs        the revocation file"
    );
    println!();
    println!("SECURITY:");
    println!("    Serves plain HTTP on localhost. Put a reverse proxy or tunnel in front for TLS");
    println!("    and public reach; keep the gRPC instance bound to 127.0.0.1.");
}

#[cfg(test)]
mod tests {
    use super::*;
    use boswell_gateway::auth::{AuthContext, Scope};
    use boswell_gateway::config::TokenConfig;
    use boswell_gateway::tokens::{
        attenuate, revocation_ids as ids_of, Attenuation, TokenAuthority,
    };

    /// A root token and a delegate narrowed from it. No revocation list is
    /// configured: these tests are about what `revoke` writes, not about what
    /// the gateway then does with it — `tokens.rs` covers that end.
    fn root_and_delegate() -> (String, String) {
        let keypair = biscuit_auth::KeyPair::new_with_algorithm(biscuit_auth::Algorithm::Ed25519);
        let authority = TokenAuthority::new(&TokenConfig {
            root_private_key: keypair.private().to_bytes_hex(),
            default_ttl_secs: 3600,
            max_ttl_secs: 86400,
            revocation_list_path: String::new(),
            revocation_refresh_secs: 0,
        })
        .expect("a freshly generated key should load");

        let context = AuthContext {
            key_id: "agent".to_string(),
            namespace: "team".to_string(),
            scopes: [Scope::Read].into_iter().collect(),
            token: None,
        };
        let root = authority.mint(&context, None).expect("mint");
        let delegate = attenuate(
            &root.token,
            keypair.public(),
            &Attenuation {
                namespace: Some("team:sub".to_string()),
                ..Attenuation::default()
            },
        )
        .expect("attenuate");

        (root.token, delegate)
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "boswell-main-revoke-{}-{}-{:?}.txt",
            name,
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_file(&path);
        path
    }

    /// The identifiers a file holds, comments stripped.
    fn listed(path: &std::path::Path) -> Vec<String> {
        std::fs::read_to_string(path)
            .expect("read the list")
            .lines()
            .map(|line| line.split('#').next().unwrap_or("").trim().to_string())
            .filter(|line| !line.is_empty())
            .collect()
    }

    #[test]
    fn revoking_a_delegate_writes_its_own_block_and_not_its_root_s() {
        // The whole reason `revoke` takes the *last* id. Writing the first one
        // would end the root and every sibling delegate along with it, which is
        // not what "revoke this token" means.
        let (_root, delegate) = root_and_delegate();
        let ids = ids_of(&delegate).expect("the delegate's ids");
        assert_eq!(ids.len(), 2, "authority block plus one attenuation");

        let path = scratch("delegate");
        revoke(path.to_str().unwrap(), &delegate, None).expect("revoke");

        assert_eq!(listed(&path), vec![ids[1].clone()]);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn revoking_a_root_token_writes_the_authority_block() {
        // A root has one block, so its last id *is* its first, and revoking it
        // ends the subtree. That is correct here and stated in the output.
        let (root, _delegate) = root_and_delegate();
        let ids = ids_of(&root).expect("the root's ids");
        assert_eq!(ids.len(), 1);

        let path = scratch("root");
        revoke(path.to_str().unwrap(), &root, Some("leaked laptop")).expect("revoke");

        assert_eq!(listed(&path), vec![ids[0].clone()]);
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("# leaked laptop"),
            "the note survives as a comment"
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_bare_identifier_is_taken_as_one_when_it_is_not_a_token() {
        let path = scratch("bare");
        revoke(path.to_str().unwrap(), "  00112233AABB  ", None).expect("revoke");
        assert_eq!(listed(&path), vec!["00112233aabb".to_string()]);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn an_argument_that_is_neither_a_token_nor_hex_is_refused() {
        let path = scratch("garbage");
        assert!(revoke(path.to_str().unwrap(), "not-a-token", None).is_err());
        assert!(!path.exists(), "nothing is written for a refused argument");
    }
}
