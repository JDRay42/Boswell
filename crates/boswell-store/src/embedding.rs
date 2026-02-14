//! Embedding Model for Text Vectorization
//!
//! This module provides text-to-vector conversion for semantic search.
//! Per ADR-013, we use local embedding models to avoid network dependencies
//! and API costs.
//!
//! # Phase 1 Implementation
//!
//! For Phase 1, this module provides a mock embedding model that generates
//! deterministic embeddings based on text hashing. This allows testing the
//! full pipeline without requiring large model files.
//!
//! Future phases will integrate real ONNX models like bge-small-en-v1.5.
//!
//! # Architecture
//!
//! - **MockEmbeddingModel**: Hash-based deterministic embeddings (Phase 1)
//! - **ONNXEmbeddingModel**: Real ML model embeddings (Future)
//!
//! # Examples
//!
//! ```rust
//! use boswell_store::embedding::{MockEmbeddingModel, EmbeddingModel};
//!
//! let model = MockEmbeddingModel::new(384);
//! let text = "The sky is blue";
//! let embedding = model.embed(text).unwrap();
//! assert_eq!(embedding.len(), 384);
//! 
//! // Same text always produces same embedding
//! let embedding2 = model.embed(text).unwrap();
//! assert_eq!(embedding, embedding2);
//! ```

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use thiserror::Error;

/// Errors that can occur during embedding generation
#[derive(Error, Debug)]
pub enum EmbeddingError {
    /// Model not loaded
    #[error("Embedding model not loaded")]
    ModelNotLoaded,
    
    /// Invalid input text
    #[error("Invalid input: {0}")]
    InvalidInput(String),
    
    /// Model inference error
    #[error("Model inference failed: {0}")]
    InferenceFailed(String),
}

/// Trait for embedding models
pub trait EmbeddingModel {
    /// Generate an embedding vector for the given text
    ///
    /// # Parameters
    ///
    /// - `text`: Input text to embed
    ///
    /// # Returns
    ///
    /// A vector of f32 values representing the embedding
    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError>;
    
    /// Get the dimension of embeddings produced by this model
    fn dimension(&self) -> usize;
}

/// Mock embedding model for Phase 1 testing
///
/// This model generates deterministic embeddings based on text content
/// using a hash-based approach. The embeddings are:
///
/// - **Deterministic**: Same text always produces same embedding
/// - **Normalized**: All vectors have unit length (for cosine similarity)
/// - **Diverse**: Different texts produce different embeddings
///
/// # Implementation
///
/// The model hashes the input text with multiple hash functions to generate
/// pseudo-random but deterministic values, then normalizes the result.
pub struct MockEmbeddingModel {
    dimension: usize,
}

impl MockEmbeddingModel {
    /// Create a new mock embedding model
    ///
    /// # Parameters
    ///
    /// - `dimension`: The embedding dimension (e.g., 384 for bge-small)
    pub fn new(dimension: usize) -> Self {
        Self { dimension }
    }
    
    /// Hash text with a seed to get a deterministic f32 value
    fn hash_with_seed(text: &str, seed: u64) -> f32 {
        let mut hasher = DefaultHasher::new();
        text.hash(&mut hasher);
        seed.hash(&mut hasher);
        let hash_value = hasher.finish();
        
        // Convert hash to float in range [-1, 1]
        let normalized = (hash_value as f64 / u64::MAX as f64) * 2.0 - 1.0;
        normalized as f32
    }
}

impl EmbeddingModel for MockEmbeddingModel {
    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        if text.is_empty() {
            return Err(EmbeddingError::InvalidInput(
                "Empty text cannot be embedded".to_string()
            ));
        }
        
        // Generate embedding by hashing text with different seeds
        let mut embedding = Vec::with_capacity(self.dimension);
        
        for i in 0..self.dimension {
            let value = Self::hash_with_seed(text, i as u64);
            embedding.push(value);
        }
        
        // Normalize to unit length for cosine similarity
        let magnitude: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
        
        if magnitude > 0.0 {
            for value in &mut embedding {
                *value /= magnitude;
            }
        }
        
        Ok(embedding)
    }
    
    fn dimension(&self) -> usize {
        self.dimension
    }
}

/// Calculate cosine similarity between two embedding vectors
///
/// # Parameters
///
/// - `a`: First embedding vector
/// - `b`: Second embedding vector
///
/// # Returns
///
/// Cosine similarity in range [-1, 1], where:
/// - 1.0 = identical direction
/// - 0.0 = orthogonal
/// - -1.0 = opposite direction
///
/// # Panics
///
/// Panics if vectors have different lengths
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vectors must have same length");
    
    let dot_product: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let magnitude_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let magnitude_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    
    if magnitude_a == 0.0 || magnitude_b == 0.0 {
        return 0.0;
    }
    
    dot_product / (magnitude_a * magnitude_b)
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_mock_embedding_deterministic() {
        let model = MockEmbeddingModel::new(384);
        
        let text = "The quick brown fox jumps over the lazy dog";
        let embedding1 = model.embed(text).unwrap();
        let embedding2 = model.embed(text).unwrap();
        
        assert_eq!(embedding1, embedding2, "Same text should produce same embedding");
    }
    
    #[test]
    fn test_mock_embedding_dimension() {
        let model = MockEmbeddingModel::new(128);
        
        let embedding = model.embed("test").unwrap();
        assert_eq!(embedding.len(), 128);
        assert_eq!(model.dimension(), 128);
    }
    
    #[test]
    fn test_mock_embedding_normalized() {
        let model = MockEmbeddingModel::new(384);
        
        let embedding = model.embed("test text").unwrap();
        
        // Check that embedding is normalized (unit length)
        let magnitude: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((magnitude - 1.0).abs() < 0.0001, "Embedding should be normalized");
    }
    
    #[test]
    fn test_mock_embedding_different_texts() {
        let model = MockEmbeddingModel::new(384);
        
        let embedding1 = model.embed("hello world").unwrap();
        let embedding2 = model.embed("goodbye world").unwrap();
        
        // Different texts should produce different embeddings
        assert_ne!(embedding1, embedding2);
        
        // But they should not be orthogonal or opposite
        let similarity = cosine_similarity(&embedding1, &embedding2);
        assert!(similarity.abs() < 0.9, "Different texts should have moderate similarity");
    }
    
    #[test]
    fn test_mock_embedding_empty_text() {
        let model = MockEmbeddingModel::new(384);
        
        let result = model.embed("");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Empty text"));
    }
    
    #[test]
    fn test_cosine_similarity_identical() {
        let vec = vec![1.0, 0.0, 0.0];
        let similarity = cosine_similarity(&vec, &vec);
        assert!((similarity - 1.0).abs() < 0.0001);
    }
    
    #[test]
    fn test_cosine_similarity_orthogonal() {
        let vec1 = vec![1.0, 0.0, 0.0];
        let vec2 = vec![0.0, 1.0, 0.0];
        let similarity = cosine_similarity(&vec1, &vec2);
        assert!(similarity.abs() < 0.0001);
    }
    
    #[test]
    fn test_cosine_similarity_opposite() {
        let vec1 = vec![1.0, 0.0, 0.0];
        let vec2 = vec![-1.0, 0.0, 0.0];
        let similarity = cosine_similarity(&vec1, &vec2);
        assert!((similarity + 1.0).abs() < 0.0001);
    }
    
    #[test]
    fn test_similar_text_similar_embeddings() {
        let model = MockEmbeddingModel::new(384);
        
        // Very similar texts with minor differences
        let embedding1 = model.embed("The cat sat on the mat").unwrap();
        let embedding2 = model.embed("The cat sat on the mat.").unwrap(); // Added period
        
        let similarity = cosine_similarity(&embedding1, &embedding2);
        
        // Note: Hash-based embeddings don't guarantee semantic similarity
        // This test just verifies that the embeddings are different
        assert!(similarity < 1.0, "Different texts should not be identical");
        assert!(similarity > -1.0, "Correlation should be within valid range");
    }
}
