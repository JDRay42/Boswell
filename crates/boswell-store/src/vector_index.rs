//! HNSW Vector Index for Semantic Search
//!
//! This module provides a wrapper around the HNSW algorithm for efficient
//! nearest-neighbor search over embedding vectors (per ADR-005).
//!
//! # Architecture
//!
//! - In-memory index for Phase 1 (persistence in later phases)
//! - Separated from SQLite for optimal performance
//! - Rebuildable from SQLite on startup
//!
//! # HNSW Parameters
//!
//! - **M**: Number of bi-directional links per node (default: 16)
//!   Higher M = better accuracy but more memory
//! - **efConstruction**: Size of dynamic candidate list during construction (default: 200)
//!   Higher efConstruction = better index quality but slower build
//! - **efSearch**: Size of dynamic candidate list during search (default: 64)
//!   Higher efSearch = better recall but slower queries

use boswell_domain::ClaimId;
use hnsw_rs::prelude::*;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use thiserror::Error;

/// Default HNSW parameters optimized for 384-dimensional embeddings
const DEFAULT_M: usize = 16;
const DEFAULT_EF_CONSTRUCTION: usize = 200;
const DEFAULT_MAX_ELEMENTS: usize = 1_000_000;

/// Errors that can occur during vector index operations
#[derive(Error, Debug)]
pub enum VectorIndexError {
    /// Invalid embedding dimension
    #[error("Invalid embedding dimension: expected {expected}, got {actual}")]
    DimensionMismatch {
        /// Expected dimension
        expected: usize,
        /// Actual dimension provided
        actual: usize
    },
    
    /// Empty search results
    #[error("No results found for query")]
    NoResults,
    
    /// Internal HNSW error
    #[error("HNSW error: {0}")]
    Internal(String),
}

/// A wrapper around HNSW for vector similarity search
///
///This index stores (claim_id, embedding) pairs and provides
/// efficient nearest-neighbor search.
///
/// # Examples
///
/// ```no_run
/// use boswell_store::vector_index::VectorIndex;
/// use boswell_domain::ClaimId;
///
/// let mut index = VectorIndex::new(384);
/// let claim_id = ClaimId::new();
/// let embedding = vec![0.1; 384];
/// index.add(claim_id, &embedding).unwrap();
///
/// let results = index.search(&embedding, 5, 64).unwrap();
/// ```
pub struct VectorIndex {
    /// Expected embedding dimension
    dimension: usize,
    
    /// HNSW index (wrapped in Arc<Mutex> for thread-safe access)
    /// Note: No lifetime parameter - hnsw_rs owns the data
    hnsw: Arc<Mutex<Hnsw<'static, f32, DistCosine>>>,
    
    /// Mapping from internal HNSW IDs to ClaimIds
    id_map: Arc<Mutex<HashMap<usize, ClaimId>>>,
    
    /// Counter for next internal ID
    next_id: Arc<Mutex<usize>>,
}

impl VectorIndex {
    /// Create a new vector index with the specified dimension
    ///
    /// # Parameters
    ///
    /// - `dimension`: Embedding vector dimension (e.g., 384 for bge-small)
    pub fn new(dimension: usize) -> Self {
        // Calculate number of layers based on expected data size
        let nb_layer = 16.min((DEFAULT_MAX_ELEMENTS as f32).ln().trunc() as usize);
        
        // Initialize HNSW
        let hnsw = Hnsw::<'static, f32, DistCosine>::new(
            DEFAULT_M,
            DEFAULT_MAX_ELEMENTS,
            nb_layer,
            DEFAULT_EF_CONSTRUCTION,
            DistCosine {},
        );
        
        Self {
            dimension,
            hnsw: Arc::new(Mutex::new(hnsw)),
            id_map: Arc::new(Mutex::new(HashMap::new())),
            next_id: Arc::new(Mutex::new(0)),
        }
    }
    
    /// Add a claim embedding to the index
    ///
    /// # Parameters
    ///
    /// - `claim_id`: The claim ID
    /// - `embedding`: The embedding vector (must match index dimension)
    pub fn add(&self, claim_id: ClaimId, embedding: &[f32]) -> Result<(), VectorIndexError> {
        if embedding.len() != self.dimension {
            return Err(VectorIndexError::DimensionMismatch {
                expected: self.dimension,
                actual: embedding.len(),
            });
        }
        
        // Get next internal ID
        let mut next_id = self.next_id.lock().unwrap();
        let internal_id = *next_id;
        *next_id += 1;
        drop(next_id);
        
        // Store the mapping
        let mut id_map = self.id_map.lock().unwrap();
        id_map.insert(internal_id, claim_id);
        drop(id_map);
        
        // Insert into HNSW (convert slice to owned Vec for 'static lifetime)
        let embedding_vec = embedding.to_vec();
        let hnsw = self.hnsw.lock().unwrap();
        hnsw.insert((&embedding_vec, internal_id));
        
        Ok(())
    }
    
    /// Search for the k nearest neighbors to the given embedding
    ///
    /// Returns a list of (ClaimId, similarity_score) pairs, sorted by similarity (descending).
    ///
    /// # Parameters
    ///
    /// - `query`: The query embedding vector
    /// - `k`: Number of results to return
    /// - `ef_search`: Search quality parameter (higher = better but slower)
    pub fn search(&self, query: &[f32], k: usize, ef_search: usize) -> Result<Vec<(ClaimId, f32)>, VectorIndexError> {
        if query.len() != self.dimension {
            return Err(VectorIndexError::DimensionMismatch {
                expected: self.dimension,
                actual: query.len(),
            });
        }
        
        let hnsw = self.hnsw.lock().unwrap();
        let id_map = self.id_map.lock().unwrap();
        
        // Search HNSW
        let results = hnsw.search(query, k, ef_search);
        
        // Map internal IDs back to ClaimIds
        let mapped_results: Vec<(ClaimId, f32)> = results
            .into_iter()
            .filter_map(|neighbour| {
                let internal_id = neighbour.d_id;
                id_map.get(&internal_id).map(|&claim_id| {
                    // Convert distance to similarity (cosine distance -> cosine similarity)
                    // HNSW returns distance, we want similarity (1 - distance)
                    let similarity = 1.0 - neighbour.distance;
                    (claim_id, similarity)
                })
            })
            .collect();
        
        Ok(mapped_results)
    }
    
    /// Get the number of vectors in the index
    pub fn len(&self) -> usize {
        let id_map = self.id_map.lock().unwrap();
        id_map.len()
    }
    
    /// Check if the index is empty
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    
    /// Clear all vectors from the index
    pub fn clear(&self) {
        let nb_layer = 16.min((DEFAULT_MAX_ELEMENTS as f32).ln().trunc() as usize);
        
        let hnsw = Hnsw::<'static, f32, DistCosine>::new(
            DEFAULT_M,
            DEFAULT_MAX_ELEMENTS,
            nb_layer,
            DEFAULT_EF_CONSTRUCTION,
            DistCosine {},
        );
        
        let mut hnsw_lock = self.hnsw.lock().unwrap();
        *hnsw_lock = hnsw;
        drop(hnsw_lock);
        
        let mut id_map = self.id_map.lock().unwrap();
        id_map.clear();
        drop(id_map);
        
        let mut next_id = self.next_id.lock().unwrap();
        *next_id = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_vector_index_creation() {
        let index = VectorIndex::new(384);
        assert_eq!(index.dimension, 384);
        assert!(index.is_empty());
    }
    
    #[test]
    fn test_add_and_search() {
        let index = VectorIndex::new(384);
        
        // Add some test vectors
        let claim_id1 = ClaimId::new();
        let embedding1: Vec<f32> = (0..384).map(|i| (i as f32) / 384.0).collect();
        index.add(claim_id1, &embedding1).unwrap();
        
        let claim_id2 = ClaimId::new();
        let mut embedding2: Vec<f32> = (0..384).map(|i| (i as f32) / 384.0).collect();
        // Make it slightly different
        embedding2[0] = 0.5;
        index.add(claim_id2, &embedding2).unwrap();
        
        assert_eq!(index.len(), 2);
        
        // Search for nearest neighbor (use ef_search = 64)
        let results = index.search(&embedding1, 2, 64).unwrap();
        assert_eq!(results.len(), 2);
        
        // First result should be the exact match
        assert_eq!(results[0].0, claim_id1);
        assert!(results[0].1 > 0.99); // Very high similarity
    }
    
    #[test]
    fn test_dimension_mismatch() {
        let index = VectorIndex::new(384);
        
        let claim_id = ClaimId::new();
        let wrong_embedding = vec![0.1; 128]; // Wrong dimension
        
        let result = index.add(claim_id, &wrong_embedding);
        assert!(matches!(result, Err(VectorIndexError::DimensionMismatch { .. })));
    }
    
    #[test]
    fn test_clear() {
        let index = VectorIndex::new(384);
        
        let claim_id = ClaimId::new();
        let embedding: Vec<f32> = (0..384).map(|i| (i as f32) / 384.0).collect();
        index.add(claim_id, &embedding).unwrap();
        
        assert_eq!(index.len(), 1);
        
        index.clear();
        assert!(index.is_empty());
    }
    
    #[test]
    fn test_cosine_similarity() {
        let index = VectorIndex::new(3);
        
        // Create normalized vectors
        let claim_id1 = ClaimId::new();
        let embedding1 = vec![1.0, 0.0, 0.0]; // Unit vector along X
        index.add(claim_id1, &embedding1).unwrap();
        
        let claim_id2 = ClaimId::new();
        let embedding2 = vec![0.0, 1.0, 0.0]; // Unit vector along Y (orthogonal)
        index.add(claim_id2, &embedding2).unwrap();
        
        let claim_id3 = ClaimId::new();
        let embedding3 = vec![0.7071, 0.7071, 0.0]; // 45 degrees from X
        index.add(claim_id3, &embedding3).unwrap();
        
        // Search for nearest to X axis
        let results = index.search(&embedding1, 3, 64).unwrap();
        
        // Should return: embedding1 (exact), embedding3 (45deg), embedding2 (orthogonal)
        assert_eq!(results[0].0, claim_id1);
        assert!(results[0].1 > 0.99); // Near perfect match
        
        assert_eq!(results[1].0, claim_id3);
        assert!(results[1].1 > 0.5); // 45 degree angle = cos(45) ~ 0.707
        
        assert_eq!(results[2].0, claim_id2);
        assert!(results[2].1 < 0.1); // Orthogonal = cos(90) = 0
    }
}
