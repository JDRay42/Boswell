//! Integration tests for semantic search functionality
//!
//! These tests verify vector search works correctly with the HNSW index.

use boswell_domain::{Claim, ClaimId};
use boswell_domain::traits::ClaimStore;
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
    let results = store.semantic_search(&embedding1, 2, 64, 0.8).unwrap();
    
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
        confidence: (0.9, 0.95),
        tier: "ephemeral".to_string(),
        created_at: 1000,
        stale_at: None,
    };
    
    store.assert_claim(claim).unwrap();
    
    // Attempt semantic search should fail
    let embedding: Vec<f32> = vec![0.1; 384];
    let result = store.semantic_search(&embedding, 5, 64, 0.8);
    
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
    let results = store.semantic_search(&embedding1, 10, 64, 0.95).unwrap();
    
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0.id, claim_id1);
    assert!(results[0].1 > 0.99);
}
