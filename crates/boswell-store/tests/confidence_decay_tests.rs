//! Integration tests for age-based confidence decay and the confidence cache
//! (per ADR-007). Timestamps are fixed (not wall-clock) for determinism.

use boswell_domain::traits::ClaimStore;
use boswell_domain::{Claim, ClaimId, DecayConfig};
use boswell_store::SqliteStore;

const CREATED: u64 = 1_000_000;
const TASK_HALF_LIFE: u64 = 12 * 3600; // matches DecayConfig::default().task

fn claim(id: ClaimId, tier: &str, created_at: u64) -> Claim {
    Claim {
        id,
        namespace: "t".to_string(),
        subject: "a".to_string(),
        predicate: "b".to_string(),
        object: "c".to_string(),
        source_type: "assertion".to_string(),
        confidence: (0.8, 0.9),
        tier: tier.to_string(),
        created_at,
        stale_at: None,
    }
}

#[test]
fn test_recompute_then_read_is_decayed() {
    let mut store = SqliteStore::new(":memory:", false, 0).unwrap();
    let cfg = DecayConfig::default();
    let id = ClaimId::new();
    store.assert_claim(claim(id, "task", CREATED)).unwrap();

    // One task half-life later: both bounds halve.
    let now = CREATED + TASK_HALF_LIFE;
    let n = store.recompute_confidence_cache(&cfg, now).unwrap();
    assert_eq!(n, 1);

    let (lo, hi) = store
        .get_effective_confidence(id, &cfg, now)
        .unwrap()
        .unwrap();
    assert!((lo - 0.4).abs() < 1e-6, "lower was {lo}");
    assert!((hi - 0.45).abs() < 1e-6, "upper was {hi}");
}

#[test]
fn test_get_effective_computes_on_cache_miss() {
    let mut store = SqliteStore::new(":memory:", false, 0).unwrap();
    let cfg = DecayConfig::default();
    let id = ClaimId::new();
    store.assert_claim(claim(id, "task", CREATED)).unwrap();

    // No recompute pass ran: the value is computed on demand from base + age.
    let now = CREATED + TASK_HALF_LIFE;
    let (lo, hi) = store
        .get_effective_confidence(id, &cfg, now)
        .unwrap()
        .unwrap();
    assert!((lo - 0.4).abs() < 1e-6);
    assert!((hi - 0.45).abs() < 1e-6);
}

#[test]
fn test_permanent_claim_does_not_decay() {
    let mut store = SqliteStore::new(":memory:", false, 0).unwrap();
    let cfg = DecayConfig::default();
    let id = ClaimId::new();
    store.assert_claim(claim(id, "permanent", CREATED)).unwrap();

    let one_year = 365 * 24 * 3600;
    let (lo, hi) = store
        .get_effective_confidence(id, &cfg, CREATED + one_year)
        .unwrap()
        .unwrap();
    assert!((lo - 0.8).abs() < 1e-9);
    assert!((hi - 0.9).abs() < 1e-9);
}

#[test]
fn test_stale_cache_is_recomputed_on_read() {
    let mut store = SqliteStore::new(":memory:", false, 0).unwrap();
    let cfg = DecayConfig::default();
    let id = ClaimId::new();
    store.assert_claim(claim(id, "task", CREATED)).unwrap();

    // Bake the cache at one half-life (effective ~0.4/0.45, computed_at = now1).
    let now1 = CREATED + TASK_HALF_LIFE;
    store.recompute_confidence_cache(&cfg, now1).unwrap();

    // Read far later (well past the freshness window): the stale entry must be
    // recomputed to the two-half-life value, not returned as 0.4/0.45.
    let now2 = CREATED + 2 * TASK_HALF_LIFE;
    let (lo, hi) = store
        .get_effective_confidence(id, &cfg, now2)
        .unwrap()
        .unwrap();
    assert!((lo - 0.2).abs() < 1e-6, "lower was {lo}");
    assert!((hi - 0.225).abs() < 1e-6, "upper was {hi}");
}

#[test]
fn test_get_effective_missing_claim_is_none() {
    let store = SqliteStore::new(":memory:", false, 0).unwrap();
    let cfg = DecayConfig::default();
    let result = store
        .get_effective_confidence(ClaimId::new(), &cfg, CREATED)
        .unwrap();
    assert!(result.is_none());
}
