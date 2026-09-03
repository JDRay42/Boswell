//! Bearer API-key authentication, scope checks, and namespace isolation.
//!
//! A client presents `Authorization: Bearer <key>`. The gateway hashes the key
//! (SHA-256) and looks it up among the configured keys, yielding an
//! [`AuthContext`] with the key's namespace and scopes. The middleware also
//! enforces a per-key rate limit. `/v1/health` is not behind this layer.

use std::collections::HashSet;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;
use sha2::{Digest, Sha256};

use crate::error::ApiError;
use crate::state::AppState;

/// A capability granted to an API key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scope {
    /// Read claims (query, get, search, recall, relationships).
    Read,
    /// Create claims (assert, batch, extract, hook ingest).
    Write,
    /// Delete claims (forget).
    Delete,
}

impl Scope {
    /// The wire/config string for this scope.
    pub fn as_str(&self) -> &'static str {
        match self {
            Scope::Read => "read",
            Scope::Write => "write",
            Scope::Delete => "delete",
        }
    }

    /// Parse a scope string; unknown values yield `None`.
    pub fn parse(s: &str) -> Option<Scope> {
        match s.trim().to_ascii_lowercase().as_str() {
            "read" => Some(Scope::Read),
            "write" => Some(Scope::Write),
            "delete" => Some(Scope::Delete),
            _ => None,
        }
    }
}

/// Per-request authorization context, injected into request extensions by
/// [`auth_middleware`] and read by handlers.
#[derive(Debug, Clone)]
pub struct AuthContext {
    /// Stable id of the authenticated key (for audit logs).
    pub key_id: String,
    /// Namespace the key is scoped to (empty or `"*"` = unrestricted).
    pub namespace: String,
    /// Scopes granted to the key.
    pub scopes: HashSet<Scope>,
}

impl AuthContext {
    /// Require a scope, returning 403 if the key lacks it.
    pub fn require(&self, scope: Scope) -> Result<(), ApiError> {
        if self.scopes.contains(&scope) {
            Ok(())
        } else {
            Err(ApiError::forbidden(format!(
                "missing required scope: {}",
                scope.as_str()
            )))
        }
    }

    /// Whether this key may act on `namespace`.
    pub fn allows_namespace(&self, namespace: &str) -> bool {
        namespace_allows(&self.namespace, namespace)
    }

    /// Enforce that a write target `namespace` is within the key's scope.
    pub fn require_namespace(&self, namespace: &str) -> Result<(), ApiError> {
        if self.allows_namespace(namespace) {
            Ok(())
        } else {
            Err(ApiError::forbidden(format!(
                "namespace '{}' is outside this key's scope '{}'",
                namespace, self.namespace
            )))
        }
    }

    /// Resolve the effective namespace filter for a read.
    ///
    /// - If the client requested a namespace, it must be within the key's scope.
    /// - If not, a restricted key falls back to its own namespace (so results
    ///   never leak across namespaces); an unrestricted key gets `None`.
    pub fn read_namespace(&self, requested: Option<String>) -> Result<Option<String>, ApiError> {
        match requested {
            Some(ns) => {
                self.require_namespace(&ns)?;
                Ok(Some(ns))
            }
            None => {
                if self.namespace.is_empty() || self.namespace == "*" {
                    Ok(None)
                } else {
                    Ok(Some(self.namespace.clone()))
                }
            }
        }
    }
}

/// Whether `key_ns` (the key's namespace) permits acting on `target`.
///
/// An empty or `"*"` key namespace is unrestricted. Otherwise the target must
/// equal the key namespace or be a child of it (`"<key_ns>:..."`).
pub fn namespace_allows(key_ns: &str, target: &str) -> bool {
    if key_ns.is_empty() || key_ns == "*" {
        return true;
    }
    target == key_ns || target.starts_with(&format!("{}:", key_ns))
}

/// Lowercase hex SHA-256 of `input`.
pub fn hash_key(input: &str) -> String {
    let digest = Sha256::digest(input.as_bytes());
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{:02x}", byte));
    }
    out
}

/// Auth + rate-limit middleware for the protected `/v1/*` routes.
pub async fn auth_middleware(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let token = bearer_token(&req).ok_or_else(|| {
        ApiError::unauthorized("Missing or malformed Authorization: Bearer <key> header")
    })?;

    let hash = hash_key(&token);
    let ctx = state
        .lookup_key(&hash)
        .ok_or_else(|| ApiError::unauthorized("Invalid API key"))?;

    // Per-key rate limit.
    if !state.check_rate_limit(&ctx.key_id) {
        return Err(ApiError::rate_limited());
    }

    req.extensions_mut().insert(ctx);
    Ok(next.run(req).await)
}

/// Extract the raw bearer token from the `Authorization` header.
fn bearer_token(req: &Request) -> Option<String> {
    let header = req.headers().get(axum::http::header::AUTHORIZATION)?;
    let value = header.to_str().ok()?;
    let token = value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))?;
    let token = token.trim();
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_namespace_allows() {
        assert!(namespace_allows("", "anything"));
        assert!(namespace_allows("*", "anything"));
        assert!(namespace_allows("team", "team"));
        assert!(namespace_allows("team", "team:sub"));
        assert!(!namespace_allows("team", "teamwork")); // not a child
        assert!(!namespace_allows("team", "other"));
    }

    #[test]
    fn test_hash_key_is_sha256_hex() {
        // Known SHA-256 of the empty string.
        assert_eq!(
            hash_key(""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn test_scope_parse_roundtrip() {
        for s in ["read", "write", "delete"] {
            assert_eq!(Scope::parse(s).unwrap().as_str(), s);
        }
        assert!(Scope::parse("admin").is_none());
    }

    #[test]
    fn test_read_namespace_restricted_key_defaults_to_own() {
        let ctx = AuthContext {
            key_id: "k".into(),
            namespace: "team".into(),
            scopes: HashSet::new(),
        };
        assert_eq!(ctx.read_namespace(None).unwrap(), Some("team".to_string()));
        assert_eq!(
            ctx.read_namespace(Some("team:x".to_string())).unwrap(),
            Some("team:x".to_string())
        );
        assert!(ctx.read_namespace(Some("other".to_string())).is_err());
    }
}
