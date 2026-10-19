//! Configuration, read from the environment an OpenRig seat or the bridge's
//! own process runs in. Every name is documented in
//! `docs/integrations/openrig.md`.

use std::path::PathBuf;

/// Where Boswell is and how to reach it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoswellConfig {
    /// The gateway's base URL, e.g. `http://127.0.0.1:8081` (no `/v1`).
    pub url: String,
    /// A bearer token for the gateway.
    pub token: String,
    /// Namespace to read and write; the token's own when absent.
    pub namespace: Option<String>,
}

/// The longest agent label the gateway accepts.
const MAX_LABEL: usize = 64;

impl BoswellConfig {
    /// Read `BOSWELL_URL` and the token from `BOSWELL_TOKEN`, or from the file
    /// `BOSWELL_TOKEN_FILE` names. `None` when either is missing: an
    /// unconfigured seat is not an error, it just has no Boswell.
    ///
    /// The file form exists because OpenRig stores rig-spec `env:` values in its
    /// database and shows them in tmux arguments, and a token belongs in
    /// neither.
    pub fn from_env(env: &dyn Fn(&str) -> Option<String>) -> Option<Self> {
        let url = env("BOSWELL_URL").filter(|v| !v.trim().is_empty())?;
        let token = match env("BOSWELL_TOKEN").filter(|v| !v.trim().is_empty()) {
            Some(t) => t,
            None => {
                let path = env("BOSWELL_TOKEN_FILE").filter(|v| !v.trim().is_empty())?;
                std::fs::read_to_string(path).ok()?.trim().to_string()
            }
        };
        if token.is_empty() {
            return None;
        }
        Some(Self {
            url: url.trim().trim_end_matches('/').to_string(),
            token,
            namespace: env("BOSWELL_NAMESPACE").filter(|v| !v.trim().is_empty()),
        })
    }
}

/// Turn an OpenRig session name into an `X-Boswell-Agent` label the gateway
/// accepts: `[A-Za-z0-9._:@-]`, at most 64 characters. Anything else becomes
/// `-`. Canonical session names (`pod-member@rig`) pass through unchanged.
pub fn agent_label(session: &str) -> Option<String> {
    let label: String = session
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '@' | '-') {
                c
            } else {
                '-'
            }
        })
        .take(MAX_LABEL)
        .collect();
    (!label.is_empty()).then_some(label)
}

/// A comma-separated environment list, trimmed, empties dropped.
pub fn list(value: Option<String>) -> Vec<String> {
    value
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// The directory the hooks and the bridge keep their state in:
/// `BOSWELL_OPENRIG_STATE_DIR`, else `$OPENRIG_HOME/boswell`, else
/// `$HOME/.openrig/boswell`.
pub fn state_dir(env: &dyn Fn(&str) -> Option<String>) -> PathBuf {
    if let Some(dir) = env("BOSWELL_OPENRIG_STATE_DIR").filter(|v| !v.trim().is_empty()) {
        return PathBuf::from(dir);
    }
    if let Some(home) = env("OPENRIG_HOME").filter(|v| !v.trim().is_empty()) {
        return PathBuf::from(home).join("boswell");
    }
    PathBuf::from(env("HOME").unwrap_or_else(|| ".".to_string()))
        .join(".openrig")
        .join("boswell")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k| map.get(k).cloned()
    }

    #[test]
    fn a_seat_without_a_url_or_token_has_no_boswell() {
        assert!(BoswellConfig::from_env(&env_of(&[])).is_none());
        assert!(BoswellConfig::from_env(&env_of(&[("BOSWELL_URL", "http://x")])).is_none());
    }

    #[test]
    fn the_url_loses_its_trailing_slash() {
        let c = BoswellConfig::from_env(&env_of(&[
            ("BOSWELL_URL", "http://127.0.0.1:8080/"),
            ("BOSWELL_TOKEN", "t"),
            ("BOSWELL_NAMESPACE", "team"),
        ]))
        .unwrap();
        assert_eq!(c.url, "http://127.0.0.1:8080");
        assert_eq!(c.namespace.as_deref(), Some("team"));
    }

    #[test]
    fn a_token_file_is_read_and_trimmed() {
        let dir = std::env::temp_dir().join(format!("boswell-openrig-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("token");
        std::fs::write(&path, "secret\n").unwrap();
        let c = BoswellConfig::from_env(&env_of(&[
            ("BOSWELL_URL", "http://x"),
            ("BOSWELL_TOKEN_FILE", path.to_str().unwrap()),
        ]))
        .unwrap();
        assert_eq!(c.token, "secret");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn session_names_become_valid_labels() {
        assert_eq!(agent_label("impl-2@build").as_deref(), Some("impl-2@build"));
        assert_eq!(
            agent_label("odd/name here").as_deref(),
            Some("odd-name-here")
        );
        assert_eq!(agent_label(&"x".repeat(80)).unwrap().len(), 64);
        assert_eq!(agent_label("  "), None);
    }

    #[test]
    fn state_lives_under_openrig_home() {
        let dir = state_dir(&env_of(&[("OPENRIG_HOME", "/tmp/rig")]));
        assert_eq!(dir, PathBuf::from("/tmp/rig/boswell"));
        let dir = state_dir(&env_of(&[
            ("OPENRIG_HOME", "/tmp/rig"),
            ("BOSWELL_OPENRIG_STATE_DIR", "/elsewhere"),
        ]));
        assert_eq!(dir, PathBuf::from("/elsewhere"));
    }
}
