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

    /// OIDC bearer-token verification. Absent means the gateway accepts API
    /// keys only.
    #[serde(default)]
    pub oidc: Option<OidcConfig>,
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
            oidc: None,
        }
    }
}

/// OIDC verification settings (ADR-022).
///
/// Boswell ships no identity provider. This section names one the operator
/// already runs, and the subjects it is willing to accept from it.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct OidcConfig {
    /// Issuer URL, matched against the token's `iss` claim. A trailing slash is
    /// ignored. Also the base for discovery when `jwks_uri` is empty.
    pub issuer: String,

    /// Accepted `aud` values. Empty disables the audience check, which is only
    /// right when the provider issues tokens for this gateway alone.
    pub audiences: Vec<String>,

    /// Key-set URL. Empty means fetch it from the issuer's
    /// `/.well-known/openid-configuration` on first use.
    pub jwks_uri: String,

    /// How long a fetched key set is used before it is refetched. A token
    /// naming an unknown key id triggers a refetch regardless, so this only
    /// bounds how long a *withdrawn* key stays usable.
    pub jwks_refresh_secs: u64,

    /// Clock-skew tolerance in seconds for `exp`, `nbf` and `iat`.
    pub leeway_secs: u64,

    /// Subjects this gateway accepts, and what each may do. A token that
    /// verifies for a subject listed nowhere here is authenticated and
    /// unauthorized.
    #[serde(default)]
    pub principals: Vec<OidcPrincipalConfig>,
}

impl Default for OidcConfig {
    fn default() -> Self {
        Self {
            issuer: String::new(),
            audiences: Vec::new(),
            jwks_uri: String::new(),
            jwks_refresh_secs: 3600,
            leeway_secs: 60,
            principals: Vec::new(),
        }
    }
}

/// One accepted OIDC subject and the authority it holds here.
///
/// The same shape as [`ApiKeyConfig`] minus the secret: authority is Boswell's
/// to grant either way, and the identity provider only says who is asking.
#[derive(Debug, Clone, Deserialize)]
pub struct OidcPrincipalConfig {
    /// The `sub` claim this entry matches. Provider-assigned and stable;
    /// never an email address, which can be reassigned.
    pub subject: String,

    /// Namespace this subject is scoped to. Empty or `"*"` means unrestricted.
    #[serde(default)]
    pub namespace: String,

    /// Granted scopes: any of `read`, `write`, `delete`.
    #[serde(default)]
    pub scopes: Vec<String>,
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

# OIDC (ADR-022). Optional: leave the whole section out to accept API keys only.
#
# Boswell ships no identity provider. Point this at one you already run (Pocket
# ID is a reasonable local choice). A caller obtains a token from that provider
# — the device-code grant needs no browser on the agent's side — and presents it
# as Authorization: Bearer <token>. The gateway verifies it against the
# provider's published keys, which it caches, so no request costs a round trip.
#
# Verification establishes who is asking. What they may do comes from the
# principals below, exactly as it comes from api_keys above: a token that
# verifies for a subject listed nowhere here is refused with 403.
#
# [oidc]
# issuer = "https://id.example.com"
# audiences = ["boswell"]           # empty disables the audience check
# jwks_uri = ""                     # empty = discover it from the issuer
# jwks_refresh_secs = 3600
# leeway_secs = 60                  # clock-skew tolerance
#
# subject: the provider's stable `sub` claim, not an email address.
# [[oidc.principals]]
# subject = "01234567-89ab-cdef-0123-456789abcdef"
# namespace = "team"
# scopes = ["read", "write"]
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
