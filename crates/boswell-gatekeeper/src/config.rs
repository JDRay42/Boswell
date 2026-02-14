//! Gatekeeper configuration

/// Configuration for validation rules
#[derive(Debug, Clone)]
pub struct ValidationConfig {
    /// Enable entity format validation (namespace:value)
    pub validate_entity_format: bool,
    
    /// Enable confidence bounds checking
    pub validate_confidence_bounds: bool,
    
    /// Enable duplicate detection (exact match)
    pub validate_duplicates: bool,
    
    /// Enable semantic duplicate detection (requires vector search)
    pub validate_semantic_duplicates: bool,
    
    /// Similarity threshold for semantic duplicate detection (0.0-1.0)
    pub semantic_duplicate_threshold: f64,
    
    /// Enable tier appropriateness checking
    pub validate_tier_appropriateness: bool,
    
    /// Minimum confidence lower bound for ephemeral tier
    pub ephemeral_min_confidence: f64,
    
    /// Minimum confidence lower bound for task tier
    pub task_min_confidence: f64,
    
    /// Minimum confidence lower bound for project tier
    pub project_min_confidence: f64,
    
    /// Minimum confidence lower bound for permanent tier
    pub permanent_min_confidence: f64,
}

impl Default for ValidationConfig {
    fn default() -> Self {
        Self {
            validate_entity_format: true,
            validate_confidence_bounds: true,
            validate_duplicates: true,
            validate_semantic_duplicates: false,
            semantic_duplicate_threshold: 0.95,
            validate_tier_appropriateness: true,
            ephemeral_min_confidence: 0.0,
            task_min_confidence: 0.4,
            project_min_confidence: 0.6,
            permanent_min_confidence: 0.8,
        }
    }
}

impl ValidationConfig {
    /// Create a permissive configuration (minimal validation)
    pub fn permissive() -> Self {
        Self {
            validate_entity_format: true,
            validate_confidence_bounds: true,
            validate_duplicates: false,
            validate_semantic_duplicates: false,
            semantic_duplicate_threshold: 0.99,
            validate_tier_appropriateness: false,
            ephemeral_min_confidence: 0.0,
            task_min_confidence: 0.0,
            project_min_confidence: 0.0,
            permanent_min_confidence: 0.0,
        }
    }
    
    /// Create a strict configuration (all validations enabled)
    pub fn strict() -> Self {
        Self {
            validate_entity_format: true,
            validate_confidence_bounds: true,
            validate_duplicates: true,
            validate_semantic_duplicates: true,
            semantic_duplicate_threshold: 0.90,
            validate_tier_appropriateness: true,
            ephemeral_min_confidence: 0.0,
            task_min_confidence: 0.5,
            project_min_confidence: 0.7,
            permanent_min_confidence: 0.85,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = ValidationConfig::default();
        assert!(config.validate_entity_format);
        assert!(config.validate_confidence_bounds);
        assert!(config.validate_duplicates);
    }

    #[test]
    fn test_permissive_config() {
        let config = ValidationConfig::permissive();
        assert!(!config.validate_duplicates);
        assert_eq!(config.ephemeral_min_confidence, 0.0);
    }

    #[test]
    fn test_strict_config() {
        let config = ValidationConfig::strict();
        assert!(config.validate_semantic_duplicates);
        assert_eq!(config.permanent_min_confidence, 0.85);
    }
}
