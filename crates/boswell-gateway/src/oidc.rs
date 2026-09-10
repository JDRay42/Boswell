//! OIDC bearer-token verification against locally cached JWKS ([ADR-022]).
//!
//! The implementer authenticates once to an external identity provider — the
//! device-code grant is the intended flow, and it happens entirely between that
//! client and the provider. The gateway never sees a password, never redirects,
//! and never calls the provider on the request path. It verifies the resulting
//! JWT with a public key it already holds.
//!
//! That last property is the point. [`JwksCache`] fetches the provider's key set
//! once, keeps it for [`OidcConfig::jwks_refresh_secs`], and refetches only when
//! the cache is stale or a token arrives signed by a key id it has never seen. A
//! provider that is down does not take the gateway's authentication with it.
//!
//! Verification establishes *who*; it does not grant anything. Authority comes
//! from a `[[oidc.principals]]` entry matching the token's `sub`, exactly as an
//! `[[api_keys]]` entry supplies it for a key. A token that verifies against a
//! subject with no entry is authenticated and unauthorized — 403, not 401.
//!
//! [ADR-022]: https://github.com/JDRay42/Boswell/blob/main/docs/ADRs/022-delegated-credentials.md

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use tokio::sync::RwLock;

use crate::auth::{AuthContext, Scope};
use crate::config::OidcConfig;
use crate::error::ApiError;

/// Shortest interval between two JWKS fetches.
///
/// Without this, a stream of tokens carrying unknown key ids would be a
/// request amplifier pointed at the identity provider.
const MIN_REFRESH_INTERVAL: Duration = Duration::from_secs(60);

/// How long a JWKS fetch may take before it is abandoned.
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// Why a presented token was not accepted.
#[derive(Debug, thiserror::Error)]
pub enum OidcError {
    /// The token is not a well-formed JWT, or its header is unusable.
    #[error("malformed token: {0}")]
    Malformed(String),

    /// The token's `kid` is absent, or names a key the provider does not publish.
    #[error("unknown signing key: {0}")]
    UnknownKey(String),

    /// Signature, issuer, audience or expiry check failed.
    #[error("token rejected: {0}")]
    Rejected(String),

    /// The token verified, but its subject has no configured authority.
    #[error("subject '{0}' has no configured authority at this gateway")]
    UnknownPrincipal(String),

    /// The key set could not be fetched.
    #[error("could not reach the identity provider: {0}")]
    Provider(String),
}

impl From<OidcError> for ApiError {
    fn from(err: OidcError) -> Self {
        match err {
            // Authenticated but unauthorized is a 403: re-presenting the same
            // token will not help, and saying so is not a disclosure — the
            // holder already proved the subject is theirs.
            OidcError::UnknownPrincipal(_) => ApiError::forbidden(err.to_string()),
            // Everything else is deliberately vague to the client and detailed
            // in the log: a caller learning *why* its forgery failed is a
            // caller being told how to forge better.
            other => {
                tracing::debug!("oidc: {}", other);
                ApiError::unauthorized("Invalid bearer token")
            }
        }
    }
}

/// A single JSON Web Key, as published at the provider's `jwks_uri`.
#[derive(Debug, Clone, Deserialize)]
struct Jwk {
    kty: String,
    #[serde(default)]
    kid: Option<String>,
    #[serde(rename = "use", default)]
    key_use: Option<String>,
    #[serde(default)]
    alg: Option<String>,
    // RSA
    #[serde(default)]
    n: Option<String>,
    #[serde(default)]
    e: Option<String>,
    // EC
    #[serde(default)]
    crv: Option<String>,
    #[serde(default)]
    x: Option<String>,
    #[serde(default)]
    y: Option<String>,
}

/// A provider's published key set.
#[derive(Debug, Clone, Deserialize)]
struct JwkSet {
    #[serde(default)]
    keys: Vec<Jwk>,
}

/// The subset of an OIDC discovery document the gateway reads.
#[derive(Debug, Deserialize)]
struct Discovery {
    jwks_uri: String,
}

/// A verification key with the algorithms it is allowed to have signed with.
#[derive(Clone)]
struct VerifyingKey {
    key: DecodingKey,
    algorithms: Vec<Algorithm>,
}

/// Turn one published JWK into a verification key, or `None` if the gateway
/// will not verify with it.
///
/// Two kinds are skipped on purpose. A key marked `use: "enc"` is for
/// encryption, not signatures. A symmetric key (`kty: "oct"`) is never
/// legitimately published in a public key set, and accepting one would mean
/// anything that can read the JWKS can also mint tokens.
fn verifying_key(jwk: &Jwk) -> Option<(String, VerifyingKey)> {
    if jwk.key_use.as_deref() == Some("enc") {
        return None;
    }
    let kid = jwk.kid.clone()?;

    let (key, default_algs) = match jwk.kty.as_str() {
        "RSA" => {
            let key =
                DecodingKey::from_rsa_components(jwk.n.as_deref()?, jwk.e.as_deref()?).ok()?;
            (
                key,
                vec![
                    Algorithm::RS256,
                    Algorithm::RS384,
                    Algorithm::RS512,
                    Algorithm::PS256,
                    Algorithm::PS384,
                    Algorithm::PS512,
                ],
            )
        }
        "EC" => {
            let key = DecodingKey::from_ec_components(jwk.x.as_deref()?, jwk.y.as_deref()?).ok()?;
            let algs = match jwk.crv.as_deref()? {
                "P-256" => vec![Algorithm::ES256],
                "P-384" => vec![Algorithm::ES384],
                _ => return None,
            };
            (key, algs)
        }
        other => {
            tracing::debug!("oidc: ignoring JWKS entry of type '{}'", other);
            return None;
        }
    };

    // A key that names its own algorithm is held to it, which is what closes
    // the algorithm-substitution gap for providers that publish `alg`.
    let algorithms = match jwk.alg.as_deref().and_then(parse_algorithm) {
        Some(alg) if default_algs.contains(&alg) => vec![alg],
        Some(_) => return None, // declared an algorithm its key type cannot do
        None => default_algs,
    };

    Some((kid, VerifyingKey { key, algorithms }))
}

/// Parse a JWS `alg` name. Deliberately has no `none` arm.
fn parse_algorithm(name: &str) -> Option<Algorithm> {
    match name {
        "RS256" => Some(Algorithm::RS256),
        "RS384" => Some(Algorithm::RS384),
        "RS512" => Some(Algorithm::RS512),
        "PS256" => Some(Algorithm::PS256),
        "PS384" => Some(Algorithm::PS384),
        "PS512" => Some(Algorithm::PS512),
        "ES256" => Some(Algorithm::ES256),
        "ES384" => Some(Algorithm::ES384),
        _ => None,
    }
}

/// Parse a JWKS document into verification keys, dropping entries the gateway
/// will not verify with.
fn parse_jwks(body: &str) -> Result<HashMap<String, VerifyingKey>, OidcError> {
    let set: JwkSet =
        serde_json::from_str(body).map_err(|e| OidcError::Provider(format!("bad JWKS: {}", e)))?;
    Ok(set.keys.iter().filter_map(verifying_key).collect())
}

/// The cached copy of an identity provider's public key set.
///
/// Held behind an `RwLock`: the steady-state path takes a read lock and makes
/// no network call at all.
struct JwksCache {
    http: reqwest::Client,
    issuer: String,
    /// Configured `jwks_uri`, or the one learned from discovery.
    jwks_uri: RwLock<Option<String>>,
    keys: RwLock<HashMap<String, VerifyingKey>>,
    /// When the key set was last successfully fetched.
    fetched_at: RwLock<Option<Instant>>,
    /// Age at which the cached set is refetched before use.
    max_age: Duration,
}

impl JwksCache {
    fn new(config: &OidcConfig) -> Self {
        let jwks_uri = {
            let configured = config.jwks_uri.trim();
            if configured.is_empty() {
                None
            } else {
                Some(configured.to_string())
            }
        };
        Self {
            http: reqwest::Client::builder()
                .timeout(FETCH_TIMEOUT)
                .build()
                .unwrap_or_default(),
            issuer: config.issuer.trim_end_matches('/').to_string(),
            jwks_uri: RwLock::new(jwks_uri),
            keys: RwLock::new(HashMap::new()),
            fetched_at: RwLock::new(None),
            max_age: Duration::from_secs(config.jwks_refresh_secs),
        }
    }

    /// A cache holding exactly these keys, so a test can verify a token without
    /// a provider to fetch from.
    #[cfg(test)]
    fn seeded(config: &OidcConfig, body: &str) -> Result<Self, OidcError> {
        let keys = parse_jwks(body)?;
        Ok(Self {
            keys: RwLock::new(keys),
            fetched_at: RwLock::new(Some(Instant::now())),
            ..Self::new(config)
        })
    }

    /// Whether the cached set is old enough to refetch before it is used.
    async fn is_stale(&self) -> bool {
        match *self.fetched_at.read().await {
            None => true,
            Some(at) => at.elapsed() >= self.max_age,
        }
    }

    /// Whether a refresh is allowed right now, given [`MIN_REFRESH_INTERVAL`].
    async fn may_refresh(&self) -> bool {
        match *self.fetched_at.read().await {
            None => true,
            Some(at) => at.elapsed() >= MIN_REFRESH_INTERVAL,
        }
    }

    /// The URI to fetch, resolving it through OIDC discovery on first use.
    async fn resolve_uri(&self) -> Result<String, OidcError> {
        if let Some(uri) = self.jwks_uri.read().await.clone() {
            return Ok(uri);
        }
        let discovery_url = format!("{}/.well-known/openid-configuration", self.issuer);
        let discovery: Discovery = self
            .http
            .get(&discovery_url)
            .send()
            .await
            .map_err(|e| OidcError::Provider(format!("{}: {}", discovery_url, e)))?
            .error_for_status()
            .map_err(|e| OidcError::Provider(format!("{}: {}", discovery_url, e)))?
            .json()
            .await
            .map_err(|e| OidcError::Provider(format!("{}: {}", discovery_url, e)))?;

        *self.jwks_uri.write().await = Some(discovery.jwks_uri.clone());
        Ok(discovery.jwks_uri)
    }

    /// Fetch and replace the cached key set.
    async fn refresh(&self) -> Result<(), OidcError> {
        let uri = self.resolve_uri().await?;
        let body = self
            .http
            .get(&uri)
            .send()
            .await
            .map_err(|e| OidcError::Provider(format!("{}: {}", uri, e)))?
            .error_for_status()
            .map_err(|e| OidcError::Provider(format!("{}: {}", uri, e)))?
            .text()
            .await
            .map_err(|e| OidcError::Provider(format!("{}: {}", uri, e)))?;

        let keys = parse_jwks(&body)?;
        tracing::info!("oidc: cached {} signing key(s) from {}", keys.len(), uri);
        *self.keys.write().await = keys;
        *self.fetched_at.write().await = Some(Instant::now());
        Ok(())
    }

    /// The verification key for `kid`.
    ///
    /// Refetches when the cache is stale, and once more when `kid` is absent —
    /// a provider that has rotated its keys is the ordinary reason for a miss,
    /// and waiting out `max_age` would reject every token until then.
    async fn key_for(&self, kid: &str) -> Result<VerifyingKey, OidcError> {
        if self.is_stale().await {
            if let Err(e) = self.refresh().await {
                // A stale set still verifies tokens signed before the rotation,
                // so serving from it beats failing closed on a provider blip.
                tracing::warn!("oidc: JWKS refresh failed, using cached keys ({})", e);
            }
        }
        if let Some(key) = self.keys.read().await.get(kid).cloned() {
            return Ok(key);
        }
        if self.may_refresh().await {
            self.refresh().await?;
            if let Some(key) = self.keys.read().await.get(kid).cloned() {
                return Ok(key);
            }
        }
        Err(OidcError::UnknownKey(kid.to_string()))
    }
}

/// The registered claims the gateway reads, plus the two it logs.
#[derive(Debug, Deserialize)]
struct Claims {
    sub: String,
    #[serde(default)]
    preferred_username: Option<String>,
}

/// The authority a configured principal is granted once its token verifies.
#[derive(Debug, Clone)]
struct Grant {
    namespace: String,
    scopes: HashSet<Scope>,
}

/// Verifies OIDC bearer tokens and maps verified subjects to gateway authority.
pub struct OidcVerifier {
    jwks: JwksCache,
    issuer: String,
    audiences: Vec<String>,
    leeway_secs: u64,
    /// `sub` → what that subject may do here.
    principals: HashMap<String, Grant>,
}

impl OidcVerifier {
    /// Build a verifier from the `[oidc]` config section.
    ///
    /// Nothing is fetched here; the first request that needs a key populates
    /// the cache, so a provider that is down at startup does not stop the
    /// gateway from serving its API keys.
    pub fn new(config: &OidcConfig) -> Self {
        Self {
            jwks: JwksCache::new(config),
            issuer: config.issuer.trim_end_matches('/').to_string(),
            audiences: config.audiences.clone(),
            leeway_secs: config.leeway_secs,
            principals: principal_map(config),
        }
    }

    /// Verify `token` and resolve it to the authority its subject holds here.
    pub async fn authenticate(&self, token: &str) -> Result<AuthContext, OidcError> {
        let header = decode_header(token).map_err(|e| OidcError::Malformed(e.to_string()))?;
        let kid = header
            .kid
            .ok_or_else(|| OidcError::UnknownKey("token carries no kid".to_string()))?;

        let key = self.jwks.key_for(&kid).await?;

        // The allowed algorithms come from the key, not from the token's own
        // header, so a token cannot nominate the algorithm it is checked with.
        let mut validation = Validation::new(key.algorithms[0]);
        validation.algorithms = key.algorithms.clone();
        validation.leeway = self.leeway_secs;
        validation.set_issuer(&[&self.issuer]);
        if self.audiences.is_empty() {
            validation.validate_aud = false;
        } else {
            validation.set_audience(&self.audiences);
        }

        let data = decode::<Claims>(token, &key.key, &validation)
            .map_err(|e| OidcError::Rejected(e.to_string()))?;
        let claims = data.claims;

        let grant = self
            .principals
            .get(&claims.sub)
            .ok_or_else(|| OidcError::UnknownPrincipal(claims.sub.clone()))?;

        tracing::debug!(
            "oidc: authenticated subject '{}'{}",
            claims.sub,
            claims
                .preferred_username
                .as_deref()
                .map(|u| format!(" ({})", u))
                .unwrap_or_default()
        );

        Ok(AuthContext {
            // Prefixed so an audit log never confuses an identity-provider
            // subject with a locally configured API key id.
            key_id: format!("oidc:{}", claims.sub),
            namespace: grant.namespace.clone(),
            scopes: grant.scopes.clone(),
        })
    }
}

/// Build the subject → authority map, warning about unusable entries rather
/// than failing startup over one typo.
fn principal_map(config: &OidcConfig) -> HashMap<String, Grant> {
    let mut map = HashMap::new();
    for principal in &config.principals {
        let mut scopes = HashSet::new();
        for raw in &principal.scopes {
            match Scope::parse(raw) {
                Some(s) => {
                    scopes.insert(s);
                }
                None => tracing::warn!(
                    "oidc principal '{}' declares unknown scope '{}' (ignored)",
                    principal.subject,
                    raw
                ),
            }
        }
        map.insert(
            principal.subject.clone(),
            Grant {
                namespace: principal.namespace.clone(),
                scopes,
            },
        );
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::OidcPrincipalConfig;

    /// Test vectors, generated once with `openssl` against a throwaway RSA-2048
    /// keypair that was not kept. Only the public half is here, so the tokens
    /// below can be verified and no new one can be signed. `valid` and `forged`
    /// carry identical claims; `forged` is signed by a second key while naming
    /// the first key's `kid`, which is the substitution a signature check has
    /// to catch.
    const JWKS: &str = r#"{"keys": [{"kty": "RSA", "use": "sig", "alg": "RS256", "kid": "test-key-1", "n": "vp9o62wq-MA0dBJDJArFXrpYKnqL2L7BpTh2QtdaBsheAraTMDFMgF7FkPz1zFLAkcYXJa0P0xeu_isTcGRuS2EmJrcElNbmYcsSVFuWCZyrBTd9uV1aOtXH47CNQD9bUTNZeym0NQ0jRb3ahM0r-MbulegVJaBHvNI-C2CAdNqJeH6I9bZvJObUsMCeAqLDchK1YrOQDpCQoCN-VzdRPUcO4C7xiJ6IsxMScRb3cbvLgqdMsfShocF__W08jsnzQ1D8cZJUkeUppAt4Nl0HBTxuCPg8L8kgV4Pp-NK98JOLRBQq0ORZCOwrNiLS_jyZQObeB__ISl5ltdbl4XF-ow", "e": "AQAB"}]}"#;

    /// `sub: user-abc123`, `iss: https://id.example.test`, `aud: boswell`,
    /// expiring in 2100.
    const VALID: &str = "eyJhbGciOiJSUzI1NiIsImtpZCI6InRlc3Qta2V5LTEiLCJ0eXAiOiJKV1QifQ.eyJhdWQiOiJib3N3ZWxsIiwiZXhwIjo0MTAyNDQ0ODAwLCJncm91cHMiOlsiaW1wbGVtZW50ZXJzIl0sImlhdCI6MTcwMDAwMDAwMCwiaXNzIjoiaHR0cHM6Ly9pZC5leGFtcGxlLnRlc3QiLCJuYmYiOjE3MDAwMDAwMDAsInByZWZlcnJlZF91c2VybmFtZSI6ImFkYSIsInN1YiI6InVzZXItYWJjMTIzIn0.XkEu6QjIX58WGtdugE4TJC9oHrmACdwZsh8vjvuUuep_0VPCoEfvUxA-6t2LKXk5APPtm-nMYM4TwRnpjDL6bh3ksPHGt0nhBb276lSHuwDPVaPwlrhTGjz7t_9EzZCCTNNQSbzSExfQg8ZtsJ3c6t03PvLtoJC8gVf8d-pjT3G_ilCNJMMBFIaDbuExsG-3ASpykuo2Oo9qo6Nw1p0XJL8QtQSfRHAy0hd44SFhtaPi1kGiONc4BW40Dkv_M1IP8AgD92Vvqu_cm4YOSOg8Nl_bXSjbzOJrLh0TglUpB8W64rUCLaSEIKM_uIoQ7bSUva0AYUCoFhJI25oP2OrotA";

    /// The same subject, expired in 2023.
    const EXPIRED: &str = "eyJhbGciOiJSUzI1NiIsImtpZCI6InRlc3Qta2V5LTEiLCJ0eXAiOiJKV1QifQ.eyJhdWQiOiJib3N3ZWxsIiwiZXhwIjoxNzAwMDAzNjAwLCJncm91cHMiOlsiaW1wbGVtZW50ZXJzIl0sImlhdCI6MTcwMDAwMDAwMCwiaXNzIjoiaHR0cHM6Ly9pZC5leGFtcGxlLnRlc3QiLCJuYmYiOjE3MDAwMDAwMDAsInByZWZlcnJlZF91c2VybmFtZSI6ImFkYSIsInN1YiI6InVzZXItYWJjMTIzIn0.J0JtEe3aqdPrvUbIlzMFJMjnlRDwc7Sw6dwICov8RfJLtlhmh5MeTpsqMjCowAQEycAhRbA5TJhw1qcBbp1h602bWRxd8qfU15-0lxf8hBr0OVb8KWESujGqCx1fqNeAgMqObxS3psCoE5CXRqoexlMxudA5WcN8ZLbwtENKBsCzR_yB9bQvT4qod-O46k1uJuGWsRZ4jXk19kZeru4h5ITxhF7lfXWXPZXgg_QZ6qLged2seTxNd6J2VO9N1_j_uAClZMo0MO_u3FaQE4LZXKRTlwt7CieiuHoxe-V2FMj-Dom8V1i-CtQvbm18cGiETX4pBvRPPaphSG6xlZXSzA";

    /// Claims identical to `VALID`, signed by a key the JWKS does not publish.
    const FORGED: &str = "eyJhbGciOiJSUzI1NiIsImtpZCI6InRlc3Qta2V5LTEiLCJ0eXAiOiJKV1QifQ.eyJhdWQiOiJib3N3ZWxsIiwiZXhwIjo0MTAyNDQ0ODAwLCJncm91cHMiOlsiaW1wbGVtZW50ZXJzIl0sImlhdCI6MTcwMDAwMDAwMCwiaXNzIjoiaHR0cHM6Ly9pZC5leGFtcGxlLnRlc3QiLCJuYmYiOjE3MDAwMDAwMDAsInByZWZlcnJlZF91c2VybmFtZSI6ImFkYSIsInN1YiI6InVzZXItYWJjMTIzIn0.ngCEdAsWmK8_qNQEMbtzwV9-Dos6Ecwisn1i5pbWxvcP4GN7PkM2ulUMKNAct3CST4iJaA5r3d8InJtkMI3kQepCgS8C_0yBi9qZX1JVYDKrsNaqj6l8B6H3Bu_kTK3McFIxj_q0xt073QFbxDKg3Cw8eiEk3SDUDt3z1wVC6vpIqgvXbs6QLvmcQRXZE8OXUZPvJ7FqnbnXY_c1c-L-KxPFjOS5_0ITUE5_Q0LnQHKaGig9mbZv7Xg-9lthq4R57f_iXyyRHfVKVhay5wRn7fREJyCcf17MV9aR4zjt2rbOSRpySQE7lsGAbBo55zYhiwMXmj82L4kjuJl3cxkJjA";

    /// Valid claims signed by the published key, naming a `kid` that is not in
    /// the key set.
    const UNKNOWN_KID: &str = "eyJhbGciOiJSUzI1NiIsImtpZCI6InVua25vd24ta2V5IiwidHlwIjoiSldUIn0.eyJhdWQiOiJib3N3ZWxsIiwiZXhwIjo0MTAyNDQ0ODAwLCJncm91cHMiOlsiaW1wbGVtZW50ZXJzIl0sImlhdCI6MTcwMDAwMDAwMCwiaXNzIjoiaHR0cHM6Ly9pZC5leGFtcGxlLnRlc3QiLCJuYmYiOjE3MDAwMDAwMDAsInByZWZlcnJlZF91c2VybmFtZSI6ImFkYSIsInN1YiI6InVzZXItYWJjMTIzIn0.iAV8H30AkrphmTEXbUPZPqJtX1olpqw4roa72q-bWUtUSAEhnSF60W9AxK0Lo3BgNwrYtAFZ13yBdCjF34JMsZvrVUawMlX9MQNUFoTSyOLW_wolTpxH5rDm69KE7NmKrsYfZdqcWizF7Mv42vrRvrT6Cvrc7CFYs670V74K-Yz3YM84cBsCchemfgkot2p2iagmkvFj-vEu_yLBM4eoxYwm18kpQMtMKnFpRSIBfSuGt7JybY4zIjob1KcodU4mUYqPNK2ImV_aLtoZZbhWbrgJR8oqwcr-A1OGRPd_zzbCdj997tE1tdKrNoygSbSNrhw3d-CUf_RMW7Hm9JdbZA";

    /// `VALID`'s claims with the header rewritten to `alg: none` and the
    /// signature removed — the oldest JWT attack there is.
    const ALG_NONE: &str = "eyJhbGciOiJub25lIiwia2lkIjoidGVzdC1rZXktMSIsInR5cCI6IkpXVCJ9.eyJhdWQiOiJib3N3ZWxsIiwiZXhwIjo0MTAyNDQ0ODAwLCJncm91cHMiOlsiaW1wbGVtZW50ZXJzIl0sImlhdCI6MTcwMDAwMDAwMCwiaXNzIjoiaHR0cHM6Ly9pZC5leGFtcGxlLnRlc3QiLCJuYmYiOjE3MDAwMDAwMDAsInByZWZlcnJlZF91c2VybmFtZSI6ImFkYSIsInN1YiI6InVzZXItYWJjMTIzIn0.";

    fn test_config() -> OidcConfig {
        OidcConfig {
            issuer: "https://id.example.test".into(),
            audiences: vec!["boswell".into()],
            // Explicit so nothing in these tests can resolve a discovery
            // document; the cache is seeded and never refreshes.
            jwks_uri: "https://id.example.test/jwks".into(),
            principals: vec![OidcPrincipalConfig {
                subject: "user-abc123".into(),
                namespace: "team".into(),
                scopes: vec!["read".into(), "write".into()],
            }],
            ..OidcConfig::default()
        }
    }

    /// A verifier whose key cache is already populated, so no test touches the
    /// network. `jwks_refresh_secs` is far enough out that nothing goes stale.
    fn verifier(config: OidcConfig) -> OidcVerifier {
        let jwks = JwksCache::seeded(&config, JWKS).expect("test JWKS should parse");
        OidcVerifier {
            jwks,
            issuer: config.issuer.trim_end_matches('/').to_string(),
            audiences: config.audiences.clone(),
            leeway_secs: config.leeway_secs,
            principals: principal_map(&config),
        }
    }

    #[tokio::test]
    async fn test_valid_token_yields_the_configured_authority() {
        let ctx = verifier(test_config())
            .authenticate(VALID)
            .await
            .expect("a well-formed token from a known subject should verify");
        assert_eq!(ctx.key_id, "oidc:user-abc123");
        assert_eq!(ctx.namespace, "team");
        assert!(ctx.scopes.contains(&Scope::Read));
        assert!(ctx.scopes.contains(&Scope::Write));
        assert!(!ctx.scopes.contains(&Scope::Delete));
    }

    #[tokio::test]
    async fn test_expired_token_is_rejected() {
        let err = verifier(test_config()).authenticate(EXPIRED).await;
        assert!(matches!(err, Err(OidcError::Rejected(_))), "{:?}", err);
    }

    #[tokio::test]
    async fn test_token_signed_by_another_key_is_rejected() {
        let err = verifier(test_config()).authenticate(FORGED).await;
        assert!(matches!(err, Err(OidcError::Rejected(_))), "{:?}", err);
    }

    #[tokio::test]
    async fn test_unsigned_token_is_rejected() {
        // `alg: none` is not a JWS algorithm the header parser will produce, so
        // this fails before any key is consulted.
        let err = verifier(test_config()).authenticate(ALG_NONE).await;
        assert!(matches!(err, Err(OidcError::Malformed(_))), "{:?}", err);
    }

    #[tokio::test]
    async fn test_unknown_kid_does_not_fall_back_to_another_key() {
        // The key set holds exactly one key. Verifying against "whatever we
        // have" would accept this token, since it is genuinely signed by it.
        let err = verifier(test_config()).authenticate(UNKNOWN_KID).await;
        assert!(matches!(err, Err(OidcError::UnknownKey(_))), "{:?}", err);
    }

    #[tokio::test]
    async fn test_wrong_issuer_is_rejected() {
        let config = OidcConfig {
            issuer: "https://someone-else.test".into(),
            ..test_config()
        };
        let err = verifier(config).authenticate(VALID).await;
        assert!(matches!(err, Err(OidcError::Rejected(_))), "{:?}", err);
    }

    #[tokio::test]
    async fn test_wrong_audience_is_rejected() {
        let config = OidcConfig {
            audiences: vec!["some-other-service".into()],
            ..test_config()
        };
        let err = verifier(config).authenticate(VALID).await;
        assert!(matches!(err, Err(OidcError::Rejected(_))), "{:?}", err);
    }

    #[tokio::test]
    async fn test_verified_subject_without_a_principal_entry_is_forbidden() {
        let config = OidcConfig {
            principals: Vec::new(),
            ..test_config()
        };
        let err = verifier(config).authenticate(VALID).await;
        assert!(
            matches!(err, Err(OidcError::UnknownPrincipal(ref s)) if s == "user-abc123"),
            "{:?}",
            err
        );
        // 403, not 401: the token proved the subject, and re-presenting it will
        // not help.
        let api: ApiError = err.unwrap_err().into();
        assert_eq!(api.status, axum::http::StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_rejection_reasons_are_not_disclosed_to_the_client() {
        for token in [EXPIRED, FORGED, UNKNOWN_KID, ALG_NONE] {
            let err = verifier(test_config())
                .authenticate(token)
                .await
                .expect_err("token should be rejected");
            let api: ApiError = err.into();
            assert_eq!(api.status, axum::http::StatusCode::UNAUTHORIZED);
            assert_eq!(api.message, "Invalid bearer token");
        }
    }

    #[test]
    fn test_jwks_parse_skips_keys_the_gateway_will_not_verify_with() {
        let body = r#"{"keys": [
            {"kty":"oct","kid":"symmetric","k":"c2VjcmV0"},
            {"kty":"RSA","kid":"encryption-only","use":"enc","n":"vp9o62wq-MA","e":"AQAB"},
            {"kty":"RSA","n":"vp9o62wq-MA","e":"AQAB"},
            {"kty":"OKP","kid":"ed25519","crv":"Ed25519","x":"abc"}
        ]}"#;
        let keys = parse_jwks(body).expect("JWKS should parse");
        assert!(
            keys.is_empty(),
            "symmetric, encryption-only, kid-less and unsupported keys must all be dropped"
        );
    }

    #[test]
    fn test_jwk_alg_pins_the_algorithm() {
        let set: JwkSet = serde_json::from_str(JWKS).unwrap();
        let (kid, key) = verifying_key(&set.keys[0]).expect("RSA key should be usable");
        assert_eq!(kid, "test-key-1");
        // The published `alg: RS256` narrows the key to one algorithm rather
        // than every RSA algorithm it could technically verify.
        assert_eq!(key.algorithms, vec![Algorithm::RS256]);
    }

    #[test]
    fn test_jwk_without_alg_allows_its_key_types_algorithms() {
        let set: JwkSet = serde_json::from_str(
            r#"{"keys":[{"kty":"RSA","kid":"k","n":"vp9o62wq-MA","e":"AQAB"}]}"#,
        )
        .unwrap();
        let (_, key) = verifying_key(&set.keys[0]).expect("RSA key should be usable");
        assert!(key.algorithms.contains(&Algorithm::RS256));
        assert!(key.algorithms.contains(&Algorithm::PS512));
        assert!(!key.algorithms.contains(&Algorithm::ES256));
    }

    #[test]
    fn test_jwk_declaring_an_impossible_algorithm_is_dropped() {
        let set: JwkSet = serde_json::from_str(
            r#"{"keys":[{"kty":"RSA","kid":"k","alg":"ES256","n":"vp9o62wq-MA","e":"AQAB"}]}"#,
        )
        .unwrap();
        assert!(verifying_key(&set.keys[0]).is_none());
    }

    #[test]
    fn test_none_is_not_a_parseable_algorithm() {
        assert!(parse_algorithm("none").is_none());
        assert!(parse_algorithm("HS256").is_none());
        assert_eq!(parse_algorithm("RS256"), Some(Algorithm::RS256));
    }
}
