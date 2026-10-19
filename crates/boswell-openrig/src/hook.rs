//! The hook commands an OpenRig seat runs (change feed design §5.2): a width
//! dossier at `SessionStart`, a digest of what changed at `UserPromptSubmit`.
//!
//! A hook is on the prompt path, so it never fails the prompt: every error goes
//! to stderr and the hook prints nothing.

use crate::boswell::{Boswell, FeedRead};
use crate::config::{agent_label, list, state_dir, BoswellConfig};
use crate::digest;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::Duration;

/// How long a hook waits on Boswell before giving up.
pub const HOOK_TIMEOUT: Duration = Duration::from_secs(3);

/// Events read per prompt; the digest shows the first [`DIGEST_LINES`].
const FEED_PAGE: u32 = 200;
/// Lines in a digest.
pub const DIGEST_LINES: usize = 20;

/// Which hook is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookEvent {
    /// A session started, resumed or came back from compaction.
    SessionStart,
    /// The user (or the rig) submitted a prompt.
    UserPromptSubmit,
}

impl HookEvent {
    /// Parse the command-line name.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "session-start" => Some(HookEvent::SessionStart),
            "user-prompt-submit" => Some(HookEvent::UserPromptSubmit),
            _ => None,
        }
    }

    fn name(&self) -> &'static str {
        match self {
            HookEvent::SessionStart => "SessionStart",
            HookEvent::UserPromptSubmit => "UserPromptSubmit",
        }
    }
}

/// The context a seat starts with, before any dossier.
pub const PREAMBLE: &str = "## Boswell: shared memory for this team\n\
Before building or researching something, check whether it already exists or is already \
being worked: `boswell_dossier` with a subject or topic (add view=depth to go deeper). \
When you finish a component, record what it is, where it lives and how to use it, so the \
next agent finds it instead of rebuilding it.\n";

/// Run one hook. `stdin` is the hook payload the harness passed. Returns the
/// JSON to print, or `None` to print nothing.
pub async fn run(
    event: HookEvent,
    stdin: &Value,
    env: &dyn Fn(&str) -> Option<String>,
) -> Option<Value> {
    let config = BoswellConfig::from_env(env)?;
    let boswell = match Boswell::new(config, HOOK_TIMEOUT) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("boswell-openrig: {}", e);
            return None;
        }
    };
    let session = env("OPENRIG_SESSION_NAME")
        .or_else(|| stdin["session_id"].as_str().map(str::to_string))
        .unwrap_or_else(|| "default".to_string());
    let label = agent_label(&session);
    let cursor_file = cursor_path(env, &session);

    let context = match event {
        HookEvent::SessionStart => session_start(&boswell, stdin, env, &cursor_file).await,
        HookEvent::UserPromptSubmit => {
            prompt_submit(&boswell, label.as_deref(), &cursor_file).await
        }
    }?;

    Some(json!({
        "hookSpecificOutput": {
            "hookEventName": event.name(),
            "additionalContext": context,
        }
    }))
}

async fn session_start(
    boswell: &Boswell,
    stdin: &Value,
    env: &dyn Fn(&str) -> Option<String>,
    cursor_file: &PathBuf,
) -> Option<String> {
    // A fresh session follows from now. A resumed or compacted one keeps its
    // cursor, so the next prompt reports what happened while it was away.
    let fresh = !matches!(stdin["source"].as_str(), Some("resume") | Some("compact"));
    if fresh || read_cursor(cursor_file).is_none() {
        match boswell.feed(None, true, &[], false, 1).await {
            Ok(FeedRead::Page { next, .. }) => write_cursor(cursor_file, &next),
            Ok(FeedRead::Gone { head }) => write_cursor(cursor_file, &head),
            Err(e) => eprintln!("boswell-openrig: could not read the feed head: {}", e),
        }
    }

    let mut text = PREAMBLE.to_string();
    for subject in list(env("BOSWELL_SUBJECTS")) {
        match boswell.dossier_width(Some(&subject), None).await {
            Ok(d) => {
                if let Some(section) = digest::dossier(&subject, &d) {
                    text.push('\n');
                    text.push_str(&section);
                }
            }
            Err(e) => eprintln!("boswell-openrig: dossier for '{}' failed: {}", subject, e),
        }
    }
    if let Some(topic) = env("BOSWELL_TOPIC").filter(|t| !t.trim().is_empty()) {
        match boswell.dossier_width(None, Some(&topic)).await {
            Ok(d) => {
                if let Some(section) = digest::dossier(&topic, &d) {
                    text.push('\n');
                    text.push_str(&section);
                }
            }
            Err(e) => eprintln!(
                "boswell-openrig: dossier for topic '{}' failed: {}",
                topic, e
            ),
        }
    }
    Some(text)
}

async fn prompt_submit(
    boswell: &Boswell,
    label: Option<&str>,
    cursor_file: &PathBuf,
) -> Option<String> {
    let Some(cursor) = read_cursor(cursor_file) else {
        // No SessionStart ran (or it failed): start following from now.
        if let Ok(FeedRead::Page { next, .. }) = boswell.feed(None, true, &[], false, 1).await {
            write_cursor(cursor_file, &next);
        }
        return None;
    };

    match boswell
        .feed(Some(&cursor), false, &[], false, FEED_PAGE)
        .await
    {
        Ok(FeedRead::Page { events, next }) => {
            write_cursor(cursor_file, &next);
            // This seat's own writes are not news to it. Its teammates' are,
            // even when they share its key, so only its own label is dropped.
            let own = label.map(|l| format!("/sub:{}", l));
            let events: Vec<Value> = events
                .into_iter()
                .filter(|e| match (&own, e["author"].as_str()) {
                    (Some(own), Some(author)) => !author.ends_with(own.as_str()),
                    _ => true,
                })
                .collect();
            digest::feed(&events, DIGEST_LINES).map(|d| format!("## Boswell\n{}", d))
        }
        Ok(FeedRead::Gone { head }) => {
            write_cursor(cursor_file, &head);
            Some(
                "## Boswell\nShared memory changed more than this session could follow while \
                 it was away. Read `boswell_dossier` for the subjects you are working on before \
                 relying on what you remember of them.\n"
                    .to_string(),
            )
        }
        Err(e) => {
            eprintln!("boswell-openrig: feed read failed: {}", e);
            None
        }
    }
}

fn cursor_path(env: &dyn Fn(&str) -> Option<String>, session: &str) -> PathBuf {
    let name = agent_label(session).unwrap_or_else(|| "default".to_string());
    state_dir(env)
        .join("cursors")
        .join(format!("{}.cursor", name))
}

fn read_cursor(path: &PathBuf) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn write_cursor(path: &PathBuf, cursor: &str) {
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("boswell-openrig: cannot create {}: {}", parent.display(), e);
            return;
        }
    }
    if let Err(e) = std::fs::write(path, cursor) {
        eprintln!("boswell-openrig: cannot write {}: {}", path.display(), e);
    }
}
