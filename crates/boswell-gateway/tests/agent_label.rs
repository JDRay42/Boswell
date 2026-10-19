//! The `X-Boswell-Agent` label is checked before a write reaches the instance
//! (ADR-023). These stop at the handler, so no live backend is needed: a
//! refused label never reaches the SDK client.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt; // for `oneshot`

use boswell_gateway::auth::hash_key;
use boswell_gateway::config::{ApiKeyConfig, GatewayConfig};
use boswell_gateway::{build_router, AppState};

const WRITE_KEY: &str = "write-key";

fn config() -> GatewayConfig {
    GatewayConfig {
        rate_limit_per_minute: 1000,
        api_keys: vec![ApiKeyConfig {
            id: "writer".into(),
            key_hash: hash_key(WRITE_KEY),
            namespace: "team".into(),
            scopes: vec!["write".into()],
        }],
        ..GatewayConfig::default()
    }
}

async fn post_with_label(uri: &str, body: &str, label: &str) -> StatusCode {
    let cfg = config();
    let app = build_router(&cfg, AppState::from_config(&cfg));
    let request = Request::builder()
        .method("POST")
        .uri(uri)
        .header("authorization", format!("Bearer {}", WRITE_KEY))
        .header("content-type", "application/json")
        .header("x-boswell-agent", label)
        .body(Body::from(body.to_string()))
        .unwrap();
    app.oneshot(request).await.unwrap().status()
}

const CLAIM: &str =
    r#"{"namespace":"team","subject":"DateParser","predicate":"provides","object":"ISO 8601"}"#;
const BATCH: &str =
    r#"{"claims":[{"namespace":"team","subject":"a","predicate":"b","object":"c"}]}"#;
const EXTRACT: &str = r#"{"namespace":"team","text":"DateParser parses ISO 8601."}"#;
const HOOK: &str = r#"{"hook_event_name":"UserPromptSubmit","prompt":"hello"}"#;

/// A `/` in the label would let a caller append a deeper derived identity of
/// its own choosing to the principal the gateway authenticated.
#[tokio::test]
async fn every_write_route_refuses_a_label_with_a_slash() {
    for (uri, body) in [
        ("/v1/claims", CLAIM),
        ("/v1/claims/batch", BATCH),
        ("/v1/extract", EXTRACT),
        ("/v1/hooks/ingest", HOOK),
    ] {
        assert_eq!(
            post_with_label(uri, body, "impl/../admin").await,
            StatusCode::BAD_REQUEST,
            "{} should refuse the label",
            uri
        );
    }
}

#[tokio::test]
async fn an_overlong_label_is_refused() {
    assert_eq!(
        post_with_label("/v1/claims", CLAIM, &"x".repeat(65)).await,
        StatusCode::BAD_REQUEST
    );
}
