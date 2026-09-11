//! Attenuable tokens: minting, verification and offline attenuation (ADR-022).
//!
//! OIDC establishes *who the person is* and lasts months; a biscuit carries
//! *what a delegate may do* and is narrowed locally, with no issuer in the loop.
//! [`TokenAuthority`] is the gateway half: it mints a **root token** from an
//! already-authenticated [`AuthContext`], and it resolves a presented token back
//! to an `AuthContext` for the request that carries it.
//!
//! # The vocabulary
//!
//! A root token's authority block states the grant as facts, and bounds it:
//!
//! ```datalog
//! principal("agent-key");
//! namespace("team");
//! scope("read");
//! scope("write");
//! check if time($time), $time <= 2026-10-10T00:00:00Z;
//! ```
//!
//! A holder attenuates by appending a block of **restrictions** over the facts
//! the gateway supplies per request — the operation being attempted, and the
//! namespace being touched:
//!
//! ```datalog
//! reject if operation($op), !{"read"}.contains($op);
//! reject if namespace_target($n), !($n == "team:sub" || $n.starts_with("team:sub:"));
//! check if time($time), $time <= 2026-09-11T00:00:00Z;
//! ```
//!
//! `reject if` rather than `check if` or `check all` is load-bearing. The gateway
//! authorizes one dimension at a time — [`AuthContext::require`] knows the
//! operation and not the namespace, [`AuthContext::require_namespace`] the
//! reverse — so a restriction has to pass *vacuously* when its fact is absent
//! from the request. Both `check` forms fail on no match: `check if` succeeds
//! only when some fact matches, and `check all`, despite reading as universal
//! quantification, returns its `found` flag and so fails on an empty match set
//! too. `reject if` is the negation — it fires only on a fact that is present
//! *and* outside the grant, which is exactly "narrow this".
//!
//! # What this does and does not decide
//!
//! Authority is still Boswell's. The authority block never grants more than the
//! `AuthContext` it was minted from, [`TokenAuthority::mint`] refuses to mint
//! from a token-authenticated caller (a delegate must attenuate, never re-mint),
//! and a block a holder appended cannot grant: biscuit scopes each block's facts
//! to its own origin and an authorizer rule trusts the authority block alone, so
//! a `scope("delete")` written into block 1 is invisible to the query that reads
//! the grant.
//!
//! Namespace enforcement remains per-handler. A token narrowed to a namespace
//! restricts only the paths that already call `require_namespace` — the same
//! property, and the same limit, an API key's namespace has today.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use biscuit_auth::builder::{date, fact, string, Term};
use biscuit_auth::{Algorithm, AuthorizerBuilder, Biscuit, BiscuitBuilder, BlockBuilder, KeyPair};
use thiserror::Error;

use crate::auth::{AuthContext, Scope};
use crate::config::TokenConfig;

/// Errors from minting, parsing or authorizing an attenuable token.
#[derive(Debug, Error)]
pub enum TokenError {
    /// The configured root key was not a valid Ed25519 private key.
    #[error("invalid root private key: {0}")]
    RootKey(String),

    /// A delegate tried to mint rather than attenuate.
    #[error("a token holder cannot mint a new token; attenuate the one you hold")]
    AlreadyDelegated,

    /// The requested lifetime exceeded `max_ttl_secs`.
    #[error("requested lifetime {requested}s exceeds the configured maximum of {maximum}s")]
    TtlTooLong {
        /// Lifetime the caller asked for, in seconds.
        requested: u64,
        /// Configured ceiling, in seconds.
        maximum: u64,
    },

    /// The token did not parse, did not verify, or failed its own checks.
    #[error("token rejected: {0}")]
    Rejected(String),

    /// The token verified but its authority block was not one this gateway minted.
    #[error("token is not a Boswell grant: {0}")]
    Malformed(String),
}

/// A freshly minted root token and when it stops being valid.
#[derive(Debug, Clone)]
pub struct MintedToken {
    /// The token itself, base64, to be presented as `Authorization: Bearer`.
    pub token: String,
    /// Expiry as a Unix timestamp in seconds.
    pub expires_at: u64,
}

/// The gateway's token-minting and token-verifying half.
///
/// Holds the root key pair. The private half signs authority blocks; the public
/// half is all a verifier needs, which is the property that keeps federation
/// open — an instance can verify a token it could never have minted.
pub struct TokenAuthority {
    root: KeyPair,
    default_ttl: Duration,
    max_ttl: Duration,
}

impl std::fmt::Debug for TokenAuthority {
    /// Deliberately prints the public key only. A root private key in a log
    /// line is the whole grant.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenAuthority")
            .field("root_public_key", &self.root.public().to_bytes_hex())
            .field("default_ttl", &self.default_ttl)
            .field("max_ttl", &self.max_ttl)
            .finish()
    }
}

impl TokenAuthority {
    /// Build from config, decoding the hex root private key.
    pub fn new(config: &TokenConfig) -> Result<Self, TokenError> {
        let private = biscuit_auth::PrivateKey::from_bytes_hex(
            config.root_private_key.trim(),
            Algorithm::Ed25519,
        )
        .map_err(|e| TokenError::RootKey(e.to_string()))?;

        Ok(Self {
            root: KeyPair::from(&private),
            default_ttl: Duration::from_secs(config.default_ttl_secs),
            max_ttl: Duration::from_secs(config.max_ttl_secs),
        })
    }

    /// The root public key, hex encoded. Safe to publish; it is what a verifier
    /// needs and it cannot mint.
    pub fn root_public_key_hex(&self) -> String {
        self.root.public().to_bytes_hex()
    }

    /// Mint a root token carrying exactly the authority of `ctx`.
    ///
    /// Refuses when `ctx` itself came from a token: a delegate that could mint
    /// could launder away its own attenuation, which is the property the whole
    /// mechanism exists to hold.
    pub fn mint(
        &self,
        ctx: &AuthContext,
        ttl: Option<Duration>,
    ) -> Result<MintedToken, TokenError> {
        if ctx.token.is_some() {
            return Err(TokenError::AlreadyDelegated);
        }

        let ttl = ttl.unwrap_or(self.default_ttl);
        if ttl > self.max_ttl {
            return Err(TokenError::TtlTooLong {
                requested: ttl.as_secs(),
                maximum: self.max_ttl.as_secs(),
            });
        }

        self.mint_at(ctx, SystemTime::now() + ttl)
    }

    /// Mint with an explicit expiry. The public [`Self::mint`] computes one from
    /// a lifetime; tests use this to produce a token that is already stale
    /// without sleeping through its life.
    fn mint_at(&self, ctx: &AuthContext, expiry: SystemTime) -> Result<MintedToken, TokenError> {
        let mut builder = BiscuitBuilder::new()
            .fact(fact("principal", &[string(&ctx.key_id)]))
            .and_then(|b| b.fact(fact("namespace", &[string(&ctx.namespace)])))
            .map_err(|e| TokenError::Rejected(e.to_string()))?;

        // Sorted so a token is a function of the grant and not of hash order:
        // two mints of the same authority produce the same authority block.
        let mut scopes: Vec<&str> = ctx.scopes.iter().map(Scope::as_str).collect();
        scopes.sort_unstable();
        for scope in scopes {
            builder = builder
                .fact(fact("scope", &[string(scope)]))
                .map_err(|e| TokenError::Rejected(e.to_string()))?;
        }

        let biscuit = builder
            .code_with_params(
                "check if time($time), $time <= {expiry};",
                HashMap::from([("expiry".to_string(), date(&expiry))]),
                HashMap::new(),
            )
            .and_then(|b| b.build(&self.root))
            .map_err(|e| TokenError::Rejected(e.to_string()))?;

        Ok(MintedToken {
            token: biscuit
                .to_base64()
                .map_err(|e| TokenError::Rejected(e.to_string()))?,
            expires_at: unix_seconds(expiry),
        })
    }

    /// Verify a presented token and resolve it to an [`AuthContext`].
    ///
    /// The returned context carries the authority block's grant *and* the parsed
    /// token, so the per-request checks in [`AuthContext::require`] and
    /// [`AuthContext::require_namespace`] can run the attenuation blocks against
    /// the facts of the request they belong to.
    pub fn authenticate(&self, token: &str) -> Result<AuthContext, TokenError> {
        let biscuit = Biscuit::from_base64(token, self.root.public())
            .map_err(|e| TokenError::Rejected(e.to_string()))?;

        // A bare authorization with no request facts proves the signature chain
        // and the lifetime bounds. Restrictions are `check all`, so they hold
        // vacuously here and bite in `require`/`require_namespace` instead.
        authorize(&biscuit, &[]).map_err(TokenError::Rejected)?;

        let principal = single_string(&biscuit, "principal")?;
        let namespace = single_string(&biscuit, "namespace")?;
        let scopes = query_strings(&biscuit, "scope")?
            .iter()
            .filter_map(|s| Scope::parse(s))
            .collect();

        Ok(AuthContext {
            key_id: principal,
            namespace,
            scopes,
            token: Some(Arc::new(biscuit)),
        })
    }
}

/// One narrowing of a token, applied by its holder with no network call.
///
/// Every field is a restriction. `None` leaves that dimension as the parent had
/// it; a value can only ever remove authority, because the parent's own blocks
/// are still there and are still checked.
#[derive(Debug, Clone, Default)]
pub struct Attenuation {
    /// Restrict to this namespace and its children.
    pub namespace: Option<String>,
    /// Restrict to these operations.
    pub scopes: Option<Vec<Scope>>,
    /// Expire this sooner than the parent does.
    ///
    /// An instant rather than a lifetime, because the useful thing to say is
    /// "no later than my own expiry", which a holder computes from the token it
    /// already has.
    pub expires_at: Option<SystemTime>,
}

/// Narrow `token` and return the result, base64 encoded.
///
/// This is the delegate's operation, not the gateway's — it needs no private
/// key, no network call and no issuer. It lives here because the Datalog it
/// writes has to match the facts [`TokenAuthority`] supplies per request, and a
/// vocabulary with two independent definitions is a vocabulary with one bug.
///
/// `root_public_key` is only used to parse the token; appending a block does not
/// need it and cannot be prevented by withholding it.
pub fn attenuate(
    token: &str,
    root_public_key: biscuit_auth::PublicKey,
    attenuation: &Attenuation,
) -> Result<String, TokenError> {
    let biscuit = Biscuit::from_base64(token, root_public_key)
        .map_err(|e| TokenError::Rejected(e.to_string()))?;

    let mut block = BlockBuilder::new();

    if let Some(namespace) = &attenuation.namespace {
        // Exactly `namespace_allows`: the target is the namespace itself or a
        // child of it. Both literals are baked in at attenuation time, so the
        // check needs no string arithmetic at authorization time.
        block = block
            .code_with_params(
                "reject if namespace_target($n), !($n == {ns} || $n.starts_with({prefix}));",
                HashMap::from([
                    ("ns".to_string(), string(namespace)),
                    ("prefix".to_string(), string(&format!("{}:", namespace))),
                ]),
                HashMap::new(),
            )
            .map_err(|e| TokenError::Rejected(e.to_string()))?;
    }

    if let Some(scopes) = &attenuation.scopes {
        let allowed: std::collections::BTreeSet<Term> = scopes
            .iter()
            .map(|s| Term::Str(s.as_str().to_string()))
            .collect();
        block = block
            .code_with_params(
                "reject if operation($op), !{allowed}.contains($op);",
                HashMap::from([("allowed".to_string(), Term::Set(allowed))]),
                HashMap::new(),
            )
            .map_err(|e| TokenError::Rejected(e.to_string()))?;
    }

    if let Some(expiry) = attenuation.expires_at {
        block = block
            .code_with_params(
                "check if time($time), $time <= {expiry};",
                HashMap::from([("expiry".to_string(), date(&expiry))]),
                HashMap::new(),
            )
            .map_err(|e| TokenError::Rejected(e.to_string()))?;
    }

    biscuit
        .append(block)
        .and_then(|b| b.to_base64())
        .map_err(|e| TokenError::Rejected(e.to_string()))
}

/// Run the token's own checks against the facts of one request.
///
/// `facts` are `(predicate, value)` pairs — `operation`/`namespace_target` — and
/// the policy is a bare `allow if true`: the token either survives its blocks or
/// it does not. Nothing here grants; the grant was read from the authority block
/// when the token was authenticated.
pub(crate) fn authorize(biscuit: &Biscuit, facts: &[(&str, &str)]) -> Result<(), String> {
    let mut builder = AuthorizerBuilder::new().time();
    for (predicate, value) in facts {
        builder = builder
            .fact(fact(predicate, &[string(value)]))
            .map_err(|e| e.to_string())?;
    }
    builder
        .policy("allow if true")
        .and_then(|b| b.build(biscuit))
        .map_err(|e| e.to_string())?
        .authorize()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Read every value of a single-term string fact from the authority block.
fn query_strings(biscuit: &Biscuit, predicate: &str) -> Result<Vec<String>, TokenError> {
    let mut authorizer = AuthorizerBuilder::new()
        .time()
        .policy("allow if true")
        .and_then(|b| b.build(biscuit))
        .map_err(|e| TokenError::Malformed(e.to_string()))?;

    let rule = format!("data($v) <- {}($v)", predicate);
    let values: Vec<(String,)> = authorizer
        .query(rule.as_str())
        .map_err(|e| TokenError::Malformed(e.to_string()))?;
    Ok(values.into_iter().map(|(v,)| v).collect())
}

/// Read a fact this gateway always mints exactly one of.
fn single_string(biscuit: &Biscuit, predicate: &str) -> Result<String, TokenError> {
    let mut values = query_strings(biscuit, predicate)?;
    match values.len() {
        1 => Ok(values.remove(0)),
        n => Err(TokenError::Malformed(format!(
            "expected exactly one `{}` fact, found {}",
            predicate, n
        ))),
    }
}

/// Seconds since the Unix epoch, saturating at 0 for pre-epoch times.
fn unix_seconds(time: SystemTime) -> u64 {
    time.duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// A throwaway root key, regenerated per test. Nothing here needs a fixed
    /// vector: unlike the OIDC tests, both halves of the key are ours to make.
    fn authority() -> TokenAuthority {
        let keypair = KeyPair::new_with_algorithm(Algorithm::Ed25519);
        TokenAuthority::new(&TokenConfig {
            root_private_key: keypair.private().to_bytes_hex(),
            default_ttl_secs: 3600,
            max_ttl_secs: 86400,
        })
        .expect("a freshly generated key should load")
    }

    fn context(namespace: &str, scopes: &[Scope]) -> AuthContext {
        AuthContext {
            key_id: "agent".to_string(),
            namespace: namespace.to_string(),
            scopes: scopes.iter().copied().collect::<HashSet<_>>(),
            token: None,
        }
    }

    #[test]
    fn a_minted_token_round_trips_to_the_grant_it_was_minted_from() {
        let authority = authority();
        let minted = authority
            .mint(&context("team", &[Scope::Read, Scope::Write]), None)
            .expect("mint");

        let ctx = authority.authenticate(&minted.token).expect("authenticate");
        assert_eq!(ctx.key_id, "agent");
        assert_eq!(ctx.namespace, "team");
        assert_eq!(
            ctx.scopes,
            [Scope::Read, Scope::Write]
                .into_iter()
                .collect::<HashSet<_>>()
        );
        assert!(
            ctx.token.is_some(),
            "the parsed token must travel with the context"
        );
    }

    #[test]
    fn an_unrestricted_grant_survives_the_empty_namespace() {
        // `namespace("")` is a legitimate fact meaning "unrestricted", not an
        // absent one. If it were dropped at mint, `single_string` would fail to
        // find it and the token would read as malformed.
        let authority = authority();
        let minted = authority
            .mint(&context("", &[Scope::Read]), None)
            .expect("mint");
        let ctx = authority.authenticate(&minted.token).expect("authenticate");
        assert_eq!(ctx.namespace, "");
        assert!(ctx.allows_namespace("anything"));
    }

    #[test]
    fn a_token_signed_by_another_root_is_rejected() {
        let minted = authority()
            .mint(&context("team", &[Scope::Read]), None)
            .expect("mint");
        let err = authority()
            .authenticate(&minted.token)
            .expect_err("a different root key must not verify");
        assert!(matches!(err, TokenError::Rejected(_)), "got {:?}", err);
    }

    #[test]
    fn a_holder_cannot_mint_a_fresh_token_from_one_it_holds() {
        // The whole mechanism rests on this. If a delegate could mint, it could
        // mint away the attenuation its parent applied.
        let authority = authority();
        let minted = authority
            .mint(&context("team", &[Scope::Read]), None)
            .expect("mint");
        let delegate = authority.authenticate(&minted.token).expect("authenticate");

        let err = authority
            .mint(&delegate, None)
            .expect_err("a token holder must not be able to mint");
        assert!(matches!(err, TokenError::AlreadyDelegated), "got {:?}", err);
    }

    #[test]
    fn a_lifetime_beyond_the_ceiling_is_refused() {
        let err = authority()
            .mint(
                &context("team", &[Scope::Read]),
                Some(Duration::from_secs(86_401)),
            )
            .expect_err("max_ttl_secs must be a ceiling");
        assert!(
            matches!(
                err,
                TokenError::TtlTooLong {
                    requested: 86_401,
                    maximum: 86_400
                }
            ),
            "got {:?}",
            err
        );
    }

    #[test]
    fn an_expired_token_does_not_authenticate() {
        let authority = authority();
        let minted = authority
            .mint_at(
                &context("team", &[Scope::Read]),
                SystemTime::now() - Duration::from_secs(60),
            )
            .expect("mint");

        let err = authority
            .authenticate(&minted.token)
            .expect_err("an expired token must not authenticate");
        assert!(matches!(err, TokenError::Rejected(_)), "got {:?}", err);
    }

    // -- attenuation ------------------------------------------------------

    /// Mint, attenuate, and hand back the context the gateway would build.
    fn attenuated(
        authority: &TokenAuthority,
        ctx: AuthContext,
        attenuation: &Attenuation,
    ) -> AuthContext {
        let minted = authority.mint(&ctx, None).expect("mint");
        let narrowed = attenuate(
            &minted.token,
            biscuit_auth::PublicKey::from_bytes_hex(
                &authority.root_public_key_hex(),
                Algorithm::Ed25519,
            )
            .expect("public key"),
            attenuation,
        )
        .expect("attenuate");
        authority.authenticate(&narrowed).expect("authenticate")
    }

    #[test]
    fn attenuating_scopes_removes_the_operation_and_keeps_the_others() {
        let authority = authority();
        let ctx = attenuated(
            &authority,
            context("team", &[Scope::Read, Scope::Write, Scope::Delete]),
            &Attenuation {
                scopes: Some(vec![Scope::Read]),
                ..Default::default()
            },
        );

        assert!(ctx.require(Scope::Read).is_ok());
        assert!(ctx.require(Scope::Write).is_err());
        assert!(ctx.require(Scope::Delete).is_err());
    }

    #[test]
    fn a_scope_attenuation_does_not_block_a_namespace_check() {
        // This is what `check all` buys. A namespace check supplies no
        // `operation` fact, so a scope restriction has to pass vacuously; under
        // `check if` it would fail and every write would 403.
        let authority = authority();
        let ctx = attenuated(
            &authority,
            context("team", &[Scope::Read, Scope::Write]),
            &Attenuation {
                scopes: Some(vec![Scope::Read]),
                ..Default::default()
            },
        );
        assert!(ctx.require_namespace("team:sub").is_ok());
    }

    #[test]
    fn attenuating_the_namespace_admits_the_namespace_and_its_children_only() {
        let authority = authority();
        let ctx = attenuated(
            &authority,
            context("team", &[Scope::Read, Scope::Write]),
            &Attenuation {
                namespace: Some("team:sub".to_string()),
                ..Default::default()
            },
        );

        assert!(ctx.require_namespace("team:sub").is_ok());
        assert!(ctx.require_namespace("team:sub:deeper").is_ok());
        assert!(ctx.require_namespace("team:other").is_err());
        // The same off-by-one `namespace_allows` guards against: a sibling whose
        // name merely starts with the granted one is not a child.
        assert!(ctx.require_namespace("team:subtle").is_err());
        // Still unrestricted on the dimension it did not narrow.
        assert!(ctx.require(Scope::Write).is_ok());
    }

    #[test]
    fn attenuation_cannot_widen_what_its_parent_narrowed() {
        // The parent's block is still in the token and is still checked, so a
        // second block naming more scopes than the first does not restore them.
        let authority = authority();
        let minted = authority
            .mint(&context("team", &[Scope::Read, Scope::Write]), None)
            .expect("mint");
        let public = biscuit_auth::PublicKey::from_bytes_hex(
            &authority.root_public_key_hex(),
            Algorithm::Ed25519,
        )
        .expect("public key");

        let narrowed = attenuate(
            &minted.token,
            public,
            &Attenuation {
                scopes: Some(vec![Scope::Read]),
                ..Default::default()
            },
        )
        .expect("attenuate");
        let widened = attenuate(
            &narrowed,
            public,
            &Attenuation {
                scopes: Some(vec![Scope::Read, Scope::Write]),
                ..Default::default()
            },
        )
        .expect("attenuate again");

        let ctx = authority.authenticate(&widened).expect("authenticate");
        assert!(ctx.require(Scope::Read).is_ok());
        assert!(
            ctx.require(Scope::Write).is_err(),
            "a later block must not restore authority an earlier one removed"
        );
    }

    #[test]
    fn attenuating_the_lifetime_expires_the_child_without_touching_the_parent() {
        let authority = authority();
        let minted = authority
            .mint(&context("team", &[Scope::Read]), None)
            .expect("mint");
        let public = biscuit_auth::PublicKey::from_bytes_hex(
            &authority.root_public_key_hex(),
            Algorithm::Ed25519,
        )
        .expect("public key");

        let expired = attenuate(
            &minted.token,
            public,
            &Attenuation {
                expires_at: Some(SystemTime::now() - Duration::from_secs(60)),
                ..Default::default()
            },
        )
        .expect("attenuate");

        assert!(authority.authenticate(&expired).is_err());
        assert!(
            authority.authenticate(&minted.token).is_ok(),
            "attenuation produces a new token and leaves the original alone"
        );
    }

    #[test]
    fn attenuation_needs_no_private_key() {
        // The point of the mechanism: a delegate narrows offline, holding only
        // the public key it was handed alongside the token.
        let authority = authority();
        let minted = authority
            .mint(&context("team", &[Scope::Read]), None)
            .expect("mint");
        let public = biscuit_auth::PublicKey::from_bytes_hex(
            &authority.root_public_key_hex(),
            Algorithm::Ed25519,
        )
        .expect("public key");

        let narrowed =
            attenuate(&minted.token, public, &Attenuation::default()).expect("attenuate");
        assert!(authority.authenticate(&narrowed).is_ok());
    }

    #[test]
    fn a_principal_with_datalog_in_its_name_is_a_string_and_not_code() {
        // `key_id` is `oidc:<sub>` for an identity-provider subject, which is
        // provider-controlled. Facts are built from terms rather than by
        // formatting into a source string, so quotes and semicolons in a subject
        // cannot close a fact and open a new one.
        let authority = authority();
        let hostile = r#"x"); scope("delete"); principal("#;
        let mut ctx = context("team", &[Scope::Read]);
        ctx.key_id = hostile.to_string();
        let minted = authority.mint(&ctx, None).expect("mint");
        let parsed = authority.authenticate(&minted.token).expect("authenticate");

        assert_eq!(parsed.key_id, hostile);
        assert_eq!(
            parsed.scopes,
            [Scope::Read].into_iter().collect::<HashSet<_>>()
        );
    }

    #[test]
    fn a_block_a_holder_appended_cannot_add_authority() {
        // The escalation to fear. `authenticate` reads the grant out of the
        // token by querying the authorizer, and a holder can append whatever
        // Datalog it likes. Biscuit scopes a block's facts to that block's
        // origin and an authorizer rule trusts the authority block only, so a
        // `scope("delete")` in block 1 is invisible to the query — but nothing
        // in *our* code says so, which is why this is a test and not a comment.
        let authority = authority();
        let minted = authority
            .mint(&context("team", &[Scope::Read]), None)
            .expect("mint");

        let biscuit = Biscuit::from_base64(&minted.token, authority.root.public()).expect("parse");
        let forged = biscuit
            .append(
                BlockBuilder::new()
                    .code(r#"scope("delete"); namespace("*"); principal("someone-else");"#)
                    .expect("block"),
            )
            .and_then(|b| b.to_base64())
            .expect("append");

        let ctx = authority.authenticate(&forged).expect("authenticate");
        assert_eq!(ctx.key_id, "agent");
        assert_eq!(ctx.namespace, "team");
        assert_eq!(
            ctx.scopes,
            [Scope::Read].into_iter().collect::<HashSet<_>>(),
            "a holder's own block must not grant it anything"
        );
        assert!(ctx.require(Scope::Delete).is_err());
    }

    #[test]
    fn a_malformed_root_key_is_an_error_and_not_a_panic() {
        let err = TokenAuthority::new(&TokenConfig {
            root_private_key: "not hex".to_string(),
            ..TokenConfig::default()
        })
        .expect_err("a bad key must not load");
        assert!(matches!(err, TokenError::RootKey(_)), "got {:?}", err);
    }

    #[test]
    fn the_debug_output_never_carries_the_private_key() {
        let keypair = KeyPair::new_with_algorithm(Algorithm::Ed25519);
        let private_hex = keypair.private().to_bytes_hex();
        let authority = TokenAuthority::new(&TokenConfig {
            root_private_key: private_hex.clone(),
            ..TokenConfig::default()
        })
        .expect("load");

        let rendered = format!("{:?}", authority);
        assert!(!rendered.contains(&private_hex));
        assert!(rendered.contains(&authority.root_public_key_hex()));
    }
}
