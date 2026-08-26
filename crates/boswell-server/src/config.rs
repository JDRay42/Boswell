//! Configuration for the Boswell instance server.
//!
//! Loaded from a TOML file (see `config/instance.toml`). Every field has a
//! default, so a minimal or absent config still yields a runnable server.

use serde::Deserialize;
use std::path::Path;
use thiserror::Error;

/// Errors that can occur while loading configuration.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// The config file could not be read.
    #[error("Failed to read config file: {0}")]
    FileRead(#[from] std::io::Error),

    /// The config file was not valid TOML for this schema.
    #[error("Failed to parse config TOML: {0}")]
    TomlParse(#[from] toml::de::Error),
}

/// Top-level instance server configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct InstanceConfig {
    /// Address the gRPC server binds to (e.g. `127.0.0.1`).
    pub bind_address: String,

    /// Port the gRPC server binds to (e.g. `50051`).
    pub bind_port: u16,

    /// Claim storage settings.
    pub storage: StorageConfig,

    /// Embedding backend settings.
    pub embedding: EmbeddingConfig,
}

impl Default for InstanceConfig {
    fn default() -> Self {
        Self {
            bind_address: "127.0.0.1".to_string(),
            bind_port: 50051,
            storage: StorageConfig::default(),
            embedding: EmbeddingConfig::default(),
        }
    }
}

/// Storage settings.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct StorageConfig {
    /// Path to the SQLite database file. Use `:memory:` for an ephemeral store.
    pub db_path: String,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            db_path: "boswell.db".to_string(),
        }
    }
}

/// Which embedding backend the store uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EmbeddingBackend {
    /// Real embeddings from a local Ollama server (see ADR-013).
    Ollama,
    /// Deterministic hash-based embeddings; no external service required.
    Mock,
    /// No vector index; semantic search is disabled.
    None,
}

/// Embedding backend settings.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct EmbeddingConfig {
    /// Backend selection: `ollama`, `mock`, or `none`.
    pub backend: EmbeddingBackend,

    /// Ollama model name (used when `backend = "ollama"`).
    pub model: String,

    /// Ollama endpoint (used when `backend = "ollama"`).
    pub endpoint: String,

    /// Vector dimension for the mock backend (used when `backend = "mock"`).
    pub mock_dimension: usize,
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            backend: EmbeddingBackend::Ollama,
            model: "embeddinggemma".to_string(),
            endpoint: "http://localhost:11434".to_string(),
            mock_dimension: 384,
        }
    }
}

impl InstanceConfig {
    /// Load configuration from a TOML file.
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let contents = std::fs::read_to_string(path)?;
        let config: InstanceConfig = toml::from_str(&contents)?;
        Ok(config)
    }

    /// A commented starter configuration, written by `boswell-server init`.
    pub fn starter_toml() -> &'static str {
        STARTER_TOML
    }
}

/// Commented starter config emitted by the `init` subcommand.
pub const STARTER_TOML: &str = r#"# Boswell instance server configuration

# Address and port the gRPC server binds to.
bind_address = "127.0.0.1"
bind_port = 50051

[storage]
# Path to the SQLite database. Use ":memory:" for an ephemeral store.
db_path = "boswell.db"

[embedding]
# Embedding backend: "ollama" (real, local), "mock" (deterministic, offline),
# or "none" (disable semantic search).
backend = "ollama"

# Ollama settings (used when backend = "ollama").
# Pull the model first: `ollama pull embeddinggemma`
model = "embeddinggemma"
endpoint = "http://localhost:11434"

# Vector dimension for the mock backend (used when backend = "mock").
mock_dimension = 384
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_defaults() {
        let c = InstanceConfig::default();
        assert_eq!(c.bind_address, "127.0.0.1");
        assert_eq!(c.bind_port, 50051);
        assert_eq!(c.storage.db_path, "boswell.db");
        assert_eq!(c.embedding.backend, EmbeddingBackend::Ollama);
        assert_eq!(c.embedding.model, "embeddinggemma");
    }

    #[test]
    fn test_parse_partial_config_uses_defaults() {
        // Only override the port and switch to the mock backend.
        let toml = r#"
            bind_port = 60000
            [embedding]
            backend = "mock"
            mock_dimension = 128
        "#;
        let c: InstanceConfig = toml::from_str(toml).unwrap();
        assert_eq!(c.bind_port, 60000);
        assert_eq!(c.bind_address, "127.0.0.1"); // default preserved
        assert_eq!(c.embedding.backend, EmbeddingBackend::Mock);
        assert_eq!(c.embedding.mock_dimension, 128);
        // Fields not mentioned still fall back to defaults.
        assert_eq!(c.storage.db_path, "boswell.db");
    }

    #[test]
    fn test_backend_parses_all_variants() {
        for (s, expected) in [
            ("ollama", EmbeddingBackend::Ollama),
            ("mock", EmbeddingBackend::Mock),
            ("none", EmbeddingBackend::None),
        ] {
            let toml = format!("[embedding]\nbackend = \"{s}\"");
            let c: InstanceConfig = toml::from_str(&toml).unwrap();
            assert_eq!(c.embedding.backend, expected);
        }
    }

    #[test]
    fn test_starter_toml_is_valid() {
        // The starter config must itself parse cleanly.
        let c: InstanceConfig = toml::from_str(InstanceConfig::starter_toml()).unwrap();
        assert_eq!(c.embedding.backend, EmbeddingBackend::Ollama);
    }
}
