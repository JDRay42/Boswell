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
        actual: usize,
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
    pub fn search(
        &self,
        query: &[f32],
        k: usize,
        ef_search: usize,
    ) -> Result<Vec<(ClaimId, f32)>, VectorIndexError> {
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

        // The exact vector we'll query for.
        let claim_id1 = ClaimId::new();
        let embedding1: Vec<f32> = (0..384).map(|i| (i as f32) / 384.0).collect();
        index.add(claim_id1, &embedding1).unwrap();

        // Populate the index with several more distinct vectors. HNSW is an
        // approximate index whose recall is unreliable with only a couple of
        // elements (a top-k search may return fewer than k), so we index enough
        // vectors for a top-2 query to be reliably filled.
        for n in 1..=10 {
            let mut embedding: Vec<f32> = (0..384).map(|i| (i as f32) / 384.0).collect();
            embedding[0] = n as f32 / 10.0; // perturb so each is distinct
            index.add(ClaimId::new(), &embedding).unwrap();
        }

        assert_eq!(index.len(), 11);

        let results = index.search(&embedding1, 2, 64).unwrap();
        assert_eq!(results.len(), 2);

        // The exact match ranks first with near-perfect similarity.
        assert_eq!(results[0].0, claim_id1);
        assert!(results[0].1 > 0.99);
    }

    #[test]
    fn test_dimension_mismatch() {
        let index = VectorIndex::new(384);

        let claim_id = ClaimId::new();
        let wrong_embedding = vec![0.1; 128]; // Wrong dimension

        let result = index.add(claim_id, &wrong_embedding);
        assert!(matches!(
            result,
            Err(VectorIndexError::DimensionMismatch { .. })
        ));
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
        let frac = std::f32::consts::FRAC_1_SQRT_2;
        let embedding3 = vec![frac, frac, 0.0]; // 45 degrees from X
        index.add(claim_id3, &embedding3).unwrap();

        // Search for nearest to X axis. Ask for more than we inserted so recall is
        // effectively complete; HNSW is approximate, so we assert on the
        // similarities of whatever it returns, not on positional order or on a
        // fixed result count (both made this test flaky).
        let results = index.search(&embedding1, 10, 64).unwrap();
        assert!(!results.is_empty(), "search returned nothing");

        // Results come back in non-increasing similarity order.
        for pair in results.windows(2) {
            assert!(
                pair[0].1 >= pair[1].1,
                "results must be ordered by similarity: {} then {}",
                pair[0].1,
                pair[1].1
            );
        }

        // Each returned claim carries the cosine similarity its vector implies;
        // check the band for whichever ones came back.
        let sims: std::collections::HashMap<_, _> = results.into_iter().collect();
        let sim1 = *sims
            .get(&claim_id1)
            .expect("the exact-match query point must be recalled");
        assert!(sim1 > 0.99, "exact match should be ~1.0, got {}", sim1);
        if let Some(&sim3) = sims.get(&claim_id3) {
            assert!((0.5..0.99).contains(&sim3), "cos(45) ~ 0.707, got {}", sim3);
        }
        if let Some(&sim2) = sims.get(&claim_id2) {
            assert!(sim2 < 0.1, "cos(90) = 0, got {}", sim2);
        }
    }

    #[test]
    fn test_cosine_similarity_function_is_exact() {
        // The pure similarity function is deterministic (no HNSW involved).
        let x = [1.0_f32, 0.0, 0.0];
        let y = [0.0_f32, 1.0, 0.0];
        let frac = std::f32::consts::FRAC_1_SQRT_2;
        let diag = [frac, frac, 0.0];

        assert!((crate::cosine_similarity(&x, &x) - 1.0).abs() < 1e-6);
        assert!(crate::cosine_similarity(&x, &y).abs() < 1e-6); // orthogonal
        assert!((crate::cosine_similarity(&x, &diag) - frac).abs() < 1e-6); // 45 degrees
    }
}
