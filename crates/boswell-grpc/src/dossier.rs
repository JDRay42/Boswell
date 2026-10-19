//! The `Dossier` RPC (research design §5): everything held about a subject, in
//! one deterministic read.
//!
//! Gathering lives here, over the store traits; grouping and standing live in
//! [`boswell_domain::dossier`], so the same claims always read the same way
//! whichever transport gathered them.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use boswell_domain::traits::{ClaimQuery, GoalStore, ProcedureStore};
use boswell_domain::{
    group_claims, namespace_within, Claim, ClaimId, DossierClaim, GoalQuery, ProcedureQuery,
    RelationshipType, TraversalContext,
};
use tonic::Status;

use crate::conversions::claim_to_proto;
use crate::proto::{
    DossierAnchor, DossierClaim as ProtoDossierClaim, DossierContradiction, DossierGoal,
    DossierGroup, DossierProcedure, DossierRequest, DossierResponse,
};

/// Claims, procedures and goals returned per kind when the request names no limit.
pub const DEFAULT_LIMIT_PER_KIND: usize = 50;
/// The most a request may ask for per kind.
pub const MAX_LIMIT_PER_KIND: usize = 500;
/// Relationship hops followed from anchored claims when the request names none.
pub const DEFAULT_HOPS: u32 = 1;
/// The most hops a request may ask for.
pub const MAX_HOPS: u32 = 3;
/// Distinct independence roots a claim group needs to read as settled.
pub const MIN_ROOTS_SETTLED: usize = 2;

const KINDS: [&str; 4] = ["claim", "procedure", "goal", "question"];

/// Build a dossier from `store`. `now_ms` is Unix milliseconds.
pub fn build<S>(store: &S, req: DossierRequest, now_ms: u64) -> Result<DossierResponse, Status>
where
    S: ProcedureStore + GoalStore,
    S::Error: std::fmt::Debug,
{
    let namespace = req.namespace.filter(|n| !n.trim().is_empty() && n != "*");
    let subject = req.subject.filter(|s| !s.trim().is_empty());
    let topic = req.topic.filter(|t| !t.trim().is_empty());
    let predicate = req.predicate.filter(|p| !p.trim().is_empty());
    if subject.is_none() && topic.is_none() {
        return Err(Status::invalid_argument(
            "a dossier needs a subject, a topic, or both",
        ));
    }

    let mut kinds: BTreeSet<&str> = BTreeSet::new();
    for k in &req.kinds {
        match KINDS.iter().find(|known| **known == k.trim()) {
            Some(known) => {
                kinds.insert(known);
            }
            None => {
                return Err(Status::invalid_argument(format!(
                    "unknown kind '{}'; expected claim, procedure, goal or question",
                    k
                )))
            }
        }
    }
    if kinds.is_empty() {
        kinds.extend(KINDS);
    }

    let limit = match req.limit_per_kind {
        0 => DEFAULT_LIMIT_PER_KIND,
        n => (n as usize).min(MAX_LIMIT_PER_KIND),
    };
    let hops = req.hops.unwrap_or(DEFAULT_HOPS).min(MAX_HOPS);

    let mut response = DossierResponse {
        // Procedure and goal retrieval match intent by substring until semantic
        // intent match ships (roadmap, "Procedural memory").
        intent_match: "substring".to_string(),
        ..DossierResponse::default()
    };
    // The Question store does not exist yet (ADR-023).
    if kinds.contains("question") {
        response.unavailable.push("question".to_string());
    }

    let in_namespace = |claim: &Claim| namespace_within(namespace.as_deref(), &claim.namespace);

    // ---- Anchors ----
    let mut anchors: BTreeMap<String, DossierAnchor> = BTreeMap::new();
    if let Some(s) = &subject {
        anchors.insert(
            s.clone(),
            DossierAnchor {
                subject: s.clone(),
                matched_by: "exact".to_string(),
                score: None,
            },
        );
    }
    if let Some(t) = &topic {
        if store.supports_semantic_search() {
            let hits = store
                .semantic_search(t, limit * 4, req.min_similarity.clamp(0.0, 1.0) as f32)
                .map_err(|e| store_err("Semantic search failed", e))?;
            for (claim, score) in hits.into_iter().filter(|(c, _)| in_namespace(c)) {
                let score = score as f64;
                anchors
                    .entry(claim.subject.clone())
                    .and_modify(|a| {
                        if a.matched_by == "semantic" && a.score.is_some_and(|s| s < score) {
                            a.score = Some(score);
                        }
                    })
                    .or_insert(DossierAnchor {
                        subject: claim.subject,
                        matched_by: "semantic".to_string(),
                        score: Some(score),
                    });
            }
        } else if subject.is_none() {
            return Err(Status::failed_precondition(
                "a topic needs semantic search, which is not enabled on this instance; \
                 anchor on a subject instead",
            ));
        } else {
            response.unavailable.push("semantic_anchor".to_string());
        }
    }
    response.anchors = anchors.values().cloned().collect();

    // ---- Claims ----
    if kinds.contains("claim") {
        let mut claims: BTreeMap<ClaimId, Claim> = BTreeMap::new();
        let mut truncated = false;

        for anchor in anchors.keys() {
            let found = store
                .query_claims(&ClaimQuery {
                    namespace: namespace.clone(),
                    subject: Some(anchor.clone()),
                    predicate: predicate.clone(),
                    limit: Some(limit + 1),
                    ..ClaimQuery::default()
                })
                .map_err(|e| store_err("Claim query failed", e))?;
            for claim in found.into_iter().filter(|c| in_namespace(c)) {
                if claims.len() >= limit {
                    truncated = true;
                    break;
                }
                claims.insert(claim.id, claim);
            }
        }

        // Follow relationships outward. A hop that leaves the namespace, or the
        // requested predicate, stops there.
        let mut frontier: Vec<ClaimId> = claims.keys().copied().collect();
        for _ in 0..hops {
            let mut next = Vec::new();
            for id in frontier {
                for rel in store
                    .get_relationships(id)
                    .map_err(|e| store_err("Relationship lookup failed", e))?
                {
                    let other = if rel.from_claim == id {
                        rel.to_claim
                    } else {
                        rel.from_claim
                    };
                    if claims.contains_key(&other) {
                        continue;
                    }
                    if claims.len() >= limit {
                        truncated = true;
                        continue;
                    }
                    if let Some(claim) = store
                        .get_claim(other)
                        .map_err(|e| store_err("Claim lookup failed", e))?
                    {
                        let predicate_ok = predicate.as_ref().is_none_or(|p| claim.predicate == *p);
                        if in_namespace(&claim) && predicate_ok {
                            next.push(other);
                            claims.insert(other, claim);
                        }
                    }
                }
            }
            frontier = next;
        }

        // Contradictions among live claims the caller may see. One whose other
        // end lies outside the namespace is left out, so a dossier never names
        // a claim the caller could not fetch.
        let mut contradictions: BTreeSet<(ClaimId, ClaimId)> = BTreeSet::new();
        let mut visible: HashMap<ClaimId, bool> = HashMap::new();
        for &id in claims.keys() {
            for rel in store
                .get_relationships(id)
                .map_err(|e| store_err("Relationship lookup failed", e))?
            {
                if rel.relationship_type != RelationshipType::Contradicts {
                    continue;
                }
                let other = if rel.from_claim == id {
                    rel.to_claim
                } else {
                    rel.from_claim
                };
                let seen = match visible.get(&other) {
                    Some(v) => *v,
                    None => {
                        let v = claims.contains_key(&other)
                            || store
                                .get_claim(other)
                                .map_err(|e| store_err("Claim lookup failed", e))?
                                .is_some_and(|c| in_namespace(&c));
                        visible.insert(other, v);
                        v
                    }
                };
                if seen {
                    contradictions.insert((rel.from_claim, rel.to_claim));
                }
            }
        }

        let mut dossier_claims = Vec::with_capacity(claims.len());
        for (id, claim) in claims {
            let authors = store
                .claim_authors(id)
                .map_err(|e| store_err("Author lookup failed", e))?;
            dossier_claims.push(DossierClaim { claim, authors });
        }
        let contradictions: Vec<_> = contradictions.into_iter().collect();

        for group in group_claims(
            dossier_claims,
            &contradictions,
            now_ms / 1000,
            MIN_ROOTS_SETTLED,
        ) {
            let claim_count = group.claims.len() as u32;
            let (claims, contradictions) = if req.depth {
                (
                    group
                        .claims
                        .into_iter()
                        .map(|dc| ProtoDossierClaim {
                            stale_at: dc.claim.stale_at,
                            claim: Some(claim_to_proto(dc.claim)),
                            authors: dc.authors,
                        })
                        .collect(),
                    group
                        .contradictions
                        .into_iter()
                        .map(|(mine, theirs)| DossierContradiction {
                            claim_id: mine.to_string(),
                            against: theirs.to_string(),
                        })
                        .collect(),
                )
            } else {
                (Vec::new(), Vec::new())
            };
            response.groups.push(DossierGroup {
                subject: group.subject,
                predicate: group.predicate,
                standing: group.standing.as_str().to_string(),
                roots: group.roots,
                claim_count,
                unattributed_count: group.unattributed_count as u32,
                lead: group.lead,
                claims,
                contradictions,
            });
        }
        if truncated {
            response.truncated.push("claim".to_string());
        }
    }

    // Procedures and goals match their intent against the topic, else the subject.
    let intent = topic.clone().or(subject.clone());

    // ---- Procedures ----
    if kinds.contains("procedure") {
        if store.supports_procedures() {
            let mut found = store
                .query_procedures(
                    &ProcedureQuery {
                        namespace: namespace.clone(),
                        intent_contains: intent.clone(),
                        limit: Some(limit + 1),
                        ..ProcedureQuery::default()
                    },
                    now_ms,
                )
                .map_err(|e| store_err("Procedure query failed", e))?;
            found.retain(|p| namespace_within(namespace.as_deref(), &p.namespace));
            if found.len() > limit {
                found.truncate(limit);
                response.truncated.push("procedure".to_string());
            }
            response.procedures = found
                .iter()
                .map(|p| DossierProcedure {
                    id: p.id.to_string(),
                    name: p.name.clone(),
                    version: p.version,
                    intent: p.intent.clone(),
                    effectiveness: p.effectiveness(now_ms),
                    success_count: p.success_count,
                    failure_count: p.failure_count,
                    unknown_count: p.unknown_count,
                    tier: p.tier.as_str().to_string(),
                })
                .collect();
        } else {
            response.unavailable.push("procedure".to_string());
        }
    }

    // ---- Goals ----
    if kinds.contains("goal") {
        if store.supports_goals() {
            let mut found = store
                .query_goals(&GoalQuery {
                    namespace: namespace.clone(),
                    intent_contains: intent,
                    limit: Some(limit + 1),
                })
                .map_err(|e| store_err("Goal query failed", e))?;
            found.retain(|g| namespace_within(namespace.as_deref(), &g.namespace));
            if found.len() > limit {
                found.truncate(limit);
                response.truncated.push("goal".to_string());
            }
            for goal in found {
                let surface = store
                    .expand(goal.id, &TraversalContext::default(), now_ms)
                    .map_err(|e| store_err("Goal expansion failed", e))?;
                response.goals.push(DossierGoal {
                    id: goal.id.to_string(),
                    name: goal.name,
                    intent: goal.intent,
                    accomplish_children: surface.candidates.len() as u32,
                    best_child_effectiveness: surface
                        .candidates
                        .iter()
                        .map(|c| c.effectiveness)
                        .reduce(f64::max),
                });
            }
        } else {
            response.unavailable.push("goal".to_string());
        }
    }

    Ok(response)
}

fn store_err<E: std::fmt::Debug>(context: &'static str, err: E) -> Status {
    tracing::error!(context, error = ?err, "store call failed");
    Status::internal(format!("{}: {:?}", context, err))
}

#[cfg(test)]
mod tests {
    use super::*;
    use boswell_domain::traits::ClaimStore;
    use boswell_domain::{
        BodyFormat, Goal, GoalId, Procedure, ProcedureId, ProcedureSource, Relationship, Tier,
    };
    use boswell_store::SqliteStore;

    const NOW_MS: u64 = 1_000_000_000;

    fn store() -> SqliteStore {
        SqliteStore::new(":memory:", false, 0).unwrap()
    }

    fn put(
        store: &mut SqliteStore,
        ns: &str,
        subject: &str,
        predicate: &str,
        object: &str,
        authors: &[&str],
    ) -> ClaimId {
        let id = store
            .assert_claim(Claim::new(
                ClaimId::new(),
                ns.into(),
                subject.into(),
                predicate.into(),
                object.into(),
                (0.8, 0.9),
                "task".into(),
                NOW_MS / 1000,
            ))
            .unwrap();
        for a in authors {
            store.stamp_claim(id, a, NOW_MS / 1000).unwrap();
        }
        id
    }

    fn relate(store: &mut SqliteStore, from: ClaimId, to: ClaimId, rt: RelationshipType) {
        store
            .add_relationship(Relationship::new(from, to, rt, 1.0, NOW_MS / 1000))
            .unwrap();
    }

    fn request(subject: &str) -> DossierRequest {
        DossierRequest {
            namespace: Some("team".into()),
            subject: Some(subject.into()),
            ..DossierRequest::default()
        }
    }

    fn group<'a>(r: &'a DossierResponse, subject: &str, predicate: &str) -> &'a DossierGroup {
        r.groups
            .iter()
            .find(|g| g.subject == subject && g.predicate == predicate)
            .unwrap_or_else(|| panic!("no group {} / {}", subject, predicate))
    }

    /// The width view is a mile wide and an inch deep: every group, its
    /// standing and counts, and none of its claims.
    #[test]
    fn the_width_view_carries_counts_and_no_claims() {
        let mut s = store();
        put(
            &mut s,
            "team",
            "DateParser",
            "provides",
            "ISO 8601",
            &["jd/sub:a"],
        );
        put(
            &mut s,
            "team",
            "DateParser",
            "provides",
            "RFC 2822",
            &["auditor"],
        );
        put(
            &mut s,
            "team",
            "DateParser",
            "is located at",
            "src/date.rs",
            &["jd/sub:a", "jd/sub:b"],
        );

        let r = build(&s, request("DateParser"), NOW_MS).unwrap();

        assert_eq!(r.anchors.len(), 1);
        assert_eq!(r.anchors[0].matched_by, "exact");
        let provides = group(&r, "DateParser", "provides");
        assert_eq!(provides.claim_count, 2);
        assert_eq!(provides.standing, "settled");
        assert!(provides.claims.is_empty(), "width carries no claims");

        // Two agents of one principal are one root, so this group is thin.
        let located = group(&r, "DateParser", "is located at");
        assert_eq!(located.roots, vec!["jd"]);
        assert_eq!(located.standing, "thin");
        assert_eq!(located.lead, "src/date.rs");
    }

    #[test]
    fn the_depth_view_carries_claims_with_their_authors() {
        let mut s = store();
        put(
            &mut s,
            "team",
            "DateParser",
            "provides",
            "ISO 8601",
            &["jd/sub:a", "auditor"],
        );

        let mut req = request("DateParser");
        req.depth = true;
        let r = build(&s, req, NOW_MS).unwrap();

        let g = group(&r, "DateParser", "provides");
        assert_eq!(g.claims.len(), 1);
        // Stamped in the same second, so they come back ordered by name.
        assert_eq!(g.claims[0].authors, vec!["auditor", "jd/sub:a"]);
        assert_eq!(g.claims[0].claim.as_ref().unwrap().object, "ISO 8601");
    }

    #[test]
    fn a_contradiction_contests_the_group_and_is_listed_in_depth() {
        let mut s = store();
        let yes = put(
            &mut s,
            "team",
            "DateParser",
            "is thread safe",
            "yes",
            &["jd", "auditor"],
        );
        let no = put(
            &mut s,
            "team",
            "DateParser",
            "is thread safe",
            "no",
            &["jd", "auditor"],
        );
        relate(&mut s, yes, no, RelationshipType::Contradicts);

        let mut req = request("DateParser");
        req.depth = true;
        let r = build(&s, req, NOW_MS).unwrap();

        let g = group(&r, "DateParser", "is thread safe");
        assert_eq!(g.standing, "contested");
        assert_eq!(
            g.contradictions.len(),
            2,
            "each end of the edge is in the group"
        );
    }

    #[test]
    fn hops_pull_in_related_claims_and_zero_hops_does_not() {
        let mut s = store();
        let a = put(&mut s, "team", "DateParser", "uses", "chrono", &["jd"]);
        let b = put(&mut s, "team", "chrono", "is licensed", "MIT", &["jd"]);
        relate(&mut s, a, b, RelationshipType::References);

        let r = build(&s, request("DateParser"), NOW_MS).unwrap();
        group(&r, "chrono", "is licensed");

        let mut req = request("DateParser");
        req.hops = Some(0);
        let r = build(&s, req, NOW_MS).unwrap();
        assert!(r.groups.iter().all(|g| g.subject == "DateParser"));
    }

    /// A dossier never names a claim its caller could not fetch: a hop stops at
    /// the namespace edge, and so does a contradiction.
    #[test]
    fn nothing_crosses_the_namespace_edge() {
        let mut s = store();
        let ours = put(
            &mut s,
            "team",
            "DateParser",
            "is thread safe",
            "yes",
            &["jd"],
        );
        let theirs = put(
            &mut s,
            "other",
            "DateParser",
            "is thread safe",
            "no",
            &["x"],
        );
        relate(&mut s, ours, theirs, RelationshipType::Contradicts);

        let mut req = request("DateParser");
        req.depth = true;
        let r = build(&s, req, NOW_MS).unwrap();

        assert_eq!(r.groups.len(), 1);
        let g = &r.groups[0];
        assert_eq!(g.claim_count, 1);
        assert!(g.contradictions.is_empty());
        assert_ne!(g.standing, "contested");
    }

    /// The store's namespace filter is a raw prefix; the dossier's is not.
    #[test]
    fn a_namespace_that_merely_shares_a_prefix_is_outside() {
        let mut s = store();
        put(&mut s, "team", "X", "p", "ours", &["jd"]);
        put(&mut s, "team:sub", "X", "p", "child", &["jd"]);
        put(&mut s, "teamwork", "X", "p", "theirs", &["x"]);

        let r = build(&s, request("X"), NOW_MS).unwrap();
        assert_eq!(r.groups[0].claim_count, 2);
        assert_eq!(r.groups[0].roots, vec!["jd"]);
    }

    #[test]
    fn a_predicate_narrows_the_dossier_to_one_group() {
        let mut s = store();
        put(
            &mut s,
            "team",
            "DateParser",
            "provides",
            "ISO 8601",
            &["jd"],
        );
        put(
            &mut s,
            "team",
            "DateParser",
            "is located at",
            "src/date.rs",
            &["jd"],
        );

        let mut req = request("DateParser");
        req.predicate = Some("provides".into());
        let r = build(&s, req, NOW_MS).unwrap();
        assert_eq!(r.groups.len(), 1);
        assert_eq!(r.groups[0].predicate, "provides");
    }

    #[test]
    fn the_claim_limit_is_reported_as_truncation() {
        let mut s = store();
        for i in 0..5 {
            put(&mut s, "team", "X", "p", &format!("o{}", i), &["jd"]);
        }
        let mut req = request("X");
        req.limit_per_kind = 3;
        let r = build(&s, req, NOW_MS).unwrap();
        assert_eq!(r.groups[0].claim_count, 3);
        assert_eq!(r.truncated, vec!["claim"]);
    }

    #[test]
    fn procedures_and_goals_join_by_intent_without_issuing_receipts() {
        let mut s = store();
        let procedure = Procedure {
            id: ProcedureId::new(),
            namespace: "team".into(),
            name: "use-date-parser".into(),
            version: 1,
            supersedes: None,
            is_current: true,
            source: ProcedureSource::Authored,
            goal: "goal:team/parse-dates".into(),
            intent: "parse a timestamp with DateParser".into(),
            tags: vec![],
            parameters: vec![],
            preconditions: vec![],
            required_tools: vec![],
            postconditions: vec![],
            est_duration_sec: None,
            usage_notes: String::new(),
            context_tags: vec![],
            body_format: BodyFormat::Prose,
            content_type: "text/plain".into(),
            body: "call DateParser::parse".into(),
            tier: Tier::Task,
            use_count: 0,
            success_count: 3,
            failure_count: 1,
            unknown_count: 0,
            last_used_at: Some(NOW_MS),
            created_at: NOW_MS,
            updated_at: NOW_MS,
            stale_at: None,
        };
        s.upsert_procedure(&procedure).unwrap();
        s.upsert_goal(&Goal {
            id: GoalId::new(),
            namespace: "team".into(),
            name: "parse-dates".into(),
            intent: "parse dates with DateParser".into(),
            definition_of_done: vec![],
            tier: Tier::Task,
            created_at: NOW_MS,
            updated_at: NOW_MS,
            stale_at: None,
        })
        .unwrap();

        let r = build(&s, request("DateParser"), NOW_MS).unwrap();

        assert_eq!(r.procedures.len(), 1);
        assert_eq!(r.procedures[0].name, "use-date-parser");
        assert!((r.procedures[0].effectiveness - 0.75).abs() < 1e-9);
        assert_eq!(r.goals.len(), 1);
        assert_eq!(r.goals[0].accomplish_children, 0);
        assert_eq!(r.intent_match, "substring");
        assert_eq!(
            s.count_receipts(boswell_domain::ReceiptStatus::Pending)
                .unwrap(),
            0,
            "a dossier is free: it issues no receipt"
        );
    }

    #[test]
    fn questions_are_reported_unavailable() {
        let r = build(&store(), request("X"), NOW_MS).unwrap();
        assert_eq!(r.unavailable, vec!["question"]);
    }

    #[test]
    fn bad_requests_are_refused() {
        let s = store();
        let empty = DossierRequest::default();
        assert_eq!(
            build(&s, empty, NOW_MS).unwrap_err().code(),
            tonic::Code::InvalidArgument
        );

        let mut unknown_kind = request("X");
        unknown_kind.kinds = vec!["rumor".into()];
        assert_eq!(
            build(&s, unknown_kind, NOW_MS).unwrap_err().code(),
            tonic::Code::InvalidArgument
        );

        // This store has no vector index, so a topic alone has nothing to anchor on.
        let topic_only = DossierRequest {
            topic: Some("date handling".into()),
            ..DossierRequest::default()
        };
        assert_eq!(
            build(&s, topic_only, NOW_MS).unwrap_err().code(),
            tonic::Code::FailedPrecondition
        );
    }
}
