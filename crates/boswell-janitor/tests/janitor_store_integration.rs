//! Integration tests exercising the Janitor against a real SqliteStore, proving
//! that decay-aware demotion and stale-claim GC actually mutate the store (not
//! just the mock used in unit tests).
//!
//! Timestamps are relative to wall-clock `now` because `Janitor::sweep` reads the
//! current time internally.

use boswell_domain::traits::ClaimStore;
use boswell_domain::{Claim, ClaimId};
use boswell_janitor::Janitor;
use boswell_store::SqliteStore;
use std::time::{SystemTime, UNIX_EPOCH};

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn claim(id: ClaimId, tier: &str, created_at: u64, conf: (f64, f64)) -> Claim {
    Claim {
        id,
        namespace: "test".to_string(),
        subject: "entity:x".to_string(),
        predicate: "is:a".to_string(),
        object: "value:y".to_string(),
        source_type: "assertion".to_string(),
        confidence: conf,
        tier: tier.to_string(),
        created_at,
        stale_at: None,
    }
}

#[test]
fn test_janitor_deletes_stale_ephemeral_in_sqlite() {
    let mut store = SqliteStore::new(":memory:", false, 0).unwrap();
    let now = now_secs();

    let stale = ClaimId::new();
    let fresh = ClaimId::new();
    // Ephemeral TTL is 12h; 20h old is stale, 2h old is fresh.
    store
        .assert_claim(claim(stale, "ephemeral", now - 20 * 3600, (0.8, 0.9)))
        .unwrap();
    store
        .assert_claim(claim(fresh, "ephemeral", now - 2 * 3600, (0.8, 0.9)))
        .unwrap();

    let mut janitor = Janitor::default_config();
    janitor.sweep(&mut store).unwrap();

    // The stale claim is really gone from SQLite; the fresh one remains.
    assert!(
        store.get_claim(stale).unwrap().is_none(),
        "stale claim should be deleted"
    );
    assert!(
        store.get_claim(fresh).unwrap().is_some(),
        "fresh claim should remain"
    );
}

#[test]
fn test_janitor_demotes_decayed_task_in_sqlite() {
    let mut store = SqliteStore::new(":memory:", false, 0).unwrap();
    let now = now_secs();

    // Task TTL is 24h (deletion) and demotion kicks in past 18h (3/4 TTL). At 20h
    // the claim is NOT deleted but IS a demotion candidate, and a 0.8 base decays
    // (12h half-life) to ~0.25 — below the 0.3 threshold.
    let id = ClaimId::new();
    store
        .assert_claim(claim(id, "task", now - 20 * 3600, (0.8, 0.9)))
        .unwrap();

    let mut janitor = Janitor::default_config();
    janitor.sweep(&mut store).unwrap();

    let after = store
        .get_claim(id)
        .unwrap()
        .expect("claim should still exist");
    assert_eq!(
        after.tier, "ephemeral",
        "decayed task claim should be demoted, not deleted"
    );
    assert_eq!(janitor.metrics().total_demoted(), 1);
}

#[test]
fn test_janitor_leaves_fresh_high_confidence_task_alone() {
    let mut store = SqliteStore::new(":memory:", false, 0).unwrap();
    let now = now_secs();

    // Fresh task claim: not stale, not decayed → untouched (may be promoted, but
    // must not be deleted or demoted).
    let id = ClaimId::new();
    store
        .assert_claim(claim(id, "task", now - 3600, (0.85, 0.95)))
        .unwrap();

    let mut janitor = Janitor::default_config();
    janitor.sweep(&mut store).unwrap();

    let after = store
        .get_claim(id)
        .unwrap()
        .expect("claim should still exist");
    // Not demoted to ephemeral (could be promoted to project, but never demoted).
    assert_ne!(after.tier, "ephemeral");
}
