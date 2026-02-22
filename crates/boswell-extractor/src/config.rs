//! Configuration for the Extractor

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Text chunking strategy for large documents
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChunkStrategy {
    /// Split by paragraphs (double newlines)
    ByParagraph,
    /// Split by sections (markdown headers or numbered sections)
    BySection,
    /// Split by approximate token count
    ByTokenCount,
}

impl Default for ChunkStrategy {
    fn default() -> Self {
        ChunkStrategy::ByParagraph
    }
}

/// Configuration for the Extractor
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractorConfig {
    /// Maximum input text length (characters)
    pub max_text_length: usize,
    
    /// Maximum existing claims to include as deduplication context
    pub context_claims_limit: usize,
    
    /// Maximum time for a single extraction call (seconds)
    pub extraction_timeout_secs: u64,
    
    /// Text chunking strategy for large documents
    pub chunk_strategy: ChunkStrategy,
    
    /// Maximum chunk size (characters)
    pub max_chunk_size: usize,
}

impl ExtractorConfig {
    /// Get the extraction timeout as a Duration
    pub fn extraction_timeout(&self) -> Duration {
        Duration::from_secs(self.extraction_timeout_secs)
    }
    
    /// Validate the configuration
    pub fn validate(&self) -> Result<(), String> {
        if self.max_text_length == 0 {
            return Err("max_text_length must be greater than 0".to_string());
        }
        if self.max_chunk_size == 0 {
            return Err("max_chunk_size must be greater than 0".to_string());
        }
        if self.max_chunk_size > self.max_text_length {
            return Err("max_chunk_size cannot exceed max_text_length".to_string());
        }
        if self.extraction_timeout_secs == 0 {
            return Err("extraction_timeout_secs must be greater than 0".to_string());
        }
        Ok(())
    }
}

impl Default for ExtractorConfig {
    /// Default configuration with balanced settings
    fn default() -> Self {
        Self {
            max_text_length: 50_000,
            context_claims_limit: 20,
            extraction_timeout_secs: 120,
            chunk_strategy: ChunkStrategy::ByParagraph,
            max_chunk_size: 10_000,
        }
    }
}

impl ExtractorConfig {
    /// Aggressive preset: shorter timeouts, smaller chunks for faster processing
    pub fn aggressive() -> Self {
        Self {
            max_text_length: 20_000,
            context_claims_limit: 10,
            extraction_timeout_secs: 60,
            chunk_strategy: ChunkStrategy::ByParagraph,
            max_chunk_size: 5_000,
        }
    }
    
    /// Lenient preset: longer timeouts, larger chunks for better quality
    pub fn lenient() -> Self {
        Self {
            max_text_length: 100_000,
            context_claims_limit: 50,
            extraction_timeout_secs: 300,
            chunk_strategy: ChunkStrategy::BySection,
            max_chunk_size: 20_000,
        }
    }
    
    /// Load configuration from TOML string
    pub fn from_toml(toml_str: &str) -> Result<Self, String> {
        toml::from_str(toml_str)
            .map_err(|e| format!("Failed to parse TOML: {}", e))
    }
    
    /// Serialize configuration to TOML string
    pub fn to_toml(&self) -> Result<String, String> {
        toml::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize to TOML: {}", e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_is_valid() {
        let config = ExtractorConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_aggressive_config_is_valid() {
        let config = ExtractorConfig::aggressive();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_lenient_config_is_valid() {
        let config = ExtractorConfig::lenient();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_invalid_max_text_length() {
        let mut config = ExtractorConfig::default();
        config.max_text_length = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_invalid_chunk_size_too_large() {
        let mut config = ExtractorConfig::default();
        config.max_chunk_size = config.max_text_length + 1;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_toml_round_trip() {
        let config = ExtractorConfig::default();
        let toml_str = config.to_toml().unwrap();
        let parsed = ExtractorConfig::from_toml(&toml_str).unwrap();
        
        assert_eq!(config.max_text_length, parsed.max_text_length);
        assert_eq!(config.context_claims_limit, parsed.context_claims_limit);
        assert_eq!(config.extraction_timeout_secs, parsed.extraction_timeout_secs);
    }
}
