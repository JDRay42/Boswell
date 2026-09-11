//! Gateway configuration, loaded from a TOML file (see `config/gateway.toml`).
//!
//! API keys are stored as **SHA-256 hashes**, never in plaintext. A client
//! presents the raw key as `Authorization: Bearer <key>`; the gateway hashes it
//! and matches against `key_hash`.

use boswell_domain::{validate_principal, PrincipalShapeError};
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

    /// A configured identity is not shaped like an authenticated principal.
    ///
    /// Every principal this gateway ever names comes from one of two places in
    /// this file, so this is the last point at which a bad one can be reported
    /// rather than silently narrowed. See [`boswell_domain::validate_principal`].
    #[error("{field} = {value:?} is not an authenticated principal: {source}")]
    Identity {
        /// Where in the file the offending value sits, e.g. `api_keys[0].id`.
        field: String,
        /// The value as configured.
        value: String,
        /// What is wrong with its shape.
        source: PrincipalShapeError,
    },
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

    /// Attenuable tokens. Absent means the gateway neither mints nor accepts
    /// them, and `POST /v1/tokens` is 404.
    #[serde(default)]
    pub tokens: Option<TokenConfig>,
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
            tokens: None,
        }
    }
}

/// Attenuable-token settings (ADR-022).
///
/// The root key is the whole grant: anything holding it can mint a token for
/// any principal. It belongs in a file the gateway alone can read, alongside
/// the API-key hashes it already lives beside.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct TokenConfig {
    /// Hex-encoded Ed25519 private key, 32 bytes. Generate one with
    /// `boswell-gateway keygen`; there is deliberately no default, because a
    /// default root key is no root key.
    pub root_private_key: String,

    /// Lifetime of a minted root token when the caller names none.
    pub default_ttl_secs: u64,

    /// Ceiling on a requested lifetime. Verification is offline, so expiry and
    /// the revocation list are the only two things that end a token; the
    /// shorter this is, the shorter the list has to stay.
    ///
    /// Defaults to 7 days. That is the window a leaked token stays usable
    /// without an operator doing anything, and the span a revocation entry has
    /// to be kept before it becomes dead weight.
    pub max_ttl_secs: u64,

    /// Path to the revocation list — one hex revocation identifier per line,
    /// `#` comments allowed. Empty means no list, and so nothing revoked.
    ///
    /// A file rather than config or a store table: it changes at incident
    /// speed, and the gateway re-reads it without a restart and without a
    /// network call. See [`crate::revocation`].
    pub revocation_list_path: String,

    /// How long a loaded revocation list is used before the file is re-`stat`ed.
    /// This is the delay between appending a line and the gateway honoring it.
    pub revocation_refresh_secs: u64,
}

impl Default for TokenConfig {
    fn default() -> Self {
        Self {
            root_private_key: String::new(),
            default_ttl_secs: 24 * 60 * 60,
            max_ttl_secs: 7 * 24 * 60 * 60,
            revocation_list_path: String::new(),
            revocation_refresh_secs: 15,
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
        config.validate_identities()?;
        Ok(config)
    }

    /// Check that every identity this file names is shaped like an
    /// authenticated principal (design §8.3).
    ///
    /// `AuthContext::key_id` becomes a receipt's `issued_to`, which becomes a
    /// stamp's `author`, which is what corroboration counts independence over —
    /// after `authenticated_principal` has stripped anything past the first
    /// `/`. That narrowing is total and silent by design, so an identity
    /// carrying a subagent path is not rejected anywhere downstream; it is just
    /// counted as something shorter than what was written down. Here is the
    /// only place left where saying so is still useful.
    ///
    /// The two sources are exhaustive: an API key's `id` is used verbatim, and
    /// an OIDC subject becomes `oidc:<sub>`. A minted token's principal is
    /// copied from the `AuthContext` that minted it, so it inherits whichever
    /// of the two it came from and adds no third shape.
    pub fn validate_identities(&self) -> Result<(), ConfigError> {
        for (i, key) in self.api_keys.iter().enumerate() {
            validate_principal(&key.id).map_err(|source| ConfigError::Identity {
                field: format!("api_keys[{}].id", i),
                value: key.id.clone(),
                source,
            })?;
        }
        for (i, principal) in self
            .oidc
            .iter()
            .flat_map(|o| o.principals.iter())
            .enumerate()
        {
            validate_principal(&principal.subject).map_err(|source| ConfigError::Identity {
                field: format!("oidc.principals[{}].subject", i),
                value: principal.subject.clone(),
                source,
            })?;
        }
        Ok(())
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

# Attenuable tokens (ADR-022). Optional: leave the section out and the gateway
# neither mints nor accepts them.
#
# An authenticated caller POSTs to /v1/tokens and gets back a root token holding
# exactly the authority it already had. The agent keeps that token and, when it
# spawns a subagent, narrows a copy locally — fewer scopes, a deeper namespace,
# a shorter life — with no call to the gateway and no call to the identity
# provider. Narrowing is enforced by the signature: a holder can only remove.
#
# root_private_key is the whole grant. Anything holding it can mint a token for
# any principal, so it belongs in a file only the gateway can read. Generate:
#   boswell-gateway keygen
#
# Verification is offline, so a token's own expiry and the revocation list are
# the only two things that end it early. max_ttl_secs caps a requested lifetime
# and so bounds how long the list has to remember anything; keep it as short as
# the deployment tolerates. The shipped default is 7 days.
#
# revocation_list_path names a file of revocation identifiers, one lowercase hex
# id per line, # comments allowed. Appending a line ends that token, and every
# token attenuated from it, within revocation_refresh_secs. /v1/tokens returns
# the id of each token it mints; `boswell-gateway revocation-ids <token>` prints
# the ids of any token you hold; `boswell-gateway revoke <list> <token>` appends
# one. Name the file when you enable this section: with no file, expiry is the
# only brake, and the file need not exist yet.
#
# [tokens]
# root_private_key = "0000000000000000000000000000000000000000000000000000000000000000"
# default_ttl_secs = 86400          # 1 day
# max_ttl_secs = 604800             # 7 days
# revocation_list_path = "config/revoked.txt"
# revocation_refresh_secs = 15
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
        // The config an operator is handed must itself pass the check the
        // loader applies to theirs.
        c.validate_identities().unwrap();
    }

    /// Uncomment the starter's `[tokens]` example and it must parse. Nothing
    /// else checks the commented half of the file, so a typo there ships.
    fn starter_tokens_example() -> TokenConfig {
        let src = GatewayConfig::starter_toml();
        let start = src
            .find("# [tokens]")
            .expect("the starter must carry a [tokens] example");
        let block: String = src[start..]
            .lines()
            .take_while(|l| l.starts_with('#'))
            .map(|l| format!("{}\n", l.trim_start_matches('#').trim_start()))
            .collect();
        let c: GatewayConfig =
            toml::from_str(&block).expect("the [tokens] example must be valid TOML");
        c.tokens.expect("the example must populate [tokens]")
    }

    /// The point of the item that produced this test: an operator who enables
    /// tokens by uncommenting the example gets the revocation list with them.
    /// A `[tokens]` section without a path is the posture revocation was for.
    #[test]
    fn the_starter_s_tokens_example_names_a_revocation_list() {
        let t = starter_tokens_example();
        assert!(
            !t.revocation_list_path.is_empty(),
            "the [tokens] example must name a revocation_list_path"
        );
        assert!(t.revocation_refresh_secs > 0);
    }

    /// The ceiling bounds how long the revocation list has to remember a token,
    /// so it is a deliberate number rather than a value to drift upward. Moving
    /// it is fine; moving it silently is not.
    #[test]
    fn the_default_token_ceiling_is_seven_days() {
        assert_eq!(TokenConfig::default().max_ttl_secs, 7 * 24 * 60 * 60);
        // The example an operator copies must not undo the shipped default.
        assert_eq!(
            starter_tokens_example().max_ttl_secs,
            TokenConfig::default().max_ttl_secs
        );
    }

    /// The default a caller gets when it names no lifetime stays under the
    /// ceiling; a default above the cap would be silently clamped on every mint.
    #[test]
    fn the_default_lifetime_fits_under_the_ceiling() {
        let t = TokenConfig::default();
        assert!(t.default_ttl_secs <= t.max_ttl_secs);
        assert_eq!(t.default_ttl_secs, 24 * 60 * 60);
    }

    /// Writing the config out and loading it back is the path that matters:
    /// `validate_identities` is only useful if `from_file` actually runs it.
    fn load(toml: &str) -> Result<GatewayConfig, ConfigError> {
        let dir = std::env::temp_dir().join(format!(
            "boswell-cfg-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("gateway.toml");
        std::fs::write(&path, toml).unwrap();
        let result = GatewayConfig::from_file(&path);
        std::fs::remove_dir_all(&dir).ok();
        result
    }

    #[test]
    fn an_api_key_id_carrying_a_subagent_path_is_refused_at_load() {
        let err = load(
            r#"
            [[api_keys]]
            id = "agent:orch-7/sub:explore-3"
            key_hash = "abc"
        "#,
        )
        .unwrap_err();
        assert!(
            matches!(
                &err,
                ConfigError::Identity { field, source, .. }
                    if field == "api_keys[0].id"
                        && *source == PrincipalShapeError::SubagentPath
            ),
            "unexpected error: {err}"
        );
        // The offending value is named, so the operator does not have to guess
        // which of several entries it was.
        assert!(err.to_string().contains("agent:orch-7/sub:explore-3"));
    }

    #[test]
    fn an_empty_api_key_id_is_refused_at_load() {
        let err = load(
            r#"
            [[api_keys]]
            id = ""
            key_hash = "abc"
        "#,
        )
        .unwrap_err();
        assert!(
            matches!(&err, ConfigError::Identity { source, .. }
                if *source == PrincipalShapeError::Empty),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn an_oidc_subject_carrying_a_slash_is_refused_at_load() {
        let err = load(
            r#"
            [oidc]
            issuer = "https://idp.example"

            [[oidc.principals]]
            subject = "alice"

            [[oidc.principals]]
            subject = "tenant-a/alice"
        "#,
        )
        .unwrap_err();
        assert!(
            matches!(
                &err,
                ConfigError::Identity { field, source, .. }
                    if field == "oidc.principals[1].subject"
                        && *source == PrincipalShapeError::SubagentPath
            ),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn a_config_naming_only_bare_principals_loads() {
        let c = load(
            r#"
            [[api_keys]]
            id = "example-agent"
            key_hash = "abc"

            [oidc]
            issuer = "https://idp.example"

            [[oidc.principals]]
            subject = "01234567-89ab-cdef-0123-456789abcdef"
        "#,
        )
        .unwrap();
        assert_eq!(c.api_keys.len(), 1);
        assert_eq!(c.oidc.unwrap().principals.len(), 1);
    }

    #[test]
    fn a_config_with_no_identities_at_all_loads() {
        load("bind_port = 9099").unwrap();
    }
}
