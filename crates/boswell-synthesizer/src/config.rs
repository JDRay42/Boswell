//! Configuration for the Synthesizer

use crate::error::SynthesizerError;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Configuration controlling how the Synthesizer discovers and creates insights.
///
/// Presets are available via [`SynthesizerConfig::default`],
/// [`SynthesizerConfig::aggressive`], and [`SynthesizerConfig::conservative`].
///
/// See `docs/architecture/06-synthesizer.md` for the design rationale behind
/// each setting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SynthesizerConfig {
    /// Whether synthesis is enabled at all. When `false`, a pass is a no-op.
    pub enabled: bool,

    /// How often the background worker runs a synthesis pass, in hours.
    pub synthesis_interval_hours: u64,

    /// Minimum tier to consider for synthesis. Claims below this tier are
    /// treated as noise and skipped (default: `task`, skipping `ephemeral`).
    pub min_tier: String,

    /// Maximum number of clusters evaluated per pass. Caps LLM cost.
    pub max_clusters_per_pass: usize,

    /// Minimum number of claims a cluster must contain to be worth synthesizing.
    pub min_cluster_size: usize,

    /// Maximum cluster size handed to the LLM in a single analysis. Larger
    /// clusters are truncated to keep prompts bounded.
    pub max_cluster_size: usize,

    /// Maximum depth of `derived_from` chains. Prevents runaway meta-synthesis
    /// (per the "runaway synthesis" consideration in the architecture doc).
    pub max_derivation_depth: usize,

    /// Insights whose assessed upper confidence falls below this threshold are
    /// discarded ("quality over quantity").
    pub min_insight_confidence: f64,

    /// When `true`, the pass performs all analysis but writes nothing to the
    /// store. Useful for testing and cost estimation.
    pub dry_run: bool,

    /// Per-cluster LLM call timeout, in seconds.
    pub cluster_timeout_secs: u64,
}

impl Default for SynthesizerConfig {
    /// Balanced defaults suitable for most instances.
    fn default() -> Self {
        Self {
            enabled: true,
            synthesis_interval_hours: 6,
            min_tier: "task".to_string(),
            max_clusters_per_pass: 50,
            min_cluster_size: 3,
            max_cluster_size: 12,
            max_derivation_depth: 5,
            min_insight_confidence: 0.5,
            dry_run: false,
            cluster_timeout_secs: 60,
        }
    }
}

impl SynthesizerConfig {
    /// More frequent, wider passes. Suitable for cheap/local models where LLM
    /// cost is not a concern and fresh insights are valued.
    pub fn aggressive() -> Self {
        Self {
            synthesis_interval_hours: 1,
            min_tier: "ephemeral".to_string(),
            max_clusters_per_pass: 200,
            min_cluster_size: 2,
            max_cluster_size: 20,
            min_insight_confidence: 0.35,
            ..Default::default()
        }
    }

    /// Infrequent, high-bar passes. Suitable for expensive frontier models
    /// where fewer, higher-quality insights are preferred.
    pub fn conservative() -> Self {
        Self {
            synthesis_interval_hours: 24,
            min_tier: "project".to_string(),
            max_clusters_per_pass: 20,
            min_cluster_size: 4,
            max_cluster_size: 10,
            min_insight_confidence: 0.7,
            ..Default::default()
        }
    }

    /// Parse a [`SynthesizerConfig`] from a TOML string.
    ///
    /// The TOML may either place the fields at the top level or under a
    /// `[synthesizer]` table.
    pub fn from_toml(toml_str: &str) -> Result<Self, SynthesizerError> {
        // Support both `[synthesizer]`-wrapped and bare tables.
        #[derive(Deserialize)]
        struct Wrapper {
            synthesizer: Option<SynthesizerConfig>,
        }

        if let Ok(Wrapper {
            synthesizer: Some(cfg),
        }) = toml::from_str::<Wrapper>(toml_str)
        {
            return cfg.validate();
        }

        toml::from_str::<SynthesizerConfig>(toml_str)
            .map_err(|e| SynthesizerError::Config(e.to_string()))
            .and_then(|cfg| cfg.validate())
    }

    /// Validate internal consistency of the configuration.
    pub fn validate(self) -> Result<Self, SynthesizerError> {
        if self.min_cluster_size == 0 {
            return Err(SynthesizerError::Config(
                "min_cluster_size must be at least 1".to_string(),
            ));
        }
        if self.max_cluster_size < self.min_cluster_size {
            return Err(SynthesizerError::Config(
                "max_cluster_size must be >= min_cluster_size".to_string(),
            ));
        }
        if !(0.0..=1.0).contains(&self.min_insight_confidence) {
            return Err(SynthesizerError::Config(
                "min_insight_confidence must be in [0.0, 1.0]".to_string(),
            ));
        }
        if self.max_derivation_depth == 0 {
            return Err(SynthesizerError::Config(
                "max_derivation_depth must be at least 1".to_string(),
            ));
        }
        Ok(self)
    }

    /// The synthesis interval as a [`Duration`].
    pub fn synthesis_interval(&self) -> Duration {
        // Guard against a zero interval (tokio's `interval` panics on zero).
        let hours = self.synthesis_interval_hours.max(1);
        Duration::from_secs(hours * 3600)
    }

    /// The per-cluster LLM timeout as a [`Duration`].
    pub fn cluster_timeout(&self) -> Duration {
        Duration::from_secs(self.cluster_timeout_secs.max(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_is_valid() {
        assert!(SynthesizerConfig::default().validate().is_ok());
    }

    #[test]
    fn test_presets_are_valid() {
        assert!(SynthesizerConfig::aggressive().validate().is_ok());
        assert!(SynthesizerConfig::conservative().validate().is_ok());
    }

    #[test]
    fn test_invalid_cluster_bounds() {
        let cfg = SynthesizerConfig {
            min_cluster_size: 5,
            max_cluster_size: 3,
            ..Default::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn test_invalid_confidence() {
        let cfg = SynthesizerConfig {
            min_insight_confidence: 1.5,
            ..Default::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn test_from_toml_wrapped() {
        let toml = r#"
            [synthesizer]
            enabled = true
            min_tier = "project"
            max_clusters_per_pass = 10
            min_cluster_size = 3
        "#;
        let cfg = SynthesizerConfig::from_toml(toml).unwrap();
        assert_eq!(cfg.min_tier, "project");
        assert_eq!(cfg.max_clusters_per_pass, 10);
    }

    #[test]
    fn test_from_toml_bare() {
        let toml = r#"
            min_tier = "permanent"
            min_cluster_size = 2
        "#;
        let cfg = SynthesizerConfig::from_toml(toml).unwrap();
        assert_eq!(cfg.min_tier, "permanent");
        assert_eq!(cfg.min_cluster_size, 2);
    }

    #[test]
    fn test_interval_never_zero() {
        let cfg = SynthesizerConfig {
            synthesis_interval_hours: 0,
            ..Default::default()
        };
        assert_eq!(cfg.synthesis_interval(), Duration::from_secs(3600));
    }
}
