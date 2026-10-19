//! The hooks and the bridge against stand-ins for the Boswell gateway and the
//! OpenRig daemon, served over real HTTP on loopback.

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use boswell_openrig::boswell::Boswell;
use boswell_openrig::bridge::{self, BridgeConfig, Inbound, State as BridgeState};
use boswell_openrig::config::BoswellConfig;
use boswell_openrig::hook::{self, HookEvent};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Everything the stand-ins were asked, in order.
#[derive(Default)]
struct Seen {
    requests: Vec<(String, Value, HashMap<String, String>)>,
    feed_events: Vec<Value>,
}

type Shared = Arc<Mutex<Seen>>;

fn headers_of(h: &HeaderMap) -> HashMap<String, String> {
    h.iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect()
}

async fn serve(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{}", addr)
}

/// A gateway stand-in: claims get sequential ids, the feed serves whatever the
/// test loaded into `feed_events`, and the head is 99.
async fn gateway(seen: Shared) -> String {
    let app = Router::new()
        .route(
            "/v1/claims",
            post(
                |State(s): State<Shared>, h: HeaderMap, Json(b): Json<Value>| async move {
                    let mut s = s.lock().unwrap();
                    s.requests
                        .push(("POST /v1/claims".into(), b, headers_of(&h)));
                    Json(json!({ "id": format!("claim-{}", s.requests.len()) }))
                },
            ),
        )
        .route(
            "/v1/claims/:id",
            delete(
                |State(s): State<Shared>, Path(id): Path<String>| async move {
                    s.lock().unwrap().requests.push((
                        format!("DELETE /v1/claims/{}", id),
                        Value::Null,
                        HashMap::new(),
                    ));
                    Json(json!({ "deleted": true }))
                },
            ),
        )
        .route(
            "/v1/feed",
            get(
                |State(s): State<Shared>, Query(q): Query<HashMap<String, String>>| async move {
                    let mut s = s.lock().unwrap();
                    s.requests
                        .push(("GET /v1/feed".into(), json!(q), HashMap::new()));
                    if q.get("from").map(String::as_str) == Some("head") {
                        return Json(json!({ "events": [], "next": "99", "head": "99" }));
                    }
                    let events = std::mem::take(&mut s.feed_events);
                    let next = events
                        .last()
                        .and_then(|e| e["cursor"].as_str())
                        .unwrap_or("99")
                        .to_string();
                    Json(json!({ "events": events, "next": next, "head": next }))
                },
            ),
        )
        .route(
            "/v1/dossier",
            post(|State(s): State<Shared>, Json(b): Json<Value>| async move {
                s.lock().unwrap().requests.push((
                    "POST /v1/dossier".into(),
                    b.clone(),
                    HashMap::new(),
                ));
                Json(json!({
                    "view": "width",
                    "groups": [{ "subject": b["subject"], "predicate": "is being built",
                                 "standing": "thin", "claim_count": 1, "root_count": 1,
                                 "lead": "by impl-1@build (OpenRig queue item q7)" }],
                    "procedures": [], "goals": []
                }))
            }),
        )
        .with_state(seen);
    serve(app).await
}

/// An OpenRig stand-in that records stream posts.
async fn openrig(seen: Shared) -> String {
    let app = Router::new()
        .route(
            "/api/stream/emit",
            post(
                |State(s): State<Shared>, h: HeaderMap, Json(b): Json<Value>| async move {
                    s.lock().unwrap().requests.push((
                        "POST /api/stream/emit".into(),
                        b.clone(),
                        headers_of(&h),
                    ));
                    (axum::http::StatusCode::CREATED, Json(b))
                },
            ),
        )
        .with_state(seen);
    serve(app).await
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("boswell-openrig-{}-{}", name, std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    dir
}

fn env_with(pairs: Vec<(&'static str, String)>) -> impl Fn(&str) -> Option<String> {
    let map: HashMap<&str, String> = pairs.into_iter().collect();
    move |k| map.get(k).cloned()
}

#[tokio::test]
async fn session_start_injects_the_width_dossier_and_starts_following_from_now() {
    let seen = Shared::default();
    let url = gateway(seen.clone()).await;
    let dir = temp_dir("session");
    let env = env_with(vec![
        ("BOSWELL_URL", url),
        ("BOSWELL_TOKEN", "t".into()),
        ("BOSWELL_SUBJECTS", "DateParser".into()),
        ("OPENRIG_SESSION_NAME", "impl-2@build".into()),
        (
            "BOSWELL_OPENRIG_STATE_DIR",
            dir.to_string_lossy().into_owned(),
        ),
    ]);

    let out = hook::run(
        HookEvent::SessionStart,
        &json!({ "source": "startup" }),
        &env,
    )
    .await
    .expect("a configured seat gets context");

    assert_eq!(out["hookSpecificOutput"]["hookEventName"], "SessionStart");
    let context = out["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(context.starts_with(hook::PREAMBLE));
    assert!(context
        .contains("- thin: DateParser is being built → by impl-1@build (OpenRig queue item q7)"));
    let cursor = std::fs::read_to_string(dir.join("cursors/impl-2@build.cursor")).unwrap();
    assert_eq!(cursor, "99");
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn a_prompt_gets_teammates_changes_but_not_its_own() {
    let seen = Shared::default();
    let url = gateway(seen.clone()).await;
    let dir = temp_dir("prompt");
    std::fs::create_dir_all(dir.join("cursors")).unwrap();
    std::fs::write(dir.join("cursors/impl-2@build.cursor"), "40").unwrap();
    seen.lock().unwrap().feed_events = vec![
        json!({ "cursor": "41", "kind": "claim", "change": "created", "tier": { "after": "task" },
                "summary": "DateParser provides ISO 8601", "author": "rig/sub:impl-2@build" }),
        json!({ "cursor": "42", "kind": "claim", "change": "created", "tier": { "after": "task" },
                "summary": "Tokenizer is located at src/tok.rs", "author": "rig/sub:impl-1@build" }),
    ];
    let env = env_with(vec![
        ("BOSWELL_URL", url),
        ("BOSWELL_TOKEN", "t".into()),
        ("OPENRIG_SESSION_NAME", "impl-2@build".into()),
        (
            "BOSWELL_OPENRIG_STATE_DIR",
            dir.to_string_lossy().into_owned(),
        ),
    ]);

    let out = hook::run(HookEvent::UserPromptSubmit, &json!({}), &env)
        .await
        .expect("a teammate's change is news");
    let context = out["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(context.contains("Tokenizer is located at src/tok.rs"));
    assert!(
        !context.contains("DateParser"),
        "a seat's own writes are not news to it"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("cursors/impl-2@build.cursor")).unwrap(),
        "42"
    );

    // The seats share a key, so the hook must not let the gateway drop
    // everything by that key.
    let feed_query = seen
        .lock()
        .unwrap()
        .requests
        .iter()
        .find(|(r, _, _)| r == "GET /v1/feed")
        .map(|(_, q, _)| q.clone())
        .unwrap();
    assert_eq!(feed_query["exclude_self"], "false");
    assert_eq!(feed_query["since"], "40");

    // Nothing new: nothing printed.
    assert!(hook::run(HookEvent::UserPromptSubmit, &json!({}), &env)
        .await
        .is_none());
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn an_unconfigured_seat_prints_nothing() {
    let env = env_with(vec![]);
    assert!(hook::run(HookEvent::SessionStart, &json!({}), &env)
        .await
        .is_none());
}

#[tokio::test]
async fn queue_work_is_announced_by_the_seat_and_retired_when_done() {
    let seen = Shared::default();
    let url = gateway(seen.clone()).await;
    let boswell = Boswell::new(
        BoswellConfig {
            url,
            token: "bridge-key".into(),
            namespace: Some("team".into()),
        },
        Duration::from_secs(5),
    )
    .unwrap();
    let mut state = BridgeState::default();

    let claimed = bridge::inbound(&json!({
        "type": "queue.claimed", "qitemId": "q7", "destinationSession": "impl-1@build",
        "summary": "Build DateParser"
    }))
    .unwrap();
    bridge::apply(&boswell, &mut state, claimed.clone())
        .await
        .unwrap();
    // A repeat (a replay after reconnecting) does not announce twice.
    bridge::apply(&boswell, &mut state, claimed).await.unwrap();
    bridge::apply(
        &boswell,
        &mut state,
        Inbound::Complete {
            qitem: "q7".into(),
            subject: "Build DateParser".into(),
            seat: "impl-1@build".into(),
        },
    )
    .await
    .unwrap();

    let requests = std::mem::take(&mut seen.lock().unwrap().requests);
    assert_eq!(requests.len(), 3);

    let (route, body, headers) = &requests[0];
    assert_eq!(route, "POST /v1/claims");
    assert_eq!(body["subject"], "Build DateParser");
    assert_eq!(body["predicate"], bridge::ANNOUNCE);
    assert_eq!(body["namespace"], "team");
    assert_eq!(
        headers["x-boswell-agent"], "impl-1@build",
        "attributed to the seat"
    );
    assert_eq!(headers["authorization"], "Bearer bridge-key");

    assert_eq!(requests[1].0, "DELETE /v1/claims/claim-1");
    assert_eq!(requests[2].1["predicate"], bridge::COMPLETED);
    assert!(state.announcements.is_empty());
}

#[tokio::test]
async fn notable_boswell_changes_reach_the_openrig_stream_once() {
    let seen = Shared::default();
    let gw = gateway(seen.clone()).await;
    let rig = openrig(seen.clone()).await;
    let boswell = Boswell::new(
        BoswellConfig {
            url: gw,
            token: "bridge-key".into(),
            namespace: None,
        },
        Duration::from_secs(5),
    )
    .unwrap();
    let dir = temp_dir("forward");
    let config = BridgeConfig {
        openrig_url: rig,
        state_path: dir.join("bridge.json"),
        poll_every: Duration::from_secs(1),
        outbound_changes: vec!["contradicted".into()],
    };
    let http = reqwest::Client::new();
    let state = Arc::new(tokio::sync::Mutex::new(BridgeState::default()));

    // First run follows from now; nothing is replayed into the rig.
    assert_eq!(
        bridge::forward_boswell(&boswell, &state, &config, &http)
            .await
            .unwrap(),
        0
    );
    assert_eq!(state.lock().await.boswell_cursor.as_deref(), Some("99"));

    seen.lock().unwrap().feed_events = vec![json!({
        "cursor": "100", "kind": "claim", "change": "contradicted",
        "tier": { "after": "project" }, "summary": "DateParser is thread safe yes"
    })];
    assert_eq!(
        bridge::forward_boswell(&boswell, &state, &config, &http)
            .await
            .unwrap(),
        1
    );

    let s = seen.lock().unwrap();
    let feed_query = s
        .requests
        .iter()
        .rev()
        .find(|(r, _, _)| r == "GET /v1/feed")
        .map(|(_, q, _)| q.clone())
        .unwrap();
    assert_eq!(feed_query["changes"], "contradicted");
    assert_eq!(
        feed_query["exclude_self"], "true",
        "the bridge's own announcements are not echoed"
    );

    let (_, body, headers) = s
        .requests
        .iter()
        .find(|(r, _, _)| r == "POST /api/stream/emit")
        .unwrap();
    assert_eq!(headers["x-openrig-session"], bridge::STREAM_SENDER);
    assert_eq!(body["streamItemId"], "boswell-100");
    assert_eq!(body["hintUrgency"], "urgent");
    drop(s);
    assert_eq!(
        BridgeState::load(&config.state_path)
            .boswell_cursor
            .as_deref(),
        Some("100")
    );
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn the_openrig_event_stream_is_followed_from_the_last_seq() {
    let seen = Shared::default();
    let gw = gateway(seen.clone()).await;

    let rig_seen = seen.clone();
    let rig = serve(
        Router::new()
            .route(
                "/api/events",
                get(
                    |State(s): State<Shared>, h: HeaderMap, Query(q): Query<HashMap<String, String>>| async move {
                        s.lock().unwrap().requests.push(("GET /api/events".into(), json!(q), headers_of(&h)));
                        // Two frames, split oddly, then the stream ends.
                        let body = concat!(
                            "id: 6\ndata: {\"type\":\"queue.claimed\",\"qitemId\":\"q8\",",
                            "\"destinationSession\":\"impl-3@build\",\"summary\":\"Build Tokenizer\"}\n\n",
                            ": keepalive\n",
                            "id: 7\ndata: {\"type\":\"node.started\"}\n\n"
                        );
                        ([("content-type", "text/event-stream")], body)
                    },
                ),
            )
            .with_state(rig_seen),
    )
    .await;

    let boswell = Boswell::new(
        BoswellConfig {
            url: gw,
            token: "bridge-key".into(),
            namespace: None,
        },
        Duration::from_secs(5),
    )
    .unwrap();
    let dir = temp_dir("follow");
    let config = BridgeConfig {
        openrig_url: rig,
        state_path: dir.join("bridge.json"),
        poll_every: Duration::from_secs(1),
        outbound_changes: vec![],
    };
    let state = Arc::new(tokio::sync::Mutex::new(BridgeState {
        openrig_seq: Some("5".into()),
        ..BridgeState::default()
    }));

    let why = bridge::follow_openrig(&boswell, &state, &config, &reqwest::Client::new()).await;
    assert_eq!(why, "OpenRig closed the stream");

    {
        let s = seen.lock().unwrap();
        let (_, query, headers) = s
            .requests
            .iter()
            .find(|(r, _, _)| r == "GET /api/events")
            .unwrap();
        assert_eq!(query["replay"], "0", "never replay the whole log");
        assert_eq!(headers["last-event-id"], "5");
        let announced = s
            .requests
            .iter()
            .find(|(r, _, _)| r == "POST /v1/claims")
            .unwrap();
        assert_eq!(announced.1["subject"], "Build Tokenizer");
    }

    let st = state.lock().await;
    assert_eq!(
        st.openrig_seq.as_deref(),
        Some("7"),
        "every frame advances the seq"
    );
    assert!(st.announcements.contains_key("q8"));
    assert_eq!(BridgeState::load(&config.state_path), *st);
    std::fs::remove_dir_all(dir).ok();
}
