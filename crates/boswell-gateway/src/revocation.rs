//! The revocation list: the one thing that ends a token before its expiry.
//!
//! Attenuable tokens verify offline (ADR-022). That is the property that makes
//! them worth having — a delegate's grant costs no round trip — and it is also
//! why a grant handed out by mistake cannot simply be deleted. Until it expires
//! it keeps working, unless something the gateway consults says otherwise.
//!
//! That something is a file. Each line is a **revocation identifier**: the hex
//! signature of one block of one token, as `Biscuit::revocation_identifiers`
//! reports it. A token is revoked when *any* of its blocks is listed, which is
//! what lets one line kill a whole subtree — every token attenuated from a root
//! still carries the root's authority block, so revoking the root revokes every
//! delegate derived from it.
//!
//! ```text
//! # ops: leaked laptop, 2026-09-12
//! 4b4a5b...   # root token for agent-ci
//! ```
//!
//! # Why a file
//!
//! The alternatives were the gateway config and a table in the store. Config
//! reloads badly — the list changes at incident speed and a restart drops every
//! connection. A store table would put a network round trip on the path whose
//! whole point is not having one, and would need the instance reachable to
//! authorize anything at all. A file the gateway watches changes the cost of a
//! request by one `stat` per refresh interval, and an operator revokes by
//! appending a line.
//!
//! # What it does when things go wrong
//!
//! It fails toward the *last known list*. A file that disappears, or that stops
//! being readable, leaves the entries already loaded in force and logs; a line
//! that is not hex is skipped and logged, because one typo should not discard
//! the entries around it. A file that was never readable at all starts empty,
//! so a gateway configured with a path that does not exist yet still serves.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::{Duration, Instant, SystemTime};

/// A set of revoked block identifiers, reloaded from disk as the file changes.
///
/// Cheap to consult: a read lock and a hash lookup, plus one `stat` at most
/// once per `refresh` interval. Nothing here awaits, so the lock is
/// [`std::sync::RwLock`] and is never held across a suspension point.
#[derive(Debug)]
pub struct RevocationList {
    /// `None` disables the list entirely — no file, no `stat`, nothing revoked.
    path: Option<PathBuf>,
    /// How stale the loaded set may be before the next check re-`stat`s.
    refresh: Duration,
    state: RwLock<Loaded>,
}

#[derive(Debug)]
struct Loaded {
    ids: HashSet<Vec<u8>>,
    /// When the file was last `stat`ed, regardless of whether it had changed.
    checked_at: Instant,
    /// Modification time and length together. Either alone can miss an edit —
    /// a filesystem with coarse mtime, or an in-place rewrite of equal length.
    stamp: Option<(SystemTime, u64)>,
}

impl RevocationList {
    /// Build from config. An empty `path` disables the list.
    ///
    /// Reads the file once, now, so a broken path is reported at startup rather
    /// than on the first request that would have been revoked.
    pub fn new(path: &str, refresh_secs: u64) -> Self {
        let path = path.trim();
        let list = Self {
            path: (!path.is_empty()).then(|| PathBuf::from(path)),
            refresh: Duration::from_secs(refresh_secs),
            state: RwLock::new(Loaded {
                ids: HashSet::new(),
                checked_at: Instant::now(),
                stamp: None,
            }),
        };
        if let Some(path) = &list.path {
            let (ids, stamp) = read_list(path, &HashSet::new());
            tracing::info!(
                "revocation list {}: {} identifier(s) loaded",
                path.display(),
                ids.len()
            );
            let mut state = list.state.write().unwrap();
            state.ids = ids;
            state.stamp = stamp;
        }
        list
    }

    /// Whether any of `ids` — one token's block identifiers — is revoked.
    ///
    /// Takes the whole list rather than one id because that is the semantic:
    /// a token dies with any of its ancestors.
    pub fn is_revoked(&self, ids: &[Vec<u8>]) -> bool {
        if self.path.is_none() {
            return false;
        }
        self.refresh_if_stale();
        let state = self.state.read().unwrap();
        if state.ids.is_empty() {
            return false;
        }
        ids.iter().any(|id| state.ids.contains(id))
    }

    /// How many identifiers are currently loaded. For logging and tests.
    pub fn len(&self) -> usize {
        self.state.read().unwrap().ids.len()
    }

    /// Whether the list currently revokes nothing.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Re-`stat` the file if the loaded set is older than the refresh interval,
    /// and re-read it if the file has changed since.
    fn refresh_if_stale(&self) {
        let Some(path) = &self.path else {
            return;
        };
        {
            let state = self.state.read().unwrap();
            if state.checked_at.elapsed() < self.refresh {
                return;
            }
        }

        let mut state = self.state.write().unwrap();
        // Another thread may have refreshed between the two locks.
        if state.checked_at.elapsed() < self.refresh {
            return;
        }
        state.checked_at = Instant::now();

        let stamp = stamp_of(path);
        if stamp.is_some() && stamp == state.stamp {
            return; // unchanged
        }

        let (ids, stamp) = read_list(path, &state.ids);
        if ids != state.ids {
            tracing::info!(
                "revocation list {} reloaded: {} identifier(s)",
                path.display(),
                ids.len()
            );
        }
        state.ids = ids;
        state.stamp = stamp;
    }
}

/// `(mtime, len)` for a path, or `None` if it cannot be `stat`ed.
fn stamp_of(path: &Path) -> Option<(SystemTime, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/// Read and parse the file, falling back to `previous` if it cannot be read.
fn read_list(
    path: &Path,
    previous: &HashSet<Vec<u8>>,
) -> (HashSet<Vec<u8>>, Option<(SystemTime, u64)>) {
    match std::fs::read_to_string(path) {
        Ok(contents) => (
            parse(&contents, &path.display().to_string()),
            stamp_of(path),
        ),
        Err(e) => {
            tracing::warn!(
                "revocation list {} could not be read ({}); keeping the {} identifier(s) already loaded",
                path.display(),
                e,
                previous.len()
            );
            (previous.clone(), None)
        }
    }
}

/// Parse one hex identifier per line. `#` starts a comment; blank lines are
/// ignored; a line that is not hex is skipped with a warning naming its number.
pub(crate) fn parse(contents: &str, source: &str) -> HashSet<Vec<u8>> {
    let mut ids = HashSet::new();
    for (number, line) in contents.lines().enumerate() {
        let line = match line.find('#') {
            Some(hash) => &line[..hash],
            None => line,
        }
        .trim();
        if line.is_empty() {
            continue;
        }
        match decode_hex(line) {
            Some(id) => {
                ids.insert(id);
            }
            None => tracing::warn!(
                "{}:{}: not a hex revocation identifier, ignoring this line",
                source,
                number + 1
            ),
        }
    }
    ids
}

/// Decode an even-length lowercase-or-uppercase hex string.
fn decode_hex(s: &str) -> Option<Vec<u8>> {
    if s.is_empty() || !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

/// Lowercase hex for a revocation identifier, the form the file expects.
pub fn to_hex(id: &[u8]) -> String {
    let mut out = String::with_capacity(id.len() * 2);
    for byte in id {
        out.push_str(&format!("{:02x}", byte));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A list backed by a temp file, refreshed on every check so a test never
    /// sleeps through an interval.
    fn list_from(contents: &str) -> (RevocationList, PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "boswell-revocations-{}-{:?}.txt",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::write(&path, contents).expect("write the list");
        (RevocationList::new(path.to_str().unwrap(), 0), path)
    }

    #[test]
    fn an_unconfigured_list_revokes_nothing_and_reads_no_file() {
        let list = RevocationList::new("", 15);
        assert!(!list.is_revoked(&[vec![1, 2, 3]]));
        assert!(list.is_empty());
    }

    #[test]
    fn comments_blank_lines_and_surrounding_space_are_ignored() {
        let ids = parse(
            "# a comment\n\n  0a0b  \n0c0d # trailing comment\n\t\n",
            "test",
        );
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&vec![0x0a, 0x0b]));
        assert!(ids.contains(&vec![0x0c, 0x0d]));
    }

    #[test]
    fn a_line_that_is_not_hex_is_skipped_and_the_rest_survive() {
        // The alternative — rejecting the file — would silently un-revoke
        // every id in it because of one typo.
        let ids = parse("0a0b\nnot-hex\nabc\n0c0d\n", "test");
        assert_eq!(ids.len(), 2, "the two valid lines are still loaded");
        assert!(ids.contains(&vec![0x0c, 0x0d]));
    }

    #[test]
    fn case_does_not_matter_in_the_file() {
        let ids = parse("AABB\n", "test");
        assert!(ids.contains(&vec![0xaa, 0xbb]));
    }

    #[test]
    fn a_listed_identifier_revokes_a_token_carrying_it() {
        let (list, path) = list_from("aabb\n");
        assert!(list.is_revoked(&[vec![0x11], vec![0xaa, 0xbb]]));
        assert!(!list.is_revoked(&[vec![0x11], vec![0x22]]));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn appending_to_the_file_takes_effect_without_a_restart() {
        let (list, path) = list_from("aabb\n");
        assert!(!list.is_revoked(&[vec![0xcc, 0xdd]]));

        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("reopen");
        writeln!(file, "ccdd").expect("append");
        file.sync_all().expect("flush");

        assert!(
            list.is_revoked(&[vec![0xcc, 0xdd]]),
            "a line added after startup must be honored"
        );
        assert!(
            list.is_revoked(&[vec![0xaa, 0xbb]]),
            "and the old one stays"
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_file_that_disappears_leaves_its_entries_in_force() {
        let (list, path) = list_from("aabb\n");
        std::fs::remove_file(&path).expect("remove");
        assert!(
            list.is_revoked(&[vec![0xaa, 0xbb]]),
            "an unreadable list fails toward the last one it read"
        );
    }

    #[test]
    fn a_path_that_does_not_exist_yet_starts_empty_rather_than_failing() {
        let path = std::env::temp_dir().join("boswell-revocations-absent.txt");
        let _ = std::fs::remove_file(&path);
        let list = RevocationList::new(path.to_str().unwrap(), 0);
        assert!(list.is_empty());
        assert!(!list.is_revoked(&[vec![0xaa]]));
    }

    #[test]
    fn to_hex_round_trips_through_the_parser() {
        let id = vec![0x00, 0x0f, 0xf0, 0xff];
        let ids = parse(&to_hex(&id), "test");
        assert!(ids.contains(&id));
    }
}
