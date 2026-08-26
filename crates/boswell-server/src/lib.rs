#![warn(missing_docs)]

//! Boswell instance server.
//!
//! Constructs a [`SqliteStore`] — with a real local embedder when configured —
//! and serves the Boswell gRPC API. This is the instance process that a
//! `boswell-router` deployment points its registered endpoints at (e.g.
//! `http://localhost:50051`).
//!
//! ```no_run
//! use boswell_server::{run, InstanceConfig};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let config = InstanceConfig::from_file("config/instance.toml")?;
//!     run(config).await?;
//!     Ok(())
//! }
//! ```

pub mod config;

use std::sync::{Arc, Mutex};

use boswell_grpc::{start_server, ServerConfig};
use boswell_store::{EmbeddingModel, OllamaEmbeddingModel, SqliteStore};
use thiserror::Error;

pub use config::{
    ConfigError, EmbeddingBackend, EmbeddingConfig, InstanceConfig, StorageConfig,
};

/// Errors that can occur while starting or running the instance server.
#[derive(Debug, Error)]
pub enum ServerError {
    /// Configuration could not be loaded.
    #[error("Configuration error: {0}")]
    Config(#[from] ConfigError),

    /// The claim store could not be opened or initialized.
    #[error("Store error: {0}")]
    Store(String),

    /// The embedding backend could not be initialized.
    #[error("Embedding backend error ({model}): {message}")]
    Embedding {
        /// The model that failed to initialize.
        model: String,
        /// The underlying error message.
        message: String,
    },

    /// The gRPC server failed to start or exited with an error.
    #[error("gRPC server error: {0}")]
    Serve(String),
}

/// Build the claim store described by `config`, initializing the configured
/// embedding backend.
pub fn build_store(config: &InstanceConfig) -> Result<SqliteStore, ServerError> {
    let db_path = &config.storage.db_path;

    match config.embedding.backend {
        EmbeddingBackend::None => {
            tracing::info!("Embedding backend disabled; semantic search unavailable");
            SqliteStore::new(db_path, false, 0).map_err(|e| ServerError::Store(e.to_string()))
        }
        EmbeddingBackend::Mock => {
            let dim = config.embedding.mock_dimension;
            tracing::info!("Using mock embedder (dimension {})", dim);
            SqliteStore::new(db_path, true, dim).map_err(|e| ServerError::Store(e.to_string()))
        }
        EmbeddingBackend::Ollama => {
            let model = &config.embedding.model;
            tracing::info!(
                "Connecting to Ollama embedder '{}' at {}",
                model,
                config.embedding.endpoint
            );
            let embedder = OllamaEmbeddingModel::new(&config.embedding.endpoint, model)
                .map_err(|e| ServerError::Embedding {
                    model: model.clone(),
                    message: e.to_string(),
                })?;
            tracing::info!(
                "Ollama embedder ready: model='{}', dimension={}",
                model,
                embedder.dimension()
            );
            SqliteStore::with_embedding_model(db_path, Box::new(embedder))
                .map_err(|e| ServerError::Store(e.to_string()))
        }
    }
}

/// Build the store and run the gRPC server until it is shut down.
pub async fn run(config: InstanceConfig) -> Result<(), ServerError> {
    let store = build_store(&config)?;
    let store = Arc::new(Mutex::new(store));

    let server_config = ServerConfig::new(config.bind_address.clone(), config.bind_port);

    tracing::info!(
        "Boswell instance server starting on {}:{} (db: {})",
        config.bind_address,
        config.bind_port,
        config.storage.db_path
    );

    start_server(server_config, store)
        .await
        .map_err(|e| ServerError::Serve(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use boswell_domain::traits::ClaimStore;
    use boswell_domain::{Claim, ClaimId};

    fn memory_config(backend: EmbeddingBackend) -> InstanceConfig {
        InstanceConfig {
            storage: StorageConfig {
                db_path: ":memory:".to_string(),
            },
            embedding: EmbeddingConfig {
                backend,
                mock_dimension: 64,
                ..EmbeddingConfig::default()
            },
            ..InstanceConfig::default()
        }
    }

    #[test]
    fn test_build_store_mock_enables_semantic_search() {
        let mut store = build_store(&memory_config(EmbeddingBackend::Mock)).unwrap();
        assert!(store.supports_semantic_search());

        // The wired store embeds on assert and can search by text.
        let id = ClaimId::new();
        store
            .assert_claim(Claim {
                id,
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

        let hits = store
            .semantic_search("rust is_a programming_language", 5, 0.5)
            .unwrap();
        assert_eq!(hits.first().map(|(c, _)| c.id), Some(id));
    }

    #[test]
    fn test_build_store_none_disables_semantic_search() {
        let store = build_store(&memory_config(EmbeddingBackend::None)).unwrap();
        assert!(!store.supports_semantic_search());
    }
}
