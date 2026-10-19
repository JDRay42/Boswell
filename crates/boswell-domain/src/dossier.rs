//! The dossier's claim grouping and standing (research design §5.2).
//!
//! A dossier groups the claims it gathered by `(subject, predicate)` and gives
//! each group a [`Standing`]: whether what Boswell holds there is in dispute,
//! out of date, backed by too few independent principals, or settled. Gathering
//! the claims is the transport's job; this module is the deterministic part, so
//! the same claims always read the same way.

use crate::{authenticated_principal, Claim, ClaimId};
use std::collections::{BTreeMap, BTreeSet};

/// A dossier group's state, in order of precedence: a group is the first of
/// these that holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Standing {
    /// A claim in the group has a recorded contradiction with a claim that still
    /// exists.
    Contested,
    /// Every claim in the group is past its `stale_at`.
    Stale,
    /// The group's claims span fewer independent principals than settling
    /// requires, however many authors they have.
    Thin,
    /// None of the above.
    Settled,
}

impl Standing {
    /// The wire string for this standing.
    pub fn as_str(&self) -> &'static str {
        match self {
            Standing::Contested => "contested",
            Standing::Stale => "stale",
            Standing::Thin => "thin",
            Standing::Settled => "settled",
        }
    }
}

/// A claim as the dossier sees it: the claim and who asserted it.
#[derive(Debug, Clone, PartialEq)]
pub struct DossierClaim {
    /// The claim.
    pub claim: Claim,
    /// Its distinct authors, earliest first. Empty when unattributed.
    pub authors: Vec<String>,
}

/// One `(subject, predicate)` group of a dossier.
#[derive(Debug, Clone, PartialEq)]
pub struct ClaimGroup {
    /// The subject the group's claims share.
    pub subject: String,
    /// The predicate the group's claims share.
    pub predicate: String,
    /// The group's standing.
    pub standing: Standing,
    /// The distinct independence roots behind the group, sorted.
    pub roots: Vec<String>,
    /// How many of the group's claims have no author at all.
    pub unattributed_count: usize,
    /// The object of the group's most confident claim; ties go to the newest.
    pub lead: String,
    /// The group's claims, most confident first.
    pub claims: Vec<DossierClaim>,
    /// Each recorded contradiction touching the group, as `(claim in the group,
    /// claim it contradicts)`.
    pub contradictions: Vec<(ClaimId, ClaimId)>,
}

/// Group `claims` by `(subject, predicate)` and give each group its standing.
///
/// `contradictions` are the `Contradicts` relationships the caller found among
/// live claims, in either direction. `now` is Unix seconds, compared against
/// each claim's `stale_at`. `min_roots` is how many distinct independence roots
/// a group needs to read as settled.
///
/// Groups come back most urgent first — contested, stale, thin, settled — and
/// alphabetically within a standing, so the order never depends on how the
/// claims were gathered.
pub fn group_claims(
    claims: Vec<DossierClaim>,
    contradictions: &[(ClaimId, ClaimId)],
    now: u64,
    min_roots: usize,
) -> Vec<ClaimGroup> {
    let mut by_key: BTreeMap<(String, String), Vec<DossierClaim>> = BTreeMap::new();
    for dc in claims {
        by_key
            .entry((dc.claim.subject.clone(), dc.claim.predicate.clone()))
            .or_default()
            .push(dc);
    }

    let mut groups: Vec<ClaimGroup> = by_key
        .into_iter()
        .map(|((subject, predicate), mut members)| {
            members.sort_by(|a, b| {
                midpoint(&b.claim)
                    .total_cmp(&midpoint(&a.claim))
                    .then(b.claim.created_at.cmp(&a.claim.created_at))
                    .then(a.claim.id.cmp(&b.claim.id))
            });

            let ids: BTreeSet<ClaimId> = members.iter().map(|m| m.claim.id).collect();
            let mut touching: Vec<(ClaimId, ClaimId)> = Vec::new();
            for &(a, b) in contradictions {
                if ids.contains(&a) {
                    touching.push((a, b));
                }
                if ids.contains(&b) && a != b {
                    touching.push((b, a));
                }
            }
            touching.sort();
            touching.dedup();

            let roots: BTreeSet<String> = members
                .iter()
                .flat_map(|m| m.authors.iter())
                .map(|a| authenticated_principal(a).to_string())
                .collect();
            let unattributed_count = members.iter().filter(|m| m.authors.is_empty()).count();

            let all_stale = members
                .iter()
                .all(|m| m.claim.stale_at.is_some_and(|t| t <= now));

            let standing = if !touching.is_empty() {
                Standing::Contested
            } else if all_stale {
                Standing::Stale
            } else if roots.len() < min_roots {
                Standing::Thin
            } else {
                Standing::Settled
            };

            ClaimGroup {
                subject,
                predicate,
                standing,
                roots: roots.into_iter().collect(),
                unattributed_count,
                lead: members
                    .first()
                    .map(|m| m.claim.object.clone())
                    .unwrap_or_default(),
                claims: members,
                contradictions: touching,
            }
        })
        .collect();

    // The map already ordered groups by (subject, predicate); a stable sort by
    // standing keeps that order within each standing.
    groups.sort_by_key(|g| g.standing);
    groups
}

fn midpoint(claim: &Claim) -> f64 {
    (claim.confidence.0 + claim.confidence.1) / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claim(subject: &str, predicate: &str, object: &str, conf: f64) -> Claim {
        Claim::new(
            ClaimId::new(),
            "team".into(),
            subject.into(),
            predicate.into(),
            object.into(),
            (conf, conf),
            "task".into(),
            100,
        )
    }

    fn by(claim: Claim, authors: &[&str]) -> DossierClaim {
        DossierClaim {
            claim,
            authors: authors.iter().map(|a| a.to_string()).collect(),
        }
    }

    #[test]
    fn claims_group_by_subject_and_predicate() {
        let groups = group_claims(
            vec![
                by(claim("DateParser", "provides", "ISO 8601", 0.9), &["a"]),
                by(claim("DateParser", "provides", "RFC 2822", 0.8), &["b"]),
                by(
                    claim("DateParser", "is located at", "src/date.rs", 0.9),
                    &["a"],
                ),
            ],
            &[],
            1_000,
            2,
        );
        assert_eq!(groups.len(), 2);
        let provides = groups.iter().find(|g| g.predicate == "provides").unwrap();
        assert_eq!(provides.claims.len(), 2);
        assert_eq!(provides.lead, "ISO 8601", "the most confident claim leads");
    }

    /// Five agents of one grant are five authors and one root, and one root is
    /// thin however many authors stand on it.
    #[test]
    fn agents_of_one_principal_are_one_root() {
        let groups = group_claims(
            vec![
                by(claim("X", "p", "o", 0.9), &["jd/sub:a", "jd/sub:b"]),
                by(claim("X", "p", "o", 0.9), &["jd/sub:c", "jd"]),
            ],
            &[],
            1_000,
            2,
        );
        assert_eq!(groups[0].roots, vec!["jd"]);
        assert_eq!(groups[0].standing, Standing::Thin);
    }

    #[test]
    fn two_principals_settle_a_group() {
        let groups = group_claims(
            vec![by(claim("X", "p", "o", 0.9), &["jd", "auditor/sub:x"])],
            &[],
            1_000,
            2,
        );
        assert_eq!(groups[0].roots, vec!["auditor", "jd"]);
        assert_eq!(groups[0].standing, Standing::Settled);
    }

    #[test]
    fn unattributed_claims_count_toward_no_root() {
        let groups = group_claims(
            vec![
                by(claim("X", "p", "o", 0.9), &[]),
                by(claim("X", "p", "o2", 0.9), &[]),
            ],
            &[],
            1_000,
            1,
        );
        assert_eq!(groups[0].unattributed_count, 2);
        assert!(groups[0].roots.is_empty());
        assert_eq!(groups[0].standing, Standing::Thin);
    }

    #[test]
    fn a_contradiction_contests_both_groups_it_touches() {
        let a = claim("X", "p", "yes", 0.9);
        let b = claim("Y", "q", "no", 0.9);
        let (ida, idb) = (a.id, b.id);
        let groups = group_claims(
            vec![by(a, &["jd", "auditor"]), by(b, &["jd", "auditor"])],
            &[(ida, idb)],
            1_000,
            2,
        );
        assert!(groups.iter().all(|g| g.standing == Standing::Contested));
        let x = groups.iter().find(|g| g.subject == "X").unwrap();
        assert_eq!(x.contradictions, vec![(ida, idb)]);
        let y = groups.iter().find(|g| g.subject == "Y").unwrap();
        assert_eq!(y.contradictions, vec![(idb, ida)]);
    }

    #[test]
    fn a_group_is_stale_only_when_every_claim_is() {
        let mut old = claim("X", "p", "o", 0.9);
        old.stale_at = Some(500);
        let mut fresh = claim("X", "p", "o2", 0.9);
        fresh.stale_at = Some(5_000);

        let one_fresh = group_claims(
            vec![by(old.clone(), &["jd", "a"]), by(fresh, &["jd", "a"])],
            &[],
            1_000,
            2,
        );
        assert_eq!(one_fresh[0].standing, Standing::Settled);

        let all_old = group_claims(vec![by(old, &["jd", "a"])], &[], 1_000, 2);
        assert_eq!(all_old[0].standing, Standing::Stale);
    }

    #[test]
    fn contested_outranks_stale_and_groups_sort_by_urgency() {
        let mut stale_contested = claim("B", "p", "o", 0.9);
        stale_contested.stale_at = Some(1);
        let other = claim("Z", "p", "o", 0.9);
        let (s, o) = (stale_contested.id, other.id);
        let groups = group_claims(
            vec![
                by(claim("A", "p", "o", 0.9), &["jd", "auditor"]),
                by(claim("C", "p", "o", 0.9), &["jd"]),
                by(stale_contested, &["jd"]),
                by(other, &["jd", "auditor"]),
            ],
            &[(s, o)],
            1_000,
            2,
        );
        let order: Vec<_> = groups
            .iter()
            .map(|g| (g.subject.as_str(), g.standing))
            .collect();
        assert_eq!(
            order,
            vec![
                ("B", Standing::Contested),
                ("Z", Standing::Contested),
                ("C", Standing::Thin),
                ("A", Standing::Settled),
            ]
        );
    }
}
