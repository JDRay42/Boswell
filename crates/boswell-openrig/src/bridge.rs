//! The bridge between an OpenRig daemon and Boswell (OpenRig integration
//! guide, "The bridge").
//!
//! Inbound, it turns OpenRig queue events into announcements: while a seat owns
//! a queue item, Boswell holds `(<item>, "is being built", "by <seat> …")`, and
//! a dossier on that subject shows the work is taken. When the item closes the
//! announcement is forgotten, and a finished item leaves
//! `(<item>, "was completed by", <seat>)` behind.
//!
//! Outbound, it posts the Boswell changes a team should hear about —
//! contradictions, promotions, supersessions — into OpenRig's stream, where the
//! rig's own views and notifications pick them up.
//!
//! The mapping is pure ([`inbound`], [`outbound`]); [`run`] does the I/O.

use crate::boswell::{Boswell, FeedRead};
use crate::config::agent_label;
use crate::sse;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

/// The predicate an announcement is asserted under (change feed design §6.1).
pub const ANNOUNCE: &str = "is being built";
/// The predicate a finished queue item leaves behind.
pub const COMPLETED: &str = "was completed by";
/// The identity the bridge posts to OpenRig's stream under.
pub const STREAM_SENDER: &str = "boswell@bridge";
/// The longest subject an announcement takes from a queue item's summary.
const MAX_SUBJECT: usize = 120;

/// What an OpenRig event asks Boswell to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inbound {
    /// A seat took an item on: announce it.
    Announce {
        /// The queue item.
        qitem: String,
        /// What the announcement is about.
        subject: String,
        /// The seat that owns it.
        seat: String,
    },
    /// The item is no longer being worked: forget its announcement.
    Retract {
        /// The queue item.
        qitem: String,
    },
    /// The item finished: forget the announcement and record who finished it.
    Complete {
        /// The queue item.
        qitem: String,
        /// What it was.
        subject: String,
        /// Who closed it.
        seat: String,
    },
}

/// The subject an item is announced under: its summary, else its id.
fn subject_for(summary: Option<&str>, qitem: &str) -> String {
    match summary.map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) if s.chars().count() > MAX_SUBJECT => {
            let kept: String = s.chars().take(MAX_SUBJECT - 1).collect();
            format!("{}…", kept)
        }
        Some(s) => s.to_string(),
        None => format!("OpenRig queue item {}", qitem),
    }
}

/// Map one OpenRig event (the JSON of an `/api/events` frame) to what Boswell
/// should do about it. Events that say nothing about who is working on what
/// map to `None`.
pub fn inbound(event: &Value) -> Option<Inbound> {
    let qitem = event["qitemId"].as_str()?.to_string();
    let summary = event["summary"].as_str();
    match event["type"].as_str()? {
        "queue.claimed" => Some(Inbound::Announce {
            subject: subject_for(summary, &qitem),
            seat: event["destinationSession"].as_str()?.to_string(),
            qitem,
        }),
        "queue.unclaimed" | "queue.handed_off" => Some(Inbound::Retract { qitem }),
        "queue.updated" => match event["toState"].as_str()? {
            "in-progress" => Some(Inbound::Announce {
                subject: subject_for(summary, &qitem),
                seat: event["actorSession"].as_str()?.to_string(),
                qitem,
            }),
            "done" => Some(Inbound::Complete {
                subject: subject_for(summary, &qitem),
                seat: event["actorSession"].as_str()?.to_string(),
                qitem,
            }),
            "pending" | "failed" | "denied" | "canceled" | "handed-off" => {
                Some(Inbound::Retract { qitem })
            }
            // A blocked item is still owned; its announcement stands.
            _ => None,
        },
        _ => None,
    }
}

/// Build the `POST /api/stream/emit` body for one Boswell feed event.
///
/// `streamItemId` is derived from the cursor, and OpenRig treats a repeated id
/// as the same item, so re-posting after a crash does not duplicate.
pub fn outbound(event: &Value) -> Value {
    let change = event["change"].as_str().unwrap_or("changed");
    let kind = event["kind"].as_str().unwrap_or("memory");
    let what = event["summary"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| format!("{} {}", kind, event["entity_id"].as_str().unwrap_or("")));
    let tier = event["tier"]["after"]
        .as_str()
        .or_else(|| event["tier"]["before"].as_str())
        .unwrap_or("?");
    let by = event["author"]
        .as_str()
        .map(|a| format!(" (by {})", a))
        .unwrap_or_default();
    json!({
        "streamItemId": format!("boswell-{}", event["cursor"].as_str().unwrap_or("0")),
        "body": format!("Boswell: {} {} was {} [{}]{}", kind, what, change, tier, by),
        "format": "text",
        "hintType": "memory",
        "hintUrgency": if change == "contradicted" { "urgent" } else { "routine" },
        "hintTags": ["boswell", change, kind],
    })
}

/// What the bridge remembers between runs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// The last OpenRig event seq handled, sent as `Last-Event-ID`.
    pub openrig_seq: Option<String>,
    /// The Boswell feed cursor.
    pub boswell_cursor: Option<String>,
    /// Queue item → the claim id of its live announcement.
    pub announcements: BTreeMap<String, String>,
}

impl State {
    /// Load from `path`; a missing or unreadable file is a fresh start.
    pub fn load(path: &PathBuf) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Write to `path`, through a temporary file so a crash mid-write cannot
    /// leave a torn state behind.
    pub fn save(&self, path: &PathBuf) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(tmp, path)
    }
}

/// How the bridge runs.
#[derive(Debug, Clone)]
pub struct BridgeConfig {
    /// The OpenRig daemon, e.g. `http://127.0.0.1:7433`.
    pub openrig_url: String,
    /// Where state is kept.
    pub state_path: PathBuf,
    /// Seconds between Boswell feed polls.
    pub poll_every: Duration,
    /// The changes posted to OpenRig.
    pub outbound_changes: Vec<String>,
}

/// Apply one inbound action to Boswell, updating `state`.
pub async fn apply(boswell: &Boswell, state: &mut State, action: Inbound) -> Result<(), String> {
    match action {
        Inbound::Announce {
            qitem,
            subject,
            seat,
        } => {
            if state.announcements.contains_key(&qitem) {
                return Ok(());
            }
            let id = boswell
                .assert(
                    agent_label(&seat).as_deref(),
                    &subject,
                    ANNOUNCE,
                    &format!("by {} (OpenRig queue item {})", seat, qitem),
                )
                .await
                .map_err(|e| e.to_string())?;
            state.announcements.insert(qitem, id);
        }
        Inbound::Retract { qitem } => {
            if let Some(id) = state.announcements.get(&qitem).cloned() {
                boswell.forget(&id).await.map_err(|e| e.to_string())?;
                state.announcements.remove(&qitem);
            }
        }
        Inbound::Complete {
            qitem,
            subject,
            seat,
        } => {
            if let Some(id) = state.announcements.get(&qitem).cloned() {
                boswell.forget(&id).await.map_err(|e| e.to_string())?;
                state.announcements.remove(&qitem);
            }
            boswell
                .assert(agent_label(&seat).as_deref(), &subject, COMPLETED, &seat)
                .await
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Follow OpenRig's `/api/events` from the last seq handled, applying each
/// event, until the stream ends. Returns why it ended; [`run`] reconnects.
pub async fn follow_openrig(
    boswell: &Boswell,
    state: &Arc<Mutex<State>>,
    config: &BridgeConfig,
    http: &reqwest::Client,
) -> String {
    let last = state.lock().await.openrig_seq.clone();
    // `replay=0`: without it the daemon replays its whole log. A known seq goes
    // in `Last-Event-ID`, which replays only what came after it.
    let mut req = http.get(format!("{}/api/events?replay=0", config.openrig_url));
    if let Some(seq) = &last {
        req = req.header("Last-Event-ID", seq);
    }
    let mut resp = match req.send().await {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => return format!("OpenRig answered {}", r.status()),
        Err(e) => return format!("cannot reach OpenRig: {}", e),
    };

    let mut parser = sse::Parser::default();
    loop {
        let chunk = match resp.chunk().await {
            Ok(Some(c)) => c,
            Ok(None) => return "OpenRig closed the stream".to_string(),
            Err(e) => return format!("OpenRig stream failed: {}", e),
        };
        for frame in parser.push(&chunk) {
            let Ok(event) = serde_json::from_str::<Value>(&frame.data) else {
                continue;
            };
            let mut st = state.lock().await;
            if let Some(action) = inbound(&event) {
                if let Err(e) = apply(boswell, &mut st, action).await {
                    // Not advancing past an event Boswell refused means a
                    // reconnect retries it.
                    return format!("could not apply OpenRig event: {}", e);
                }
            }
            if let Some(id) = frame.id {
                st.openrig_seq = Some(id);
            }
            if let Err(e) = st.save(&config.state_path) {
                eprintln!("boswell-openrig bridge: cannot save state: {}", e);
            }
        }
    }
}

/// Post one page of Boswell changes to OpenRig's stream.
pub async fn forward_boswell(
    boswell: &Boswell,
    state: &Arc<Mutex<State>>,
    config: &BridgeConfig,
    http: &reqwest::Client,
) -> Result<usize, String> {
    let cursor = state.lock().await.boswell_cursor.clone();
    let Some(cursor) = cursor else {
        // First run: follow from now rather than replaying history into the rig.
        if let Ok(FeedRead::Page { next, .. }) = boswell.feed(None, true, &[], true, 1).await {
            let mut st = state.lock().await;
            st.boswell_cursor = Some(next);
            st.save(&config.state_path).map_err(|e| e.to_string())?;
        }
        return Ok(0);
    };

    match boswell
        .feed(Some(&cursor), false, &config.outbound_changes, true, 100)
        .await
        .map_err(|e| e.to_string())?
    {
        FeedRead::Gone { head } => {
            let mut st = state.lock().await;
            st.boswell_cursor = Some(head);
            st.save(&config.state_path).map_err(|e| e.to_string())?;
            Ok(0)
        }
        FeedRead::Page { events, next } => {
            let mut posted = 0;
            for event in &events {
                let resp = http
                    .post(format!("{}/api/stream/emit", config.openrig_url))
                    .header("x-openrig-session", STREAM_SENDER)
                    .json(&outbound(event))
                    .send()
                    .await
                    .map_err(|e| format!("cannot reach OpenRig: {}", e))?;
                if !resp.status().is_success() {
                    return Err(format!("OpenRig refused a stream item: {}", resp.status()));
                }
                posted += 1;
                // Advance per event, so a failure part-way resumes there.
                let mut st = state.lock().await;
                st.boswell_cursor = event["cursor"].as_str().map(str::to_string);
                st.save(&config.state_path).map_err(|e| e.to_string())?;
            }
            let mut st = state.lock().await;
            st.boswell_cursor = Some(next);
            st.save(&config.state_path).map_err(|e| e.to_string())?;
            Ok(posted)
        }
    }
}

/// Run the bridge until the process is stopped: follow OpenRig's events,
/// reconnecting with backoff, and poll Boswell's feed on an interval.
pub async fn run(boswell: Boswell, config: BridgeConfig) {
    let state = Arc::new(Mutex::new(State::load(&config.state_path)));
    // No overall timeout: the events request is a long-lived stream.
    let stream_http = reqwest::Client::new();
    let post_http = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("a client with only a timeout always builds");

    let inbound_task = {
        let (boswell, state, config) = (boswell.clone(), state.clone(), config.clone());
        tokio::spawn(async move {
            let mut backoff = Duration::from_secs(1);
            loop {
                let why = follow_openrig(&boswell, &state, &config, &stream_http).await;
                eprintln!(
                    "boswell-openrig bridge: {}; reconnecting in {:?}",
                    why, backoff
                );
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(30));
            }
        })
    };

    let outbound_task = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(config.poll_every);
        loop {
            ticker.tick().await;
            if let Err(e) = forward_boswell(&boswell, &state, &config, &post_http).await {
                eprintln!("boswell-openrig bridge: {}", e);
            }
        }
    });

    let _ = tokio::join!(inbound_task, outbound_task);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_claimed_item_is_announced_under_its_summary() {
        let e = json!({ "type": "queue.claimed", "qitemId": "q1",
                        "destinationSession": "impl-2@build", "summary": "Build DateParser",
                        "claimedAt": "…" });
        assert_eq!(
            inbound(&e),
            Some(Inbound::Announce {
                qitem: "q1".into(),
                subject: "Build DateParser".into(),
                seat: "impl-2@build".into()
            })
        );
    }

    #[test]
    fn an_item_without_a_summary_is_announced_under_its_id() {
        let e = json!({ "type": "queue.claimed", "qitemId": "q1",
                        "destinationSession": "s", "summary": null });
        let Some(Inbound::Announce { subject, .. }) = inbound(&e) else {
            panic!("expected an announcement");
        };
        assert_eq!(subject, "OpenRig queue item q1");
    }

    #[test]
    fn closing_states_retract_and_done_completes() {
        let updated = |to: &str| {
            json!({ "type": "queue.updated", "qitemId": "q1", "fromState": "in-progress",
                    "toState": to, "actorSession": "impl-2@build", "summary": "Build DateParser" })
        };
        assert!(matches!(
            inbound(&updated("done")),
            Some(Inbound::Complete { .. })
        ));
        for to in ["failed", "canceled", "denied", "handed-off", "pending"] {
            assert_eq!(
                inbound(&updated(to)),
                Some(Inbound::Retract { qitem: "q1".into() })
            );
        }
        assert_eq!(
            inbound(&updated("blocked")),
            None,
            "a blocked item is still owned"
        );
        assert_eq!(
            inbound(&json!({ "type": "queue.handed_off", "qitemId": "q1" })),
            Some(Inbound::Retract { qitem: "q1".into() })
        );
    }

    #[test]
    fn unrelated_events_are_ignored() {
        assert_eq!(
            inbound(&json!({ "type": "node.started", "rigId": "r" })),
            None
        );
        assert_eq!(
            inbound(&json!({ "type": "stream.emitted", "streamItemId": "x" })),
            None
        );
    }

    #[test]
    fn a_long_summary_is_cut() {
        let long = "x".repeat(300);
        assert_eq!(subject_for(Some(&long), "q").chars().count(), MAX_SUBJECT);
    }

    #[test]
    fn a_contradiction_goes_out_urgent_and_idempotent() {
        let e = json!({ "cursor": "57", "kind": "claim", "change": "contradicted",
                        "tier": { "after": "project" },
                        "summary": "DateParser is thread safe yes", "author": "oidc:jd/sub:impl-2" });
        let body = outbound(&e);
        assert_eq!(body["streamItemId"], "boswell-57");
        assert_eq!(body["hintUrgency"], "urgent");
        assert_eq!(
            body["body"],
            "Boswell: claim DateParser is thread safe yes was contradicted [project] (by oidc:jd/sub:impl-2)"
        );
        assert_eq!(
            body["hintTags"],
            json!(["boswell", "contradicted", "claim"])
        );
    }

    #[test]
    fn state_round_trips_through_a_file() {
        let dir = std::env::temp_dir().join(format!("boswell-bridge-test-{}", std::process::id()));
        let path = dir.join("bridge.json");
        let mut s = State {
            openrig_seq: Some("12".into()),
            ..State::default()
        };
        s.announcements.insert("q1".into(), "01J".into());
        s.save(&path).unwrap();
        assert_eq!(State::load(&path), s);
        assert_eq!(State::load(&dir.join("missing.json")), State::default());
        std::fs::remove_dir_all(dir).ok();
    }
}
