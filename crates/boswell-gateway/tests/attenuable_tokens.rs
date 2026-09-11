//! Integration tests for attenuable tokens on the wire (ADR-022).
//!
//! The unit tests in `tokens.rs` prove the Datalog. These prove the two things
//! only the assembled gateway can show: that a minted token is accepted by the
//! same middleware an API key goes through, and that an attenuated one is
//! refused by the handler rather than merely by a library call.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::get;
use axum::{middleware, Extension, Router};
use http_body_util::BodyExt; // for `collect`
use tower::ServiceExt; // for `oneshot`

use std::collections::HashSet;
use std::time::{Duration, SystemTime};

use boswell_domain::{
    Assurance, Authority, DelegationChain, EvidenceType, Op, ProvenanceStamp, Tier,
};
use boswell_gateway::auth::{auth_middleware, hash_key, AuthContext, Scope};
use boswell_gateway::config::{ApiKeyConfig, GatewayConfig, TokenConfig};
use boswell_gateway::error::ApiError;
use boswell_gateway::tokens::{attenuate, Attenuation};
use boswell_gateway::AppState;

const WRITE_KEY: &str = "read-write-key";

/// A dummy protected handler standing in for a write route: it asks for the
/// write scope and for a namespace, which is exactly the pair a token can be
/// attenuated on.
async fn writes_to_team_sub(
    Extension(ctx): Extension<AuthContext>,
) -> Result<&'static str, ApiError> {
    ctx.require(Scope::Write)?;
    ctx.require_namespace("team:sub")?;
    Ok("ok")
}

async fn reads(Extension(ctx): Extension<AuthContext>) -> Result<&'static str, ApiError> {
    ctx.require(Scope::Read)?;
    Ok("ok")
}

fn app(state: AppState) -> Router {
    Router::new()
        .route("/write", get(writes_to_team_sub))
        .route("/read", get(reads))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        .with_state(state)
}

/// A gateway with one read+write API key and a fresh token root key.
fn test_state() -> AppState {
    state_revoking_from("")
}

/// The same, with a revocation list read from `path` on every check.
fn state_revoking_from(path: &str) -> AppState {
    let keypair = biscuit_auth::KeyPair::new_with_algorithm(biscuit_auth::Algorithm::Ed25519);
    AppState::from_config(&GatewayConfig {
        api_keys: vec![ApiKeyConfig {
            id: "writer".into(),
            key_hash: hash_key(WRITE_KEY),
            namespace: "team".into(),
            scopes: vec!["read".into(), "write".into()],
        }],
        tokens: Some(TokenConfig {
            root_private_key: keypair.private().to_bytes_hex(),
            default_ttl_secs: 3600,
            max_ttl_secs: 86400,
            revocation_list_path: path.to_string(),
            revocation_refresh_secs: 0,
        }),
        ..GatewayConfig::default()
    })
}

async fn status_for(state: &AppState, path: &str, bearer: &str) -> StatusCode {
    let request = Request::builder()
        .uri(path)
        .header("authorization", format!("Bearer {}", bearer))
        .body(Body::empty())
        .unwrap();
    app(state.clone()).oneshot(request).await.unwrap().status()
}

/// Mint a root token the way `POST /v1/tokens` would, for the API key's grant.
fn root_token(state: &AppState) -> String {
    let authority = state.tokens().expect("tokens configured");
    let ctx = state
        .lookup_key(&hash_key(WRITE_KEY))
        .expect("key configured");
    authority.mint(&ctx, None).expect("mint").token
}

fn public_key(state: &AppState) -> biscuit_auth::PublicKey {
    biscuit_auth::PublicKey::from_bytes_hex(
        &state.tokens().unwrap().root_public_key_hex(),
        biscuit_auth::Algorithm::Ed25519,
    )
    .expect("public key")
}

#[tokio::test]
async fn a_root_token_is_accepted_wherever_the_key_it_came_from_is() {
    let state = test_state();
    let token = root_token(&state);
    assert_eq!(status_for(&state, "/write", &token).await, StatusCode::OK);
}

#[tokio::test]
async fn a_token_narrowed_to_read_is_refused_a_write() {
    let state = test_state();
    let narrowed = attenuate(
        &root_token(&state),
        public_key(&state),
        &Attenuation {
            scopes: Some(vec![Scope::Read]),
            ..Default::default()
        },
    )
    .expect("attenuate");

    assert_eq!(status_for(&state, "/read", &narrowed).await, StatusCode::OK);
    assert_eq!(
        status_for(&state, "/write", &narrowed).await,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn a_token_narrowed_to_a_sibling_namespace_is_refused() {
    let state = test_state();
    let narrowed = attenuate(
        &root_token(&state),
        public_key(&state),
        &Attenuation {
            namespace: Some("team:other".to_string()),
            ..Default::default()
        },
    )
    .expect("attenuate");

    // The scope survives; only the namespace check fails, which is what tells
    // us the two dimensions are evaluated independently.
    assert_eq!(status_for(&state, "/read", &narrowed).await, StatusCode::OK);
    assert_eq!(
        status_for(&state, "/write", &narrowed).await,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn a_refusal_names_the_dimension_and_not_the_datalog() {
    let state = test_state();
    let narrowed = attenuate(
        &root_token(&state),
        public_key(&state),
        &Attenuation {
            scopes: Some(vec![Scope::Read]),
            ..Default::default()
        },
    )
    .expect("attenuate");

    let request = Request::builder()
        .uri("/write")
        .header("authorization", format!("Bearer {}", narrowed))
        .body(Body::empty())
        .unwrap();
    let response = app(state).oneshot(request).await.unwrap();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let body = String::from_utf8(body.to_vec()).unwrap();

    assert!(body.contains("write"), "got {}", body);
    assert!(
        !body.contains("check") && !body.contains("reject if"),
        "a Datalog failure is not something a caller can act on: {}",
        body
    );
}

#[tokio::test]
async fn a_gateway_with_no_tokens_section_rejects_a_token_as_a_bad_key() {
    // Minted by a gateway that does issue them; presented to one that does not.
    let issuer = test_state();
    let token = root_token(&issuer);

    let bare = AppState::from_config(&GatewayConfig {
        api_keys: vec![ApiKeyConfig {
            id: "writer".into(),
            key_hash: hash_key(WRITE_KEY),
            namespace: "team".into(),
            scopes: vec!["read".into(), "write".into()],
        }],
        ..GatewayConfig::default()
    });

    assert_eq!(
        status_for(&bare, "/read", &token).await,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn a_token_from_another_gateways_root_key_is_401_not_403() {
    // 401: nothing about this token was established. A 403 would say the
    // signature was good and only the authority was short, which is a
    // disclosure and is also false.
    let theirs = test_state();
    let ours = test_state();
    assert_eq!(
        status_for(&ours, "/read", &root_token(&theirs)).await,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn an_api_key_still_reads_as_a_bad_key_when_tokens_are_configured() {
    // The regression #68 guarded against, in its token-shaped form: adding a
    // second credential type must not turn a stale API key into a confusing
    // token error.
    let state = test_state();
    let request = Request::builder()
        .uri("/read")
        .header("authorization", "Bearer stale-key")
        .body(Body::empty())
        .unwrap();
    let response = app(state).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let body = String::from_utf8(body.to_vec()).unwrap();
    assert!(body.contains("Invalid API key"), "got {}", body);
}

/// A revoked token is refused by the middleware, and told so.
///
/// The wire half of what `tokens.rs` proves about the list itself: a token that
/// authenticated a moment ago stops authenticating once its id is in the file,
/// with no restart and no second request to anything.
#[tokio::test]
async fn a_revoked_token_is_refused_at_the_middleware() {
    let path = std::env::temp_dir().join(format!(
        "boswell-wire-revocations-{}.txt",
        std::process::id()
    ));
    std::fs::write(&path, "# revoked tokens\n").expect("create the list");
    let state = state_revoking_from(path.to_str().unwrap());

    let authority = state.tokens().expect("tokens configured");
    let ctx = state.lookup_key(&hash_key(WRITE_KEY)).expect("the key");
    let minted = authority.mint(&ctx, None).expect("mint");

    assert_eq!(
        status_for(&state, "/read", &minted.token).await,
        StatusCode::OK,
        "the token works before it is revoked"
    );

    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    writeln!(file, "{}", minted.revocation_id).unwrap();
    file.sync_all().unwrap();

    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .uri("/read")
                .header("authorization", format!("Bearer {}", minted.token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let body = String::from_utf8_lossy(&body);
    assert!(
        body.contains("revoked"),
        "a holder is told its token was revoked rather than that the key is bad: {}",
        body
    );

    // The API key behind the same gateway is untouched: revocation ends tokens,
    // not the identity that minted them.
    assert_eq!(status_for(&state, "/read", WRITE_KEY).await, StatusCode::OK);
    let _ = std::fs::remove_file(path);
}

// ---------------------------------------------------------------------------
// The delegation root, as corroboration counts it (design §8.3).
// ---------------------------------------------------------------------------

/// The stamp the write path produces for a caller the gateway resolved to
/// `issued_to`.
///
/// This reproduces two hops the gateway itself cannot reach from here: the
/// gateway sets `ExecutionReceipt::issued_to` from `AuthContext::key_id`
/// (`handlers.rs`), and the instance sets both the stamp's author and its
/// one-element delegation chain from that receipt (`service.rs`). Only the
/// identity is under test, so every other field is a constant.
fn stamp_issued_to(issued_to: &str) -> ProvenanceStamp {
    ProvenanceStamp {
        author: issued_to.to_string(),
        delegation_chain: DelegationChain(vec![issued_to.to_string()]),
        authority: Authority {
            namespaces: vec!["team".into()],
            max_tier: Tier::Ephemeral,
            ops: vec![Op::Read, Op::Write],
        },
        evidence: EvidenceType::ToolOutput,
        assurance: Assurance::None,
        task_id: None,
        session_id: None,
        timestamp: 1,
        dev_provider: false,
    }
}

/// Mint, then narrow ten ways, then resolve each back to an independence root.
///
/// Returns the roots *and* the tokens, because the caller has to be able to
/// show the ten tokens were ten different credentials. A test that collapsed
/// ten copies of one string onto one root would pass while proving nothing.
fn ten_subagent_tokens(state: &AppState) -> (Vec<String>, HashSet<String>) {
    let root = root_token(state);
    let authority = state.tokens().expect("tokens configured");

    (0..10)
        .map(|i| {
            // Each subagent gets its own token, narrowed its own way. These are
            // ten distinct credentials, held separately, with distinct
            // revocation ids — everything except distinct authority.
            let narrowed = attenuate(
                &root,
                public_key(state),
                &Attenuation {
                    expires_at: Some(SystemTime::now() + Duration::from_secs(60 + i)),
                    ..Default::default()
                },
            )
            .expect("attenuate");

            let ctx = authority.authenticate(&narrowed).expect("authenticate");
            let root = stamp_issued_to(&ctx.key_id).independence_root().to_string();
            (narrowed, root)
        })
        .unzip()
}

/// Ten subagents of one agent are one witness, not ten.
///
/// This is the seam the Sybil defense hangs on, and it spans two crates that
/// cannot see each other: `boswell-gateway` resolves a presented token to a
/// principal, and `boswell-store` counts distinct independence roots. Nothing
/// structural forces the first to be a value the second collapses correctly.
/// Both halves are covered in their own crates — the token's `principal` comes
/// from the authority block (`a_block_a_holder_appended_cannot_add_authority`),
/// and equal roots count once (`subagents_of_one_credential_are_one_delegation_root`)
/// — but until this test, nothing joined them, so the property held by
/// accident rather than on purpose.
#[tokio::test]
async fn ten_subagent_tokens_are_one_delegation_root() {
    let (tokens, roots) = ten_subagent_tokens(&test_state());

    // The premise, checked rather than assumed: ten credentials, not one reused.
    assert_eq!(
        tokens.iter().collect::<HashSet<_>>().len(),
        10,
        "the ten subagents must hold ten different tokens for the collapse to mean anything"
    );

    assert_eq!(
        roots,
        HashSet::from(["writer".to_string()]),
        "ten delegated tokens must collapse onto the one principal they were minted from"
    );
}

/// The same ten tokens, counted the way the pre-#33 code counted: by author.
///
/// The contrast is the whole point of the item. If a delegate's token named the
/// delegate, each of these would be its own author *and* its own root, and ten
/// subagents would corroborate each other into a promotion. They do not, because
/// an attenuated token's grant is read from the authority block.
#[tokio::test]
async fn a_delegate_cannot_author_under_a_name_of_its_own() {
    let state = test_state();
    let root = root_token(&state);
    let authority = state.tokens().expect("tokens configured");

    let narrowed = attenuate(
        &root,
        public_key(&state),
        &Attenuation {
            namespace: Some("team:sub".to_string()),
            scopes: Some(vec![Scope::Read]),
            ..Default::default()
        },
    )
    .expect("attenuate");

    let ctx = authority.authenticate(&narrowed).expect("authenticate");
    assert_eq!(
        ctx.key_id, "writer",
        "narrowing a token must not rename its principal"
    );
}

/// A self-declared subagent path does not buy independence either.
///
/// The token half above stops a delegate renaming itself at the gateway. This
/// is the other half: even where an identity legitimately carries a subagent
/// suffix, the counting end strips it, so the unit stays the principal an
/// identity provider actually established.
#[tokio::test]
async fn a_subagent_suffix_does_not_split_one_principal_into_many() {
    let roots: HashSet<String> = ["writer/sub:explore-1", "writer/sub:explore-2", "writer"]
        .into_iter()
        .map(|id| stamp_issued_to(id).independence_root().to_string())
        .collect();

    assert_eq!(roots, HashSet::from(["writer".to_string()]));
}

/// The defense narrows corroboration; it does not abolish it.
///
/// Two genuinely distinct credentials are two independence roots, which is what
/// keeps §8.3's mitigation from collapsing into "nothing ever corroborates".
/// ADR-022 accepts an adversary holding two real credentials counting twice.
#[tokio::test]
async fn two_distinct_principals_are_two_delegation_roots() {
    let roots: HashSet<String> = ["writer", "auditor"]
        .into_iter()
        .map(|id| stamp_issued_to(id).independence_root().to_string())
        .collect();

    assert_eq!(
        roots.len(),
        2,
        "distinct credentials must still corroborate"
    );
}
