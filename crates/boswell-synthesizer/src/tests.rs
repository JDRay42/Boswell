//! End-to-end integration tests for the Synthesizer using a real SQLite store
//! and the deterministic MockProvider.

use crate::{Synthesizer, SynthesizerConfig, SynthesisScope};
use boswell_domain::traits::{ClaimQuery, ClaimStore};
use boswell_domain::{Claim, ClaimId, Relationship, RelationshipType};
use boswell_llm::MockProvider;
use boswell_store::SqliteStore;

/// A well-formed positive-insight response. Subject/predicate/object all use
/// `namespace:value` form so the default Gatekeeper accepts them.
const POSITIVE_INSIGHT: &str = r#"{
    "insight": true,
    "subject": "team:atlas",
    "predicate": "trend:focus",
    "object": "topic:authentication",
    "confidence_lower": 0.8,
    "confidence_upper": 0.9,
    "rationale": "Several members are working on authentication."
}"#;

const NO_INSIGHT: &str = r#"{"insight": false, "rationale": "Claims are unrelated."}"#;

fn new_store() -> SqliteStore {
    SqliteStore::new(":memory:", false, 0).expect("in-memory store")
}

/// Insert a claim directly into the store (bypassing the Gatekeeper).
fn insert(
    store: &mut SqliteStore,
    subject: &str,
    predicate: &str,
    object: &str,
    tier: &str,
    confidence: (f64, f64),
) -> ClaimId {
    let claim = Claim {
        id: ClaimId::new(),
        namespace: "eng:team".to_string(),
        subject: subject.to_string(),
        predicate: predicate.to_string(),
        object: object.to_string(),
        source_type: "assertion".to_string(),
        confidence,
        tier: tier.to_string(),
        created_at: 1_000,
        stale_at: None,
    };
    store.assert_claim(claim).expect("insert claim")
}

/// Insert three high-confidence claims sharing a subject → one cluster of size 3.
fn insert_cluster(store: &mut SqliteStore) {
    insert(store, "team:atlas", "member:has", "person:alice", "task", (0.9, 0.95));
    insert(store, "team:atlas", "member:has", "person:bob", "task", (0.9, 0.95));
    insert(store, "team:atlas", "member:has", "person:carol", "task", (0.9, 0.95));
}

fn count_task_claims(store: &SqliteStore) -> usize {
    store
        .query_claims(&ClaimQuery {
            tier: Some("task".to_string()),
            ..Default::default()
        })
        .unwrap()
        .len()
}

#[tokio::test]
async fn test_pass_creates_insight() {
    let mut store = new_store();
    insert_cluster(&mut store);

    let synthesizer = Synthesizer::new(MockProvider::new(POSITIVE_INSIGHT), SynthesizerConfig::default());
    let report = synthesizer
        .run_pass(&mut store, SynthesisScope::all("task", 50))
        .await
        .unwrap();

    assert_eq!(report.claims_examined, 3);
    assert_eq!(report.clusters_evaluated, 1);
    assert_eq!(report.insights_created, 1, "{}", report.summary());
    assert_eq!(report.insights.len(), 1);

    // A fourth claim now exists (the derived insight).
    assert_eq!(count_task_claims(&store), 4);

    // The insight is linked to all three constituents via derived_from.
    let insight = &report.insights[0];
    assert_eq!(insight.derived_from.len(), 3);
    let rels = store.get_relationships(insight.claim_id).unwrap();
    let derived: Vec<_> = rels
        .iter()
        .filter(|r| r.relationship_type == RelationshipType::DerivedFrom)
        .collect();
    assert_eq!(derived.len(), 3);
}

#[tokio::test]
async fn test_confidence_propagates_outward() {
    let mut store = new_store();
    insert_cluster(&mut store);

    let synthesizer = Synthesizer::new(MockProvider::new(POSITIVE_INSIGHT), SynthesizerConfig::default());
    let report = synthesizer
        .run_pass(&mut store, SynthesisScope::all("task", 50))
        .await
        .unwrap();

    let insight = &report.insights[0];
    // Constituents have lower bound 0.9; derived lower must not exceed that,
    // and is discounted by the LLM's 0.8 → 0.72.
    assert!(insight.confidence.0 <= 0.9);
    assert!(insight.confidence.0 < 0.9);
    assert!(insight.confidence.1 <= 0.9); // bounded by the LLM upper
    assert!(insight.confidence.0 < insight.confidence.1);
}

#[tokio::test]
async fn test_no_insight_creates_nothing() {
    let mut store = new_store();
    insert_cluster(&mut store);

    let synthesizer = Synthesizer::new(MockProvider::new(NO_INSIGHT), SynthesizerConfig::default());
    let report = synthesizer
        .run_pass(&mut store, SynthesisScope::all("task", 50))
        .await
        .unwrap();

    assert_eq!(report.clusters_evaluated, 1);
    assert_eq!(report.insights_created, 0);
    assert_eq!(count_task_claims(&store), 3);
}

#[tokio::test]
async fn test_disabled_is_noop() {
    let mut store = new_store();
    insert_cluster(&mut store);

    let config = SynthesizerConfig {
        enabled: false,
        ..Default::default()
    };
    let synthesizer = Synthesizer::new(MockProvider::new(POSITIVE_INSIGHT), config);
    let report = synthesizer
        .run_pass(&mut store, SynthesisScope::all("task", 50))
        .await
        .unwrap();

    assert_eq!(report.claims_examined, 0);
    assert_eq!(report.insights_created, 0);
    assert_eq!(count_task_claims(&store), 3);
}

#[tokio::test]
async fn test_dry_run_does_not_persist() {
    let mut store = new_store();
    insert_cluster(&mut store);

    let config = SynthesizerConfig {
        dry_run: true,
        ..Default::default()
    };
    let synthesizer = Synthesizer::new(MockProvider::new(POSITIVE_INSIGHT), config);
    let report = synthesizer
        .run_pass(&mut store, SynthesisScope::all("task", 50))
        .await
        .unwrap();

    // The insight is reported...
    assert_eq!(report.insights_created, 1);
    assert_eq!(report.insights.len(), 1);
    // ...but nothing was written to the store.
    assert_eq!(count_task_claims(&store), 3);
}

#[tokio::test]
async fn test_below_min_cluster_size() {
    let mut store = new_store();
    // Only one claim → no cluster reaches min_cluster_size (3).
    insert(&mut store, "team:atlas", "member:has", "person:alice", "task", (0.9, 0.95));

    let synthesizer = Synthesizer::new(MockProvider::new(POSITIVE_INSIGHT), SynthesizerConfig::default());
    let report = synthesizer
        .run_pass(&mut store, SynthesisScope::all("task", 50))
        .await
        .unwrap();

    assert_eq!(report.clusters_evaluated, 0);
    assert_eq!(report.insights_created, 0);
}

#[tokio::test]
async fn test_low_confidence_insight_rejected() {
    let mut store = new_store();
    // Weak constituents: task tier requires lower >= 0.4, and propagation will
    // push the derived lower bound below the min_insight_confidence bar.
    insert(&mut store, "team:atlas", "member:has", "person:alice", "task", (0.45, 0.5));
    insert(&mut store, "team:atlas", "member:has", "person:bob", "task", (0.45, 0.5));
    insert(&mut store, "team:atlas", "member:has", "person:carol", "task", (0.45, 0.5));

    // LLM reports low confidence in the inference.
    let weak_insight = r#"{"insight": true, "subject": "team:atlas", "predicate": "trend:focus", "object": "topic:auth", "confidence_lower": 0.2, "confidence_upper": 0.3, "rationale": "weak"}"#;
    let synthesizer = Synthesizer::new(MockProvider::new(weak_insight), SynthesizerConfig::default());
    let report = synthesizer
        .run_pass(&mut store, SynthesisScope::all("task", 50))
        .await
        .unwrap();

    // Upper 0.3 < min_insight_confidence 0.5 → rejected.
    assert_eq!(report.clusters_evaluated, 1);
    assert_eq!(report.insights_created, 0);
    assert_eq!(report.insights_rejected, 1);
    assert_eq!(count_task_claims(&store), 3);
}

#[tokio::test]
async fn test_derivation_depth_limit() {
    let mut store = new_store();

    // A base (ephemeral) claim that the candidates were derived from.
    let base = insert(&mut store, "base:z", "is:a", "thing:origin", "ephemeral", (0.5, 0.6));

    // Three task claims sharing a subject, each already derived_from `base`
    // (so their derivation depth is 1).
    let c1 = insert(&mut store, "team:atlas", "member:has", "person:alice", "task", (0.9, 0.95));
    let c2 = insert(&mut store, "team:atlas", "member:has", "person:bob", "task", (0.9, 0.95));
    let c3 = insert(&mut store, "team:atlas", "member:has", "person:carol", "task", (0.9, 0.95));
    for c in [c1, c2, c3] {
        store
            .add_relationship(Relationship::new(c, base, RelationshipType::DerivedFrom, 0.9, 1_000))
            .unwrap();
    }

    // max_derivation_depth = 1 → the cluster (depth 1) must be skipped.
    let config = SynthesizerConfig {
        max_derivation_depth: 1,
        ..Default::default()
    };
    let synthesizer = Synthesizer::new(MockProvider::new(POSITIVE_INSIGHT), config);
    let report = synthesizer
        .run_pass(&mut store, SynthesisScope::all("task", 50))
        .await
        .unwrap();

    assert_eq!(report.clusters_depth_skipped, 1, "{}", report.summary());
    assert_eq!(report.clusters_evaluated, 0);
    assert_eq!(report.insights_created, 0);
}

#[tokio::test]
async fn test_since_filter_excludes_old_claims() {
    let mut store = new_store();
    insert_cluster(&mut store); // created_at = 1_000

    let synthesizer = Synthesizer::new(MockProvider::new(POSITIVE_INSIGHT), SynthesizerConfig::default());
    let scope = SynthesisScope {
        namespaces: None,
        min_tier: "task".to_string(),
        since: Some(2_000), // all claims are older than this
        max_clusters: 50,
    };
    let report = synthesizer.run_pass(&mut store, scope).await.unwrap();

    assert_eq!(report.claims_examined, 0);
    assert_eq!(report.insights_created, 0);
}
