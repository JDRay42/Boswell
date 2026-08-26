//! Integration tests for semantic search functionality
//!
//! These tests verify vector search works correctly with the HNSW index.

use boswell_domain::traits::ClaimStore;
use boswell_domain::{Claim, ClaimId};
use boswell_store::SqliteStore;

#[test]
fn test_semantic_search_basic() {
    // Create store with vector search enabled (384 dimensions)
    let mut store = SqliteStore::new(":memory:", true, 384).unwrap();

    // Create some test claims
    let claim_id1 = ClaimId::new();
    let claim1 = Claim {
        id: claim_id1,
        namespace: "test".to_string(),
        subject: "rust".to_string(),
        predicate: "is_a".to_string(),
        object: "programming_language".to_string(),
        source_type: "assertion".to_string(),
        confidence: (0.9, 0.95),
        tier: "permanent".to_string(),
        created_at: 1000,
        stale_at: None,
    };

    let claim_id2 = ClaimId::new();
    let claim2 = Claim {
        id: claim_id2,
        namespace: "test".to_string(),
        subject: "python".to_string(),
        predicate: "is_a".to_string(),
        object: "programming_language".to_string(),
        source_type: "assertion".to_string(),
        confidence: (0.9, 0.95),
        tier: "permanent".to_string(),
        created_at: 1001,
        stale_at: None,
    };

    // Assert claims
    store.assert_claim(claim1).unwrap();
    store.assert_claim(claim2).unwrap();

    // Create embeddings (mock embeddings for testing)
    let embedding1: Vec<f32> = (0..384).map(|i| (i as f32) / 384.0).collect();
    let mut embedding2: Vec<f32> = (0..384).map(|i| (i as f32) / 384.0).collect();
    embedding2[0] = 0.5; // Make it slightly different

    // Add embeddings
    store.add_embedding(claim_id1, &embedding1).unwrap();
    store.add_embedding(claim_id2, &embedding2).unwrap();

    // Search for similar claims
    let results = store
        .semantic_search_by_embedding(&embedding1, 2, 64, 0.8)
        .unwrap();

    // Should return both claims, with claim1 being more similar
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].0.id, claim_id1);
    assert!(results[0].1 > 0.99); // Very high similarity
    assert_eq!(results[1].0.id, claim_id2);
}

#[test]
fn test_semantic_search_disabled() {
    // Create store without vector search
    let mut store = SqliteStore::new(":memory:", false, 0).unwrap();

    let claim_id = ClaimId::new();
    let claim = Claim {
        id: claim_id,
        namespace: "test".to_string(),
        subject: "test".to_string(),
        predicate: "test".to_string(),
        object: "test".to_string(),
        source_type: "assertion".to_string(),
        confidence: (0.9, 0.95),
        tier: "ephemeral".to_string(),
        created_at: 1000,
        stale_at: None,
    };

    store.assert_claim(claim).unwrap();

    // Attempt semantic search should fail
    let embedding: Vec<f32> = vec![0.1; 384];
    let result = store.semantic_search_by_embedding(&embedding, 5, 64, 0.8);

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not enabled"));
}

#[test]
fn test_add_embedding_for_nonexistent_claim() {
    let store = SqliteStore::new(":memory:", true, 384).unwrap();

    let nonexistent_id = ClaimId::new();
    let embedding: Vec<f32> = vec![0.1; 384];

    let result = store.add_embedding(nonexistent_id, &embedding);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not found"));
}

#[test]
fn test_semantic_search_with_threshold() {
    let mut store = SqliteStore::new(":memory:", true, 3).unwrap();

    // Create claims with very different embeddings
    let claim_id1 = ClaimId::new();
    let claim1 = Claim {
        id: claim_id1,
        namespace: "test".to_string(),
        subject: "similar".to_string(),
        predicate: "is".to_string(),
        object: "close".to_string(),
        source_type: "assertion".to_string(),
        confidence: (0.9, 0.95),
        tier: "task".to_string(),
        created_at: 1000,
        stale_at: None,
    };

    let claim_id2 = ClaimId::new();
    let claim2 = Claim {
        id: claim_id2,
        namespace: "test".to_string(),
        subject: "different".to_string(),
        predicate: "is".to_string(),
        object: "far".to_string(),
        source_type: "assertion".to_string(),
        confidence: (0.8, 0.9),
        tier: "task".to_string(),
        created_at: 1001,
        stale_at: None,
    };

    store.assert_claim(claim1).unwrap();
    store.assert_claim(claim2).unwrap();

    // Very similar embedding
    let embedding1 = vec![1.0, 0.0, 0.0];
    // Orthogonal embedding (should have low similarity)
    let embedding2 = vec![0.0, 1.0, 0.0];

    store.add_embedding(claim_id1, &embedding1).unwrap();
    store.add_embedding(claim_id2, &embedding2).unwrap();

    // Search with high threshold - should only return claim1
    let results = store
        .semantic_search_by_embedding(&embedding1, 10, 64, 0.95)
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0.id, claim_id1);
    assert!(results[0].1 > 0.99);
}

#[test]
fn test_semantic_search_by_text_trait() {
    // Vector search enabled: claims are auto-embedded from "subject predicate object".
    let mut store = SqliteStore::new(":memory:", true, 384).unwrap();
    assert!(store.supports_semantic_search());

    let rust_id = ClaimId::new();
    store
        .assert_claim(Claim {
            id: rust_id,
            namespace: "lang".to_string(),
            subject: "rust".to_string(),
            predicate: "is_a".to_string(),
            object: "programming_language".to_string(),
            source_type: "assertion".to_string(),
            confidence: (0.9, 0.95),
            tier: "permanent".to_string(),
            created_at: 1000,
            stale_at: None,
        })
        .unwrap();
    store
        .assert_claim(Claim {
            id: ClaimId::new(),
            namespace: "food".to_string(),
            subject: "banana".to_string(),
            predicate: "is_a".to_string(),
            object: "fruit".to_string(),
            source_type: "assertion".to_string(),
            confidence: (0.9, 0.95),
            tier: "permanent".to_string(),
            created_at: 1001,
            stale_at: None,
        })
        .unwrap();

    // Trait-level text search embeds the query with the same model.
    let results = store
        .semantic_search("rust is_a programming_language", 5, 0.5)
        .unwrap();

    assert!(!results.is_empty(), "expected at least one match");
    // The exact-text match ranks first with near-perfect similarity.
    assert_eq!(results[0].0.id, rust_id);
    assert!(results[0].1 > 0.99, "similarity was {}", results[0].1);
}

#[test]
fn test_supports_semantic_search_flag() {
    let with = SqliteStore::new(":memory:", true, 384).unwrap();
    assert!(with.supports_semantic_search());

    let without = SqliteStore::new(":memory:", false, 0).unwrap();
    assert!(!without.supports_semantic_search());
    // Text search against a non-vector store surfaces a clear error.
    assert!(without.semantic_search("anything", 5, 0.5).is_err());
}

// End-to-end semantic search with the real local embedder (EmbeddingGemma via
// Ollama). Ignored by default because it needs a running Ollama:
//   ollama pull embeddinggemma
//   cargo test -p boswell-store --test semantic_search_tests --ignored real_embedder
#[test]
#[ignore = "requires a local Ollama with embeddinggemma"]
fn test_semantic_search_with_real_embedder() {
    use boswell_store::OllamaEmbeddingModel;

    let embedder = OllamaEmbeddingModel::default_local("embeddinggemma").unwrap();
    let mut store = SqliteStore::with_embedding_model(":memory:", Box::new(embedder)).unwrap();
    assert!(store.supports_semantic_search());

    // Claims are auto-embedded from "subject predicate object" on assert.
    let facts = [
        ("animals", "cat", "is_a", "feline"),
        ("animals", "dog", "is_a", "canine"),
        ("finance", "invoice", "due_on", "the first of the month"),
    ];
    let mut cat_id = None;
    for (ns, s, p, o) in facts {
        let id = ClaimId::new();
        if s == "cat" {
            cat_id = Some(id);
        }
        store
            .assert_claim(Claim {
                id,
                namespace: ns.to_string(),
                subject: s.to_string(),
                predicate: p.to_string(),
                object: o.to_string(),
                source_type: "assertion".to_string(),
                confidence: (0.9, 0.95),
                tier: "permanent".to_string(),
                created_at: 1000,
                stale_at: None,
            })
            .unwrap();
    }

    // A conceptually-related query (never the literal claim text) should surface
    // the cat claim first thanks to real semantic embeddings.
    let results = store
        .semantic_search("which animal is a kind of cat", 3, 0.0)
        .unwrap();

    assert!(!results.is_empty(), "expected semantic matches");
    assert_eq!(
        results[0].0.id,
        cat_id.unwrap(),
        "the cat claim should rank first for a cat-related query"
    );
}
