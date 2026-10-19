//! The change feed's vocabulary (change feed design §3, ADR-024).
//!
//! Every change to a memory object that a reader would act on appends one
//! [`ChangeEvent`] to an append-only log, in the same transaction as the change.
//! The feed is a filtered read of that log from a cursor the client holds.

use crate::{authenticated_principal, Tier};

/// What kind of memory object an event is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EntityKind {
    /// A claim.
    Claim,
    /// A procedure.
    Procedure,
    /// A goal.
    Goal,
}

impl EntityKind {
    /// The wire and storage string.
    pub fn as_str(&self) -> &'static str {
        match self {
            EntityKind::Claim => "claim",
            EntityKind::Procedure => "procedure",
            EntityKind::Goal => "goal",
        }
    }

    /// Parse the wire string.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "claim" => Some(EntityKind::Claim),
            "procedure" => Some(EntityKind::Procedure),
            "goal" => Some(EntityKind::Goal),
            _ => None,
        }
    }
}

/// What happened to the object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Change {
    /// It was written for the first time.
    Created,
    /// It moved up a tier.
    Climbed,
    /// It moved down a tier.
    Fell,
    /// A contradiction was recorded against it.
    Contradicted,
    /// A newer version replaced it (procedures).
    Superseded,
    /// It was deleted, by Forget or by collection.
    Forgotten,
}

impl Change {
    /// The wire and storage string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Change::Created => "created",
            Change::Climbed => "climbed",
            Change::Fell => "fell",
            Change::Contradicted => "contradicted",
            Change::Superseded => "superseded",
            Change::Forgotten => "forgotten",
        }
    }

    /// Parse the wire string.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "created" => Some(Change::Created),
            "climbed" => Some(Change::Climbed),
            "fell" => Some(Change::Fell),
            "contradicted" => Some(Change::Contradicted),
            "superseded" => Some(Change::Superseded),
            "forgotten" => Some(Change::Forgotten),
            _ => None,
        }
    }

    /// The change a tier move amounts to, or `None` if the tier did not move.
    pub fn for_tier_move(before: Tier, after: Tier) -> Option<Self> {
        match after.rank().cmp(&before.rank()) {
            std::cmp::Ordering::Greater => Some(Change::Climbed),
            std::cmp::Ordering::Less => Some(Change::Fell),
            std::cmp::Ordering::Equal => None,
        }
    }
}

/// One entry in the change log.
///
/// A reference, never a copy: the reader fetches the object if it wants it.
/// Once the object is forgotten, `subject` and `summary` are stripped from
/// every event about it, so the log never keeps what Forget removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeEvent {
    /// Position in the log, and the cursor a reader resumes from. Strictly
    /// increasing and never reused.
    pub seq: u64,
    /// When the change happened (Unix ms).
    pub at: u64,
    /// The object's namespace.
    pub namespace: String,
    /// The object's kind.
    pub kind: EntityKind,
    /// The object's id.
    pub entity_id: String,
    /// What happened.
    pub change: Change,
    /// The tier before the change, where there was one.
    pub tier_before: Option<Tier>,
    /// The tier after the change, where there is one.
    pub tier_after: Option<Tier>,
    /// A claim's subject. Absent for other kinds, and once forgotten.
    pub subject: Option<String>,
    /// One line describing the object. Absent once forgotten.
    pub summary: Option<String>,
    /// Who made the change, where the store knows. Background verdicts carry none.
    pub author: Option<String>,
}

impl ChangeEvent {
    /// The independence root of the event's author, if it has one.
    pub fn independence_root(&self) -> Option<&str> {
        self.author.as_deref().map(authenticated_principal)
    }

    /// The tier the event is judged at for `min_tier`: the tier after the
    /// change, or before it for a change that leaves no tier (a forget).
    pub fn effective_tier(&self) -> Option<Tier> {
        self.tier_after.or(self.tier_before)
    }
}

/// Which events a feed read returns.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FeedFilter {
    /// Confine to this namespace and its children (`:` or `/`). `None` is
    /// unrestricted.
    pub namespace: Option<String>,
    /// Only these kinds; empty means all.
    pub kinds: Vec<EntityKind>,
    /// Only these changes; empty means all.
    pub changes: Vec<Change>,
    /// Only events about this exact subject.
    pub subject: Option<String>,
    /// Leave out events below this tier.
    pub min_tier: Option<Tier>,
    /// Leave out events whose author's independence root is this one: the
    /// caller's own writes.
    pub exclude_root: Option<String>,
    /// Page size.
    pub limit: usize,
}

impl FeedFilter {
    /// Whether `event` passes this filter.
    pub fn admits(&self, event: &ChangeEvent) -> bool {
        if !namespace_within(self.namespace.as_deref(), &event.namespace) {
            return false;
        }
        if !self.kinds.is_empty() && !self.kinds.contains(&event.kind) {
            return false;
        }
        if !self.changes.is_empty() && !self.changes.contains(&event.change) {
            return false;
        }
        if let Some(subject) = &self.subject {
            if event.subject.as_deref() != Some(subject.as_str()) {
                return false;
            }
        }
        if let Some(min) = self.min_tier {
            if event
                .effective_tier()
                .is_some_and(|t| t.rank() < min.rank())
            {
                return false;
            }
        }
        if let (Some(excluded), Some(root)) = (&self.exclude_root, event.independence_root()) {
            if root == authenticated_principal(excluded) {
                return false;
            }
        }
        true
    }
}

/// One page of the feed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FeedPage {
    /// The events after the cursor that passed the filter, oldest first.
    pub events: Vec<ChangeEvent>,
    /// The cursor to send next time. Unchanged when nothing new was scanned.
    pub next: u64,
    /// True when events after the requested cursor have already been pruned,
    /// so this page cannot be complete. The reader re-syncs from a dossier and
    /// continues from `next`.
    pub gone: bool,
}

/// Whether `namespace` lies within `scope`: equal to it, or a child of it under
/// either separator in use (`:` at the gateway, `/` in ADR-006). A raw prefix
/// match would put `teamwork` inside `team`.
pub fn namespace_within(scope: Option<&str>, namespace: &str) -> bool {
    match scope {
        None => true,
        Some(scope) => match namespace.strip_prefix(scope) {
            Some(rest) => rest.is_empty() || rest.starts_with(':') || rest.starts_with('/'),
            None => false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(
        namespace: &str,
        change: Change,
        tier: Option<Tier>,
        author: Option<&str>,
    ) -> ChangeEvent {
        ChangeEvent {
            seq: 1,
            at: 0,
            namespace: namespace.into(),
            kind: EntityKind::Claim,
            entity_id: "c".into(),
            change,
            tier_before: None,
            tier_after: tier,
            subject: Some("X".into()),
            summary: Some("X p o".into()),
            author: author.map(str::to_string),
        }
    }

    #[test]
    fn a_tier_move_names_its_direction() {
        assert_eq!(
            Change::for_tier_move(Tier::Task, Tier::Project),
            Some(Change::Climbed)
        );
        assert_eq!(
            Change::for_tier_move(Tier::Project, Tier::Task),
            Some(Change::Fell)
        );
        assert_eq!(Change::for_tier_move(Tier::Task, Tier::Task), None);
    }

    #[test]
    fn the_namespace_filter_is_not_a_raw_prefix() {
        let f = FeedFilter {
            namespace: Some("team".into()),
            ..FeedFilter::default()
        };
        assert!(f.admits(&event("team", Change::Created, None, None)));
        assert!(f.admits(&event("team:sub", Change::Created, None, None)));
        assert!(!f.admits(&event("teamwork", Change::Created, None, None)));
    }

    /// The caller's own agents, under any label, are the caller.
    #[test]
    fn excluding_self_drops_every_agent_of_the_principal() {
        let f = FeedFilter {
            exclude_root: Some("jd".into()),
            ..FeedFilter::default()
        };
        assert!(!f.admits(&event("t", Change::Created, None, Some("jd/sub:a"))));
        assert!(!f.admits(&event("t", Change::Created, None, Some("jd"))));
        assert!(f.admits(&event("t", Change::Created, None, Some("auditor"))));
        assert!(f.admits(&event("t", Change::Created, None, None)));
    }

    #[test]
    fn min_tier_judges_a_forget_by_the_tier_it_left() {
        let f = FeedFilter {
            min_tier: Some(Tier::Task),
            ..FeedFilter::default()
        };
        assert!(!f.admits(&event("t", Change::Created, Some(Tier::Ephemeral), None)));
        let mut forgotten = event("t", Change::Forgotten, None, None);
        forgotten.tier_before = Some(Tier::Project);
        assert!(f.admits(&forgotten));
    }

    #[test]
    fn kinds_changes_and_subject_narrow_the_feed() {
        let f = FeedFilter {
            kinds: vec![EntityKind::Procedure],
            ..FeedFilter::default()
        };
        assert!(!f.admits(&event("t", Change::Created, None, None)));

        let f = FeedFilter {
            changes: vec![Change::Contradicted],
            subject: Some("X".into()),
            ..FeedFilter::default()
        };
        assert!(f.admits(&event("t", Change::Contradicted, None, None)));
        assert!(!f.admits(&event("t", Change::Created, None, None)));
    }
}
