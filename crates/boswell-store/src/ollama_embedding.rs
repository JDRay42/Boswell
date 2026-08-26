//! Local embedding model backed by [Ollama](https://ollama.com).
//!
//! Per ADR-013, Boswell embeds text with a local model to avoid network
//! dependencies on hosted APIs and to keep memory private. This module provides
//! a real embedding backend that talks to a locally-running Ollama instance,
//! superseding the deterministic [`MockEmbeddingModel`](crate::MockEmbeddingModel)
//! used for tests and early phases.
//!
//! # Why a blocking HTTP client
//!
//! [`EmbeddingModel::embed`] is a synchronous trait method, and it is called
//! from inside async request handlers (the gRPC service embeds on `assert` and
//! `search`). A blocking client that spins its own Tokio runtime (such as
//! `reqwest::blocking`) panics when invoked from within an existing runtime, so
//! this uses [`ureq`], which performs blocking socket I/O without a runtime.
//!
//! # Example
//!
//! ```no_run
//! use boswell_store::{OllamaEmbeddingModel, EmbeddingModel};
//!
//! // Connects to the default local Ollama endpoint and probes the model to
//! // discover its output dimension.
//! let model = OllamaEmbeddingModel::default_local("embeddinggemma").unwrap();
//! assert_eq!(model.dimension(), 768);
//!
//! let embedding = model.embed("semantic memory for AI agents").unwrap();
//! assert_eq!(embedding.len(), model.dimension());
//! ```

use std::time::Duration;

use serde::Deserialize;

use crate::embedding::{EmbeddingError, EmbeddingModel};

/// Default local Ollama endpoint.
pub const DEFAULT_OLLAMA_ENDPOINT: &str = "http://localhost:11434";

/// Default request timeout for embedding calls.
pub const DEFAULT_TIMEOUT_SECS: u64 = 60;

/// An [`EmbeddingModel`] that generates embeddings via a local Ollama server.
pub struct OllamaEmbeddingModel {
    endpoint: String,
    model: String,
    dimension: usize,
    agent: ureq::Agent,
}

/// Response body from Ollama's `/api/embed` endpoint.
#[derive(Deserialize)]
struct EmbedResponse {
    #[serde(default)]
    embeddings: Vec<Vec<f32>>,
}

impl OllamaEmbeddingModel {
    /// Create an embedder against the default local endpoint (`localhost:11434`).
    ///
    /// The model is probed once to discover its output dimension and to verify
    /// the server is reachable; construction fails if either is not the case.
    pub fn default_local(model: impl Into<String>) -> Result<Self, EmbeddingError> {
        Self::new(DEFAULT_OLLAMA_ENDPOINT, model)
    }

    /// Create an embedder against a specific Ollama `endpoint`.
    ///
    /// The model is probed once to discover its output dimension and validate
    /// connectivity.
    pub fn new(
        endpoint: impl Into<String>,
        model: impl Into<String>,
    ) -> Result<Self, EmbeddingError> {
        let endpoint = endpoint.into().trim_end_matches('/').to_string();
        let model = model.into();
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
            .build();

        let mut this = Self {
            endpoint,
            model,
            dimension: 0,
            agent,
        };

        // Probe to discover the true output dimension (handles model-specific
        // and Matryoshka-truncated configurations) and confirm connectivity.
        let probe = this.request_embedding("dimension probe")?;
        if probe.is_empty() {
            return Err(EmbeddingError::InferenceFailed(
                "Ollama returned an empty embedding during probe".to_string(),
            ));
        }
        this.dimension = probe.len();

        Ok(this)
    }

    /// The Ollama model name this embedder uses.
    pub fn model_name(&self) -> &str {
        &self.model
    }

    /// The endpoint this embedder targets.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Perform a single embedding request against `/api/embed`.
    fn request_embedding(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        let url = format!("{}/api/embed", self.endpoint);
        let body = serde_json::json!({ "model": self.model, "input": text });

        let response = self.agent.post(&url).send_json(body).map_err(|e| {
            EmbeddingError::InferenceFailed(format!("Ollama request failed: {}", e))
        })?;

        let parsed: EmbedResponse = response.into_json().map_err(|e| {
            EmbeddingError::InferenceFailed(format!("Invalid Ollama response: {}", e))
        })?;

        parsed.embeddings.into_iter().next().ok_or_else(|| {
            EmbeddingError::InferenceFailed("Ollama returned no embeddings".to_string())
        })
    }
}

impl EmbeddingModel for OllamaEmbeddingModel {
    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        if text.trim().is_empty() {
            return Err(EmbeddingError::InvalidInput(
                "Empty text cannot be embedded".to_string(),
            ));
        }

        let embedding = self.request_embedding(text)?;

        if embedding.len() != self.dimension {
            return Err(EmbeddingError::InferenceFailed(format!(
                "Expected {} dimensions, got {}",
                self.dimension,
                embedding.len()
            )));
        }

        Ok(embedding)
    }

    fn dimension(&self) -> usize {
        self.dimension
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // These tests require a running Ollama with `embeddinggemma` pulled, so they
    // are ignored by default. Run with:
    //   ollama pull embeddinggemma
    //   cargo test -p boswell-store --ignored ollama
    const TEST_MODEL: &str = "embeddinggemma";

    #[test]
    #[ignore = "requires a local Ollama with embeddinggemma"]
    fn test_ollama_embed_dimension_and_determinism() {
        let model = OllamaEmbeddingModel::default_local(TEST_MODEL).unwrap();
        assert_eq!(model.dimension(), 768);

        let a = model.embed("semantic memory for AI agents").unwrap();
        let b = model.embed("semantic memory for AI agents").unwrap();
        assert_eq!(a.len(), 768);
        // Ollama embeddings are deterministic for identical input.
        assert_eq!(a, b);
    }

    #[test]
    #[ignore = "requires a local Ollama with embeddinggemma"]
    fn test_ollama_embed_semantic_similarity() {
        use crate::embedding::cosine_similarity;

        let model = OllamaEmbeddingModel::default_local(TEST_MODEL).unwrap();
        let cats = model.embed("cats are small domesticated felines").unwrap();
        let kittens = model.embed("kittens are baby cats").unwrap();
        let finance = model.embed("quarterly tax filing deadlines").unwrap();

        let related = cosine_similarity(&cats, &kittens);
        let unrelated = cosine_similarity(&cats, &finance);

        // Real embeddings should place the two cat sentences closer together
        // than the unrelated finance sentence.
        assert!(
            related > unrelated,
            "expected related ({related}) > unrelated ({unrelated})"
        );
    }

    #[test]
    #[ignore = "requires a local Ollama"]
    fn test_ollama_empty_text_rejected() {
        let model = OllamaEmbeddingModel::default_local(TEST_MODEL).unwrap();
        assert!(model.embed("   ").is_err());
    }
}
