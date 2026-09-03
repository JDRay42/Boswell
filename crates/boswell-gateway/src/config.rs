//! Gateway configuration, loaded from a TOML file (see `config/gateway.toml`).
//!
//! API keys are stored as **SHA-256 hashes**, never in plaintext. A client
//! presents the raw key as `Authorization: Bearer <key>`; the gateway hashes it
//! and matches against `key_hash`.

use serde::Deserialize;
use std::path::Path;
use thiserror::Error;

/// Errors that can occur while loading gateway configuration.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// The config file could not be read.
    #[error("Failed to read config file: {0}")]
    FileRead(#[from] std::io::Error),

    /// The config file was not valid TOML for this schema.
    #[error("Failed to parse config TOML: {0}")]
    TomlParse(#[from] toml::de::Error),
}

/// Top-level gateway configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct GatewayConfig {
    /// Address the HTTP server binds to. Defaults to `127.0.0.1` (localhost
    /// only); TLS and public reach are provided by a reverse proxy or tunnel.
    pub bind_address: String,

    /// Port the HTTP server binds to.
    pub bind_port: u16,

    /// Router endpoint the internal SDK client uses to establish a session and
    /// reach the private gRPC instance.
    pub router_endpoint: String,

    /// Maximum request body size in bytes.
    pub max_body_bytes: usize,

    /// Per-request timeout in seconds.
    pub request_timeout_secs: u64,

    /// Per-key rate limit in requests per minute. `0` disables rate limiting.
    pub rate_limit_per_minute: u32,

    /// Registered API keys, each bound to a namespace and a set of scopes.
    #[serde(default)]
    pub api_keys: Vec<ApiKeyConfig>,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            bind_address: "127.0.0.1".to_string(),
            bind_port: 8081,
            router_endpoint: "http://127.0.0.1:8080".to_string(),
            max_body_bytes: 1024 * 1024, // 1 MiB
            request_timeout_secs: 30,
            rate_limit_per_minute: 120,
            api_keys: Vec::new(),
        }
    }
}

/// A single API key entry. The raw key is never stored — only its SHA-256 hash.
#[derive(Debug, Clone, Deserialize)]
pub struct ApiKeyConfig {
    /// Stable identifier for the key (used in audit logs; never the secret).
    pub id: String,

    /// Lowercase hex SHA-256 of the raw bearer key.
    pub key_hash: String,

    /// Namespace this key is scoped to. Empty or `"*"` means unrestricted.
    #[serde(default)]
    pub namespace: String,

    /// Granted scopes: any of `read`, `write`, `delete`.
    #[serde(default)]
    pub scopes: Vec<String>,
}

impl GatewayConfig {
    /// Load configuration from a TOML file.
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let contents = std::fs::read_to_string(path)?;
        let config: GatewayConfig = toml::from_str(&contents)?;
        Ok(config)
    }

    /// Full `address:port` the server binds to.
    pub fn bind_addr(&self) -> String {
        format!("{}:{}", self.bind_address, self.bind_port)
    }

    /// A commented starter configuration, written by `boswell-gateway init`.
    pub fn starter_toml() -> &'static str {
        STARTER_TOML
    }
}

/// Commented starter config emitted by the `init` subcommand.
pub const STARTER_TOML: &str = r#"# Boswell public HTTP API gateway configuration
#
# The gateway serves plain HTTP on localhost. Put a reverse proxy or tunnel in
# front of it for TLS and public reach; keep the gRPC instance private.

# Address and port the HTTP server binds to.
bind_address = "127.0.0.1"
bind_port = 8081

# Router endpoint used internally to establish a session and reach the instance.
router_endpoint = "http://127.0.0.1:8080"

# Hardening.
max_body_bytes = 1048576      # 1 MiB request-body cap
request_timeout_secs = 30
rate_limit_per_minute = 120   # per key; 0 disables

# API keys. Store the SHA-256 hash of each key, never the raw key. Generate one:
#   KEY=$(openssl rand -hex 32); echo "raw:   $KEY"; \
#     printf '%s' "$KEY" | sha256sum | cut -d' ' -f1
# Give the raw KEY to the client (Authorization: Bearer <key>); put the hash here.
#
# namespace: the key may only read/write within this namespace (or its children,
#            i.e. "<namespace>:..."). Empty or "*" means unrestricted.
# scopes:    any of "read", "write", "delete".
[[api_keys]]
id = "example-agent"
key_hash = "0000000000000000000000000000000000000000000000000000000000000000"
namespace = "agent"
scopes = ["read", "write"]
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_defaults() {
        let c = GatewayConfig::default();
        assert_eq!(c.bind_address, "127.0.0.1");
        assert_eq!(c.bind_port, 8081);
        assert_eq!(c.router_endpoint, "http://127.0.0.1:8080");
        assert!(c.api_keys.is_empty());
    }

    #[test]
    fn test_parse_partial_uses_defaults() {
        let toml = r#"
            bind_port = 9099
            [[api_keys]]
            id = "k1"
            key_hash = "abc"
            namespace = "team"
            scopes = ["read"]
        "#;
        let c: GatewayConfig = toml::from_str(toml).unwrap();
        assert_eq!(c.bind_port, 9099);
        assert_eq!(c.bind_address, "127.0.0.1"); // default preserved
        assert_eq!(c.api_keys.len(), 1);
        assert_eq!(c.api_keys[0].id, "k1");
        assert_eq!(c.api_keys[0].scopes, vec!["read"]);
    }

    #[test]
    fn test_starter_toml_is_valid() {
        let c: GatewayConfig = toml::from_str(GatewayConfig::starter_toml()).unwrap();
        assert_eq!(c.api_keys.len(), 1);
        assert_eq!(c.api_keys[0].namespace, "agent");
    }
}
