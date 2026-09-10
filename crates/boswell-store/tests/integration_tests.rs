//! Integration tests for boswell-store
//!
//! These tests verify the full CRUD cycle for claims and relationships.

use boswell_domain::traits::{ClaimQuery, ClaimStore};
use boswell_domain::{Claim, ClaimId, Relationship, RelationshipType};
use boswell_store::SqliteStore;

#[test]
fn test_store_initialization() {
    let store = SqliteStore::new(":memory:", false, 0);
    assert!(store.is_ok(), "Store should initialize successfully");
}

#[test]
fn test_assert_and_get_claim() {
    let mut store = SqliteStore::new(":memory:", false, 0).unwrap();

    let claim_id = ClaimId::new();
    let claim = Claim {
        id: claim_id,
        namespace: "test".to_string(),
        subject: "Alice".to_string(),
        predicate: "knows".to_string(),
        object: "Bob".to_string(),
        source_type: "assertion".to_string(),
        confidence: (0.8, 0.9),
        tier: "ephemeral".to_string(),
        created_at: 1000,
        stale_at: None,
    };

    // Assert the claim
    let result = store.assert_claim(claim.clone());
    assert!(result.is_ok(), "Should assert claim successfully");
    assert_eq!(result.unwrap(), claim_id);

    // Retrieve the claim
    let retrieved = store.get_claim(claim_id).unwrap();
    assert!(retrieved.is_some(), "Should retrieve the claim");

    let retrieved_claim = retrieved.unwrap();
    assert_eq!(retrieved_claim.id, claim.id);
    assert_eq!(retrieved_claim.namespace, claim.namespace);
    assert_eq!(retrieved_claim.subject, claim.subject);
    assert_eq!(retrieved_claim.predicate, claim.predicate);
    assert_eq!(retrieved_claim.object, claim.object);
    assert_eq!(retrieved_claim.confidence, claim.confidence);
    assert_eq!(retrieved_claim.tier, claim.tier);
    assert_eq!(retrieved_claim.created_at, claim.created_at);
}

#[test]
fn test_duplicate_detection() {
    let mut store = SqliteStore::new(":memory:", false, 0).unwrap();

    let claim_id = ClaimId::new();
    let claim = Claim {
        id: claim_id,
        namespace: "test".to_string(),
        subject: "Alice".to_string(),
        predicate: "knows".to_string(),
        object: "Bob".to_string(),
        source_type: "assertion".to_string(),
        confidence: (0.8, 0.9),
        tier: "ephemeral".to_string(),
        created_at: 1000,
        stale_at: None,
    };

    // First assertion should succeed
    assert!(store.assert_claim(claim.clone()).is_ok());

    // Second assertion with same ID should fail
    let result = store.assert_claim(claim);
    assert!(result.is_err(), "Should reject duplicate claim");
}

#[test]
fn test_query_claims_by_namespace() {
    let mut store = SqliteStore::new(":memory:", false, 0).unwrap();

    // Add claims in different namespaces
    for i in 0..5 {
        let claim = Claim {
            id: ClaimId::new(),
            namespace: format!("namespace{}", i % 2),
            subject: format!("Subject{}", i),
            predicate: "is".to_string(),
            object: "something".to_string(),
            source_type: "assertion".to_string(),
            confidence: (0.5, 0.6),
            tier: "ephemeral".to_string(),
            created_at: 1000 + i as u64,
            stale_at: None,
        };
        store.assert_claim(claim).unwrap();
    }

    // Query for namespace0
    let query = ClaimQuery {
        namespace: Some("namespace0".to_string()),
        ..Default::default()
    };

    let results = store.query_claims(&query).unwrap();
    assert_eq!(results.len(), 3, "Should find 3 claims in namespace0");

    for claim in &results {
        assert!(claim.namespace.starts_with("namespace0"));
    }
}

#[test]
fn test_query_claims_by_triple() {
    let mut store = SqliteStore::new(":memory:", false, 0).unwrap();

    // Three claims sharing a subject, differing in predicate and object, so a
    // filter on one leg cannot be mistaken for a filter on another.
    let triples = [
        ("person:jd", "attr:in-pantry", "ingredient:eggs"),
        ("person:jd", "attr:in-pantry", "ingredient:flour"),
        ("person:jd", "attr:allergic-to", "ingredient:eggs"),
    ];

    for (i, (subject, predicate, object)) in triples.iter().enumerate() {
        let claim = Claim {
            id: ClaimId::new(),
            namespace: "test".to_string(),
            subject: subject.to_string(),
            predicate: predicate.to_string(),
            object: object.to_string(),
            source_type: "assertion".to_string(),
            confidence: (0.5, 0.6),
            tier: "project".to_string(),
            created_at: 1000 + i as u64,
            stale_at: None,
        };
        store.assert_claim(claim).unwrap();
    }

    // All three legs: exactly one row.
    let exact = ClaimQuery {
        subject: Some("person:jd".to_string()),
        predicate: Some("attr:in-pantry".to_string()),
        object: Some("ingredient:eggs".to_string()),
        ..Default::default()
    };
    let results = store.query_claims(&exact).unwrap();
    assert_eq!(results.len(), 1, "the full triple should match one claim");
    assert_eq!(results[0].predicate, "attr:in-pantry");
    assert_eq!(results[0].object, "ingredient:eggs");

    // One leg: the filters are independent, not a single composite.
    let by_object = ClaimQuery {
        object: Some("ingredient:eggs".to_string()),
        ..Default::default()
    };
    assert_eq!(store.query_claims(&by_object).unwrap().len(), 2);

    let by_predicate = ClaimQuery {
        predicate: Some("attr:allergic-to".to_string()),
        ..Default::default()
    };
    assert_eq!(store.query_claims(&by_predicate).unwrap().len(), 1);

    // Exact, not prefix, and case-sensitive — unlike the namespace filter.
    let prefix = ClaimQuery {
        subject: Some("person".to_string()),
        ..Default::default()
    };
    assert!(
        store.query_claims(&prefix).unwrap().is_empty(),
        "subject is an exact match, not a prefix"
    );

    let wrong_case = ClaimQuery {
        subject: Some("Person:JD".to_string()),
        ..Default::default()
    };
    assert!(
        store.query_claims(&wrong_case).unwrap().is_empty(),
        "subject matching is case-sensitive"
    );
}

#[test]
fn test_query_claims_by_tier() {
    let mut store = SqliteStore::new(":memory:", false, 0).unwrap();

    let tiers = ["ephemeral", "task", "project", "permanent"];

    // Add claims with different tiers
    for (i, tier) in tiers.iter().enumerate() {
        let claim = Claim {
            id: ClaimId::new(),
            namespace: "test".to_string(),
            subject: format!("Subject{}", i),
            predicate: "is".to_string(),
            object: "something".to_string(),
            source_type: "assertion".to_string(),
            confidence: (0.5, 0.6),
            tier: tier.to_string(),
            created_at: 1000 + i as u64,
            stale_at: None,
        };
        store.assert_claim(claim).unwrap();
    }

    // Query for task tier
    let query = ClaimQuery {
        tier: Some("task".to_string()),
        ..Default::default()
    };

    let results = store.query_claims(&query).unwrap();
    assert_eq!(results.len(), 1, "Should find 1 claim with tier 'task'");
    assert_eq!(results[0].tier, "task");
}

#[test]
fn test_query_claims_by_confidence() {
    let mut store = SqliteStore::new(":memory:", false, 0).unwrap();

    // Add claims with different confidence levels
    for i in 0..5 {
        let confidence = (0.1 * i as f64, 0.1 * i as f64 + 0.1);
        let claim = Claim {
            id: ClaimId::new(),
            namespace: "test".to_string(),
            subject: format!("Subject{}", i),
            predicate: "is".to_string(),
            object: "something".to_string(),
            source_type: "assertion".to_string(),
            confidence,
            tier: "ephemeral".to_string(),
            created_at: 1000 + i as u64,
            stale_at: None,
        };
        store.assert_claim(claim).unwrap();
    }

    // Query for claims with min_confidence >= 0.3
    let query = ClaimQuery {
        min_confidence: Some(0.3),
        ..Default::default()
    };

    let results = store.query_claims(&query).unwrap();
    assert_eq!(
        results.len(),
        2,
        "Should find 2 claims with confidence >= 0.3"
    );

    for claim in &results {
        assert!(claim.confidence.0 >= 0.3);
    }
}

#[test]
fn test_query_claims_with_limit() {
    let mut store = SqliteStore::new(":memory:", false, 0).unwrap();

    // Add 10 claims
    for i in 0..10 {
        let claim = Claim {
            id: ClaimId::new(),
            namespace: "test".to_string(),
            subject: format!("Subject{}", i),
            predicate: "is".to_string(),
            object: "something".to_string(),
            source_type: "assertion".to_string(),
            confidence: (0.5, 0.6),
            tier: "ephemeral".to_string(),
            created_at: 1000 + i as u64,
            stale_at: None,
        };
        store.assert_claim(claim).unwrap();
    }

    // Query with limit of 5
    let query = ClaimQuery {
        limit: Some(5),
        ..Default::default()
    };

    let results = store.query_claims(&query).unwrap();
    assert_eq!(results.len(), 5, "Should return exactly 5 claims");
}

#[test]
fn test_add_and_get_relationships() {
    let mut store = SqliteStore::new(":memory:", false, 0).unwrap();

    // Create and store two claims
    let claim1_id = ClaimId::new();
    let claim1 = Claim {
        id: claim1_id,
        namespace: "test".to_string(),
        subject: "Alice".to_string(),
        predicate: "knows".to_string(),
        object: "Bob".to_string(),
        source_type: "assertion".to_string(),
        confidence: (0.8, 0.9),
        tier: "ephemeral".to_string(),
        created_at: 1000,
        stale_at: None,
    };

    let claim2_id = ClaimId::new();
    let claim2 = Claim {
        id: claim2_id,
        namespace: "test".to_string(),
        subject: "Bob".to_string(),
        predicate: "knows".to_string(),
        object: "Charlie".to_string(),
        source_type: "assertion".to_string(),
        confidence: (0.7, 0.8),
        tier: "ephemeral".to_string(),
        created_at: 1001,
        stale_at: None,
    };

    store.assert_claim(claim1).unwrap();
    store.assert_claim(claim2).unwrap();

    // Add a relationship
    let relationship =
        Relationship::new(claim1_id, claim2_id, RelationshipType::Supports, 0.9, 1002);

    let result = store.add_relationship(relationship.clone());
    assert!(result.is_ok(), "Should add relationship successfully");

    // Retrieve relationships for claim1
    let relationships = store.get_relationships(claim1_id).unwrap();
    assert_eq!(relationships.len(), 1, "Should have 1 relationship");

    let retrieved_rel = &relationships[0];
    assert_eq!(retrieved_rel.from_claim, claim1_id);
    assert_eq!(retrieved_rel.to_claim, claim2_id);
    assert_eq!(retrieved_rel.relationship_type, RelationshipType::Supports);
    assert_eq!(retrieved_rel.strength, 0.9);
    assert_eq!(retrieved_rel.created_at, 1002);
}

#[test]
fn test_relationship_types() {
    let mut store = SqliteStore::new(":memory:", false, 0).unwrap();

    let claim1_id = ClaimId::new();
    let claim2_id = ClaimId::new();

    // Add two claims
    for (i, id) in [claim1_id, claim2_id].iter().enumerate() {
        let claim = Claim {
            id: *id,
            namespace: "test".to_string(),
            subject: format!("Subject{}", i),
            predicate: "is".to_string(),
            object: "something".to_string(),
            source_type: "assertion".to_string(),
            confidence: (0.5, 0.6),
            tier: "ephemeral".to_string(),
            created_at: 1000 + i as u64,
            stale_at: None,
        };
        store.assert_claim(claim).unwrap();
    }

    // Test all relationship types
    let types = [
        RelationshipType::Supports,
        RelationshipType::Contradicts,
        RelationshipType::DerivedFrom,
        RelationshipType::References,
        RelationshipType::Supersedes,
    ];

    for (i, rel_type) in types.iter().enumerate() {
        let relationship = Relationship::new(claim1_id, claim2_id, *rel_type, 0.8, 1000 + i as u64);

        // Each relationship type is unique, so this should work
        let result = store.add_relationship(relationship);
        assert!(result.is_ok(), "Should add {:?} relationship", rel_type);
    }

    // Verify all relationships were stored
    let relationships = store.get_relationships(claim1_id).unwrap();
    assert_eq!(
        relationships.len(),
        5,
        "Should have 5 different relationship types"
    );
}

#[test]
fn test_uuid_temporal_ordering() {
    let mut store = SqliteStore::new(":memory:", false, 0).unwrap();

    // Create claims with different times (via their UUIDv7s)
    let mut claim_ids = Vec::new();

    for i in 0..5 {
        // Small delay to ensure different ULID timestamps
        std::thread::sleep(std::time::Duration::from_millis(2));

        let claim_id = ClaimId::new();
        claim_ids.push(claim_id);

        let claim = Claim {
            id: claim_id,
            namespace: "test".to_string(),
            subject: format!("Subject{}", i),
            predicate: "is".to_string(),
            object: "something".to_string(),
            source_type: "assertion".to_string(),
            confidence: (0.5, 0.6),
            tier: "ephemeral".to_string(),
            created_at: 1000 + i as u64,
            stale_at: None,
        };
        store.assert_claim(claim).unwrap();
    }

    // Query all claims (they should be in insertion order due to UUIDv7 ordering)
    let query = ClaimQuery::default();
    let results = store.query_claims(&query).unwrap();

    assert_eq!(results.len(), 5);

    // Verify that claim IDs are in ascending order (temporal ordering)
    for i in 0..results.len() - 1 {
        assert!(
            results[i].id < results[i + 1].id,
            "Claims should be ordered by UUIDv7 (temporal order)"
        );
    }
}

#[test]
fn test_get_nonexistent_claim() {
    let store = SqliteStore::new(":memory:", false, 0).unwrap();

    let nonexistent_id = ClaimId::new();
    let result = store.get_claim(nonexistent_id).unwrap();

    assert!(result.is_none(), "Should return None for nonexistent claim");
}

#[test]
fn test_stale_at_field() {
    let mut store = SqliteStore::new(":memory:", false, 0).unwrap();

    let claim_id = ClaimId::new();
    let claim = Claim {
        id: claim_id,
        namespace: "test".to_string(),
        subject: "Alice".to_string(),
        predicate: "knows".to_string(),
        object: "Bob".to_string(),
        source_type: "assertion".to_string(),
        confidence: (0.8, 0.9),
        tier: "ephemeral".to_string(),
        created_at: 1000,
        stale_at: Some(2000),
    };

    store.assert_claim(claim.clone()).unwrap();

    let retrieved = store.get_claim(claim_id).unwrap().unwrap();
    assert_eq!(
        retrieved.stale_at,
        Some(2000),
        "Should preserve stale_at value"
    );
}

/// `count_claims` and `query_claims` share one filter builder so they cannot
/// disagree. This walks a spread of queries and asserts the equivalence the
/// trait promises, including the limited case — a count under a limit is the
/// number of rows that would come back, not the number that match.
#[test]
fn test_count_claims_agrees_with_query_claims() {
    let mut store = SqliteStore::new(":memory:", false, 0).unwrap();

    for i in 0..10 {
        let claim = Claim {
            id: ClaimId::new(),
            namespace: format!("ns{}", i % 2),
            subject: format!("Subject{}", i),
            predicate: "is".to_string(),
            object: "something".to_string(),
            source_type: if i % 3 == 0 { "import" } else { "assertion" }.to_string(),
            confidence: (0.1 * i as f64, 0.9),
            tier: if i % 2 == 0 { "task" } else { "permanent" }.to_string(),
            created_at: 1000 + i as u64,
            stale_at: None,
        };
        store.assert_claim(claim).unwrap();
    }

    let queries = [
        ClaimQuery::default(),
        ClaimQuery {
            namespace: Some("ns0".to_string()),
            ..Default::default()
        },
        ClaimQuery {
            tier: Some("permanent".to_string()),
            ..Default::default()
        },
        ClaimQuery {
            source_type: Some("import".to_string()),
            ..Default::default()
        },
        ClaimQuery {
            min_confidence: Some(0.5),
            ..Default::default()
        },
        ClaimQuery {
            subject: Some("Subject7".to_string()),
            ..Default::default()
        },
        ClaimQuery {
            tier: Some("task".to_string()),
            limit: Some(2),
            ..Default::default()
        },
        ClaimQuery {
            tier: Some("nonexistent".to_string()),
            ..Default::default()
        },
    ];

    for query in &queries {
        let counted = store.count_claims(query).unwrap();
        let queried = store.query_claims(query).unwrap().len() as u64;
        assert_eq!(counted, queried, "count and query disagree for {:?}", query);
    }

    // The spread is only worth something if some of those queries matched.
    assert_eq!(store.count_claims(&ClaimQuery::default()).unwrap(), 10);
    assert_eq!(
        store
            .count_claims(&ClaimQuery {
                tier: Some("task".to_string()),
                limit: Some(2),
                ..Default::default()
            })
            .unwrap(),
        2
    );
}
