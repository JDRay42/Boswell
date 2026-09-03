//! Shared application state: the internal SDK client, the API-key registry, and
//! a simple per-key token-bucket rate limiter.

use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Instant;

use boswell_sdk::BoswellClient;
use tokio::sync::Mutex as TokioMutex;

use crate::auth::{AuthContext, Scope};
use crate::config::GatewayConfig;

/// Cloneable handle to gateway state (cheap `Arc` clone).
#[derive(Clone)]
pub struct AppState {
    inner: Arc<Inner>,
}

struct Inner {
    /// One shared client to the Router/instance. `tokio::Mutex` because SDK
    /// methods take `&mut self` and await across the call.
    client: TokioMutex<BoswellClient>,
    /// key_hash → authorization context template.
    keys: HashMap<String, AuthContext>,
    /// key_id → token bucket.
    buckets: StdMutex<HashMap<String, Bucket>>,
    /// Requests per minute per key; 0 disables limiting.
    rate_limit_per_minute: u32,
}

struct Bucket {
    tokens: f64,
    last_refill: Instant,
}

impl AppState {
    /// Build state from config, constructing (but not yet connecting) the client.
    pub fn from_config(config: &GatewayConfig) -> Self {
        let mut keys = HashMap::new();
        for key in &config.api_keys {
            let mut scopes = std::collections::HashSet::new();
            for raw in &key.scopes {
                match Scope::parse(raw) {
                    Some(s) => {
                        scopes.insert(s);
                    }
                    None => tracing::warn!(
                        "api key '{}' declares unknown scope '{}' (ignored)",
                        key.id,
                        raw
                    ),
                }
            }
            let ctx = AuthContext {
                key_id: key.id.clone(),
                namespace: key.namespace.clone(),
                scopes,
            };
            keys.insert(key.key_hash.trim().to_ascii_lowercase(), ctx);
        }

        let client = BoswellClient::new(&config.router_endpoint);

        Self {
            inner: Arc::new(Inner {
                client: TokioMutex::new(client),
                keys,
                buckets: StdMutex::new(HashMap::new()),
                rate_limit_per_minute: config.rate_limit_per_minute,
            }),
        }
    }

    /// The shared SDK client.
    pub fn client(&self) -> &TokioMutex<BoswellClient> {
        &self.inner.client
    }

    /// Look up an [`AuthContext`] by the SHA-256 hash of the presented key.
    pub fn lookup_key(&self, key_hash: &str) -> Option<AuthContext> {
        self.inner.keys.get(&key_hash.to_ascii_lowercase()).cloned()
    }

    /// Consume one token from `key_id`'s bucket. Returns `true` if allowed.
    pub fn check_rate_limit(&self, key_id: &str) -> bool {
        let capacity = self.inner.rate_limit_per_minute;
        if capacity == 0 {
            return true; // limiting disabled
        }
        let capacity = capacity as f64;
        let refill_per_sec = capacity / 60.0;

        let mut buckets = self.inner.buckets.lock().unwrap();
        let now = Instant::now();
        let bucket = buckets.entry(key_id.to_string()).or_insert(Bucket {
            tokens: capacity,
            last_refill: now,
        });

        let elapsed = now.duration_since(bucket.last_refill).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * refill_per_sec).min(capacity);
        bucket.last_refill = now;

        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ApiKeyConfig;

    fn config_with_key(rate: u32) -> GatewayConfig {
        GatewayConfig {
            rate_limit_per_minute: rate,
            api_keys: vec![ApiKeyConfig {
                id: "k1".into(),
                key_hash: "ABC123".into(), // stored uppercase; lookup lowercases
                namespace: "team".into(),
                scopes: vec!["read".into(), "write".into()],
            }],
            ..GatewayConfig::default()
        }
    }

    #[test]
    fn test_lookup_key_case_insensitive() {
        let state = AppState::from_config(&config_with_key(0));
        let ctx = state.lookup_key("abc123").expect("key should be found");
        assert_eq!(ctx.key_id, "k1");
        assert!(ctx.scopes.contains(&Scope::Read));
        assert!(ctx.scopes.contains(&Scope::Write));
        assert!(state.lookup_key("nope").is_none());
    }

    #[test]
    fn test_rate_limit_disabled_when_zero() {
        let state = AppState::from_config(&config_with_key(0));
        for _ in 0..1000 {
            assert!(state.check_rate_limit("k1"));
        }
    }

    #[test]
    fn test_rate_limit_exhausts_then_blocks() {
        let state = AppState::from_config(&config_with_key(3));
        assert!(state.check_rate_limit("k1"));
        assert!(state.check_rate_limit("k1"));
        assert!(state.check_rate_limit("k1"));
        // Bucket now empty; the next immediate request is blocked.
        assert!(!state.check_rate_limit("k1"));
    }
}
