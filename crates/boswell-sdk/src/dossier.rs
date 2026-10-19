//! JSON rendering of a dossier (research design §5) and of change events
//! (change feed design §3), shared by every transport that hands them to an
//! agent, so the gateway and MCP shapes cannot drift.

use boswell_grpc::proto::{ChangeEvent, DossierResponse};
use serde_json::{json, Value};

/// Render a dossier as the JSON `17-research.md` §5.2–5.3 specifies.
///
/// `depth` selects the view: `false` is width (groups with counts and a lead,
/// no claims), `true` adds the claims, roots and contradictions. The gateway's
/// `POST /v1/dossier` and the MCP `boswell_dossier` tool both render through
/// this, so the two cannot drift.
pub fn dossier_json(r: &DossierResponse, depth: bool) -> Value {
    let anchors: Vec<Value> = r
        .anchors
        .iter()
        .map(|a| json!({ "subject": a.subject, "matched_by": a.matched_by, "score": a.score }))
        .collect();

    let groups: Vec<Value> = r
        .groups
        .iter()
        .map(|g| {
            let mut v = json!({
                "subject": g.subject,
                "predicate": g.predicate,
                "standing": g.standing,
                "claim_count": g.claim_count,
                "root_count": g.roots.len(),
                "unattributed_count": g.unattributed_count,
                "lead": g.lead,
            });
            if depth {
                v["roots"] = json!(g.roots);
                v["claims"] = g
                    .claims
                    .iter()
                    .filter_map(|dc| {
                        let c = dc.claim.as_ref()?;
                        Some(json!({
                            "claim_id": c.id,
                            "object": c.object,
                            "confidence": c.confidence.as_ref().map(|i| json!({ "lower": i.lower, "upper": i.upper })),
                            "tier": tier_name(c.tier),
                            "source_type": c.source_type,
                            "authors": dc.authors.iter().map(|a| json!({
                                "author": a,
                                "independence_root": boswell_domain::authenticated_principal(a),
                            })).collect::<Vec<_>>(),
                            "stale_at": dc.stale_at,
                        }))
                    })
                    .collect();
                v["contradictions"] = g
                    .contradictions
                    .iter()
                    .map(|c| json!({ "claim_id": c.claim_id, "against": c.against }))
                    .collect();
            }
            v
        })
        .collect();

    let procedures: Vec<Value> = r
        .procedures
        .iter()
        .map(|p| {
            let mut v = json!({
                "id": p.id,
                "name": p.name,
                "intent": p.intent,
                "effectiveness": p.effectiveness,
            });
            if depth {
                v["version"] = json!(p.version);
                v["success_count"] = json!(p.success_count);
                v["failure_count"] = json!(p.failure_count);
                v["unknown_count"] = json!(p.unknown_count);
                v["tier"] = json!(p.tier);
            }
            v
        })
        .collect();

    let goals: Vec<Value> = r
        .goals
        .iter()
        .map(|g| {
            let mut v = json!({
                "id": g.id,
                "name": g.name,
                "accomplish_children": g.accomplish_children,
            });
            if depth {
                v["intent"] = json!(g.intent);
                v["best_child_effectiveness"] = json!(g.best_child_effectiveness);
            }
            v
        })
        .collect();

    json!({
        "view": if depth { "depth" } else { "width" },
        "anchors": anchors,
        "groups": groups,
        "procedures": procedures,
        "goals": goals,
        "questions": [],
        "coverage": {
            "intent_match": r.intent_match,
            "truncated": r.truncated,
            "unavailable": r.unavailable,
        },
    })
}

/// The name of a wire `Tier` enum value (`boswell.v1.Tier`).
fn tier_name(tier: i32) -> &'static str {
    match tier {
        1 => "ephemeral",
        2 => "task",
        3 => "project",
        4 => "permanent",
        _ => "unspecified",
    }
}

/// One change event as the feed renders it, with the independence root its
/// author counts as.
pub fn change_event_json(e: &ChangeEvent) -> Value {
    json!({
        "cursor": e.seq.to_string(),
        "at": e.at,
        "namespace": e.namespace,
        "kind": e.kind,
        "entity_id": e.entity_id,
        "change": e.change,
        "tier": { "before": e.tier_before, "after": e.tier_after },
        "subject": e.subject,
        "summary": e.summary,
        "author": e.author,
        "independence_root": e.author.as_deref().map(boswell_domain::authenticated_principal),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use boswell_grpc::proto::{
        Claim, ConfidenceInterval, DossierClaim, DossierContradiction, DossierGroup,
        DossierProcedure,
    };

    fn response() -> DossierResponse {
        DossierResponse {
            groups: vec![DossierGroup {
                subject: "DateParser".into(),
                predicate: "provides".into(),
                standing: "thin".into(),
                roots: vec!["jd".into()],
                claim_count: 1,
                unattributed_count: 0,
                lead: "ISO 8601".into(),
                claims: vec![DossierClaim {
                    claim: Some(Claim {
                        id: "01J".into(),
                        object: "ISO 8601".into(),
                        confidence: Some(ConfidenceInterval {
                            lower: 0.8,
                            upper: 0.9,
                        }),
                        tier: 2,
                        ..Claim::default()
                    }),
                    authors: vec!["jd/sub:impl-2".into()],
                    stale_at: None,
                }],
                contradictions: vec![DossierContradiction {
                    claim_id: "01J".into(),
                    against: "01K".into(),
                }],
            }],
            procedures: vec![DossierProcedure {
                id: "p1".into(),
                name: "use-date-parser".into(),
                effectiveness: 0.75,
                success_count: 3,
                ..DossierProcedure::default()
            }],
            intent_match: "substring".into(),
            unavailable: vec!["question".into()],
            ..DossierResponse::default()
        }
    }

    /// Width is the inch-deep view: counts and a lead, never the claims.
    #[test]
    fn width_omits_claims_roots_and_detail() {
        let v = dossier_json(&response(), false);
        assert_eq!(v["view"], "width");
        let g = &v["groups"][0];
        assert_eq!(g["root_count"], 1);
        assert_eq!(g["lead"], "ISO 8601");
        assert!(g.get("claims").is_none());
        assert!(g.get("roots").is_none());
        assert!(v["procedures"][0].get("success_count").is_none());
        assert_eq!(v["coverage"]["unavailable"][0], "question");
    }

    /// Depth names each author and the root it counts as, so a reader can see
    /// five agents of one principal are one witness.
    #[test]
    fn depth_names_each_author_and_the_root_it_counts_as() {
        let v = dossier_json(&response(), true);
        let claim = &v["groups"][0]["claims"][0];
        assert_eq!(claim["tier"], "task");
        assert_eq!(claim["authors"][0]["author"], "jd/sub:impl-2");
        assert_eq!(claim["authors"][0]["independence_root"], "jd");
        assert_eq!(v["groups"][0]["contradictions"][0]["against"], "01K");
        assert_eq!(v["procedures"][0]["success_count"], 3);
    }
}
