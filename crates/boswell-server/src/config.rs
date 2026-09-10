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

    /// A `run_between` window could not be understood.
    ///
    /// Refusing to start beats accepting it: a window that never opens means a
    /// background job silently never runs, and it fails at 2 a.m. where nobody
    /// is watching.
    #[error("[{section}] run_between is invalid: {reason}")]
    InvalidRunWindow {
        /// The config section carrying the bad window.
        section: &'static str,
        /// What was wrong with it.
        reason: String,
    },
}

/// Top-level instance server configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct InstanceConfig {
    /// Address the gRPC server binds to. **Loopback only.**
    ///
    /// A routable address is a startup error, not a warning: the instance
    /// authenticates nothing and sits inside the boundary the HTTP gateway
    /// draws (ADR-021). Enforced in `boswell_grpc::ServerConfig`.
    pub bind_address: String,

    /// Port the gRPC server binds to (e.g. `50051`).
    pub bind_port: u16,

    /// Claim storage settings.
    pub storage: StorageConfig,

    /// Embedding backend settings.
    pub embedding: EmbeddingConfig,

    /// Background maintenance (Janitor) settings.
    pub janitor: JanitorSettings,

    /// Background synthesis (Synthesizer) settings.
    pub synthesizer: SynthesizerSettings,

    /// Background contradiction-detection settings.
    pub contradiction: ContradictionSettings,

    /// Server-side LLM extraction settings (backs the `Extract` RPC).
    pub extraction: ExtractionSettings,
}

impl Default for InstanceConfig {
    fn default() -> Self {
        Self {
            bind_address: "127.0.0.1".to_string(),
            bind_port: 50051,
            storage: StorageConfig::default(),
            embedding: EmbeddingConfig::default(),
            janitor: JanitorSettings::default(),
            synthesizer: SynthesizerSettings::default(),
            contradiction: ContradictionSettings::default(),
            extraction: ExtractionSettings::default(),
        }
    }
}

/// Server-side extraction settings. When enabled, the gRPC `Extract` RPC (and
/// LLM-mode hook ingest via the gateway) turns text into claims using a local
/// Ollama chat model. Off by default (LLM cost); deterministic ingest via
/// `Learn` works regardless.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ExtractionSettings {
    /// Whether the server-side Extractor is available for the `Extract` RPC.
    pub enabled: bool,
    /// Ollama chat model used for extraction.
    pub model: String,
    /// Ollama endpoint for the extraction model.
    pub endpoint: String,
    /// Maximum input text length in characters (rejects larger requests).
    pub max_text_length: usize,
}

impl Default for ExtractionSettings {
    fn default() -> Self {
        Self {
            enabled: false, // opt-in (LLM cost)
            model: "granite4.2:8b".to_string(),
            endpoint: "http://localhost:11434".to_string(),
            max_text_length: 50_000,
        }
    }
}

impl ExtractionSettings {
    /// Build the [`boswell_extractor::ExtractorConfig`] this settings block
    /// describes, keeping extractor defaults for everything not exposed here.
    pub fn to_extractor_config(&self) -> boswell_extractor::ExtractorConfig {
        boswell_extractor::ExtractorConfig {
            max_text_length: self.max_text_length,
            ..boswell_extractor::ExtractorConfig::default()
        }
    }
}

/// Background maintenance settings for the in-process Janitor.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct JanitorSettings {
    /// Whether to run the decay-aware Janitor sweep loop in the background.
    pub enabled: bool,
    /// Minutes between sweep cycles.
    pub sweep_interval_minutes: u64,
    /// Dry-run: log intended deletions/demotions without applying them.
    pub dry_run: bool,
    /// Local-time window this job is allowed to run in, as `"HH:MM-HH:MM"`.
    ///
    /// Absent means any time. A window whose end precedes its start wraps
    /// midnight. The job's interval still governs how often it runs; the
    /// window governs when it may.
    pub run_between: Option<String>,
}

impl Default for JanitorSettings {
    fn default() -> Self {
        Self {
            enabled: false, // opt-in
            sweep_interval_minutes: 60,
            dry_run: false,
            run_between: None,
        }
    }
}

impl JanitorSettings {
    /// Build the [`boswell_janitor::JanitorConfig`] this settings block describes,
    /// keeping default TTLs and thresholds for everything not exposed here.
    pub fn to_janitor_config(&self) -> boswell_janitor::JanitorConfig {
        boswell_janitor::JanitorConfig {
            sweep_interval_minutes: self.sweep_interval_minutes,
            dry_run: self.dry_run,
            ..boswell_janitor::JanitorConfig::default()
        }
    }
}

/// Background synthesis settings for the in-process Synthesizer.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct SynthesizerSettings {
    /// Whether to run scheduled LLM-backed synthesis passes in the background.
    pub enabled: bool,
    /// Ollama chat model used for synthesis (distinct from the embedding model).
    pub model: String,
    /// Ollama endpoint for the synthesis model.
    pub endpoint: String,
    /// Hours between synthesis passes.
    pub interval_hours: u64,
    /// Minimum tier considered for synthesis (claims below this are skipped).
    pub min_tier: String,
    /// Dry-run: analyze and log insights without writing them to the store.
    pub dry_run: bool,
    /// Local-time window this job is allowed to run in, as `"HH:MM-HH:MM"`.
    ///
    /// Absent means any time. A window whose end precedes its start wraps
    /// midnight. The job's interval still governs how often it runs; the
    /// window governs when it may.
    pub run_between: Option<String>,
}

impl Default for SynthesizerSettings {
    fn default() -> Self {
        Self {
            enabled: false, // opt-in (LLM cost)
            model: "granite4.2:8b".to_string(),
            endpoint: "http://localhost:11434".to_string(),
            interval_hours: 6,
            min_tier: "task".to_string(),
            dry_run: false,
            run_between: None,
        }
    }
}

impl SynthesizerSettings {
    /// Build the [`boswell_synthesizer::SynthesizerConfig`] this settings block
    /// describes. `enabled` is set to `true` because the server only builds this
    /// when the settings-level `enabled` gate is already on.
    pub fn to_synthesizer_config(&self) -> boswell_synthesizer::SynthesizerConfig {
        boswell_synthesizer::SynthesizerConfig {
            enabled: true,
            synthesis_interval_hours: self.interval_hours,
            min_tier: self.min_tier.clone(),
            dry_run: self.dry_run,
            ..boswell_synthesizer::SynthesizerConfig::default()
        }
    }
}

/// Background contradiction-detection settings for the in-process janitor.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ContradictionSettings {
    /// Whether to run scheduled LLM-backed contradiction scans in the background.
    pub enabled: bool,
    /// Ollama chat model used for contradiction detection.
    pub model: String,
    /// Ollama endpoint for the contradiction model.
    pub endpoint: String,
    /// Hours between contradiction scans.
    pub interval_hours: u64,
    /// Minimum tier considered (claims below this are skipped).
    pub min_tier: String,
    /// Dry-run: detect and log contradictions without recording them.
    pub dry_run: bool,
    /// Local-time window this job is allowed to run in, as `"HH:MM-HH:MM"`.
    ///
    /// Absent means any time. A window whose end precedes its start wraps
    /// midnight. The job's interval still governs how often it runs; the
    /// window governs when it may.
    pub run_between: Option<String>,
}

impl Default for ContradictionSettings {
    fn default() -> Self {
        Self {
            enabled: false, // opt-in (LLM cost)
            model: "granite4.2:8b".to_string(),
            endpoint: "http://localhost:11434".to_string(),
            interval_hours: 12,
            min_tier: "task".to_string(),
            dry_run: false,
            run_between: None,
        }
    }
}

impl ContradictionSettings {
    /// Build the [`boswell_janitor::ContradictionConfig`] this settings block
    /// describes. `enabled` is set to `true` because the server only builds this
    /// when the settings-level `enabled` gate is already on.
    pub fn to_contradiction_config(&self) -> boswell_janitor::ContradictionConfig {
        boswell_janitor::ContradictionConfig {
            enabled: true,
            scan_interval_hours: self.interval_hours,
            min_tier: self.min_tier.clone(),
            dry_run: self.dry_run,
            ..boswell_janitor::ContradictionConfig::default()
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
        config.validate()?;
        Ok(config)
    }

    /// Check the fields TOML parsing cannot check for itself.
    ///
    /// Only `run_between` needs this today. It is a string to serde and a time
    /// window to the scheduler, and the gap between those two is where a typo
    /// lives.
    pub fn validate(&self) -> Result<(), ConfigError> {
        for (section, spec) in [
            ("janitor", self.janitor.run_between.as_deref()),
            ("synthesizer", self.synthesizer.run_between.as_deref()),
            ("contradiction", self.contradiction.run_between.as_deref()),
        ] {
            if let Some(spec) = spec {
                crate::schedule::RunWindow::parse(spec)
                    .map_err(|reason| ConfigError::InvalidRunWindow { section, reason })?;
            }
        }
        Ok(())
    }

    /// The parsed `run_between` window for a section, if it has one.
    ///
    /// Only called after [`Self::validate`] has passed, so an unparseable
    /// window here is a bug rather than bad input; it degrades to "no window"
    /// rather than panicking in a background task.
    fn window(spec: Option<&str>) -> Option<crate::schedule::RunWindow> {
        spec.and_then(|spec| crate::schedule::RunWindow::parse(spec).ok())
    }

    /// The Janitor's run window, if configured.
    pub fn janitor_window(&self) -> Option<crate::schedule::RunWindow> {
        Self::window(self.janitor.run_between.as_deref())
    }

    /// The Synthesizer's run window, if configured.
    pub fn synthesizer_window(&self) -> Option<crate::schedule::RunWindow> {
        Self::window(self.synthesizer.run_between.as_deref())
    }

    /// The Contradiction scan's run window, if configured.
    pub fn contradiction_window(&self) -> Option<crate::schedule::RunWindow> {
        Self::window(self.contradiction.run_between.as_deref())
    }

    /// A commented starter configuration, written by `boswell-server init`.
    pub fn starter_toml() -> &'static str {
        STARTER_TOML
    }
}

/// Commented starter config emitted by the `init` subcommand.
pub const STARTER_TOML: &str = r#"# Boswell instance server configuration

# Address and port the gRPC server binds to. Loopback only — the instance does not
# authenticate, so a routable address is refused at startup (ADR-021). Put
# boswell-gateway in front of it to reach memory from anywhere else.
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

[janitor]
# Run the decay-aware maintenance sweep (tier demotion + stale-claim GC) in the
# background. Off by default.
enabled = false
sweep_interval_minutes = 60
# Dry-run logs what would be deleted/demoted without changing anything.
dry_run = false
# Only run inside this local-time window. Commented out means any time. An end
# before the start wraps midnight ("22:00-06:00"). The interval above still says
# how often; this says when it is allowed to.
# run_between = "02:00-06:00"

[synthesizer]
# Run scheduled LLM-backed synthesis passes that discover higher-order insights
# across the claim graph. Off by default (LLM cost). Requires the chat model:
#   ollama pull granite4.2:8b
enabled = false
model = "granite4.2:8b"
endpoint = "http://localhost:11434"
interval_hours = 6
min_tier = "task"
# Dry-run analyzes and logs insights without writing them to the store.
dry_run = false
# Only run inside this local-time window — the LLM work is heavy, and this keeps
# it off the machine while you are using it.
# run_between = "02:00-06:00"

[contradiction]
# Run scheduled LLM-backed contradiction detection: compares same-subject claims
# and records a Contradicts relationship for incompatible pairs (which lowers the
# effective confidence of both). Off by default (LLM cost).
enabled = false
model = "granite4.2:8b"
endpoint = "http://localhost:11434"
interval_hours = 12
min_tier = "task"
# Dry-run detects and logs contradictions without recording them.
dry_run = false
# Only run inside this local-time window.
# run_between = "02:00-06:00"

[extraction]
# Server-side LLM extraction that turns text into claims, backing the gRPC
# Extract RPC and the gateway's /v1/extract and LLM-mode /v1/hooks/ingest.
# Off by default (LLM cost). Requires the chat model:
#   ollama pull granite4.2:8b
enabled = false
model = "granite4.2:8b"
endpoint = "http://localhost:11434"
max_text_length = 50000
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bad_run_window_stops_the_server_and_names_the_section() {
        let config: InstanceConfig = toml::from_str(
            r#"
            [synthesizer]
            run_between = "2am-6am"
            "#,
        )
        .unwrap();

        match config.validate() {
            Err(ConfigError::InvalidRunWindow { section, reason }) => {
                assert_eq!(section, "synthesizer");
                assert!(reason.contains("HH:MM"), "got: {reason}");
            }
            other => panic!("expected an InvalidRunWindow error, got {other:?}"),
        }
    }

    #[test]
    fn a_good_run_window_validates_and_parses_back() {
        let config: InstanceConfig = toml::from_str(
            r#"
            [contradiction]
            run_between = "22:00-06:00"
            "#,
        )
        .unwrap();

        assert!(config.validate().is_ok());
        assert_eq!(
            config.contradiction_window(),
            Some(crate::schedule::RunWindow::parse("22:00-06:00").unwrap())
        );
    }

    #[test]
    fn no_run_window_is_the_default_and_valid() {
        let config = InstanceConfig::default();

        assert!(config.validate().is_ok());
        assert_eq!(config.janitor_window(), None);
        assert_eq!(config.synthesizer_window(), None);
        assert_eq!(config.contradiction_window(), None);
    }

    #[test]
    fn the_starter_config_this_server_emits_is_one_it_accepts() {
        let config: InstanceConfig =
            toml::from_str(InstanceConfig::starter_toml()).expect("the starter config must parse");
        assert!(config.validate().is_ok());
    }

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
