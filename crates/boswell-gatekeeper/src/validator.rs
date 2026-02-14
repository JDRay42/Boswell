//! Claim validation logic

use boswell_domain::{Claim, ClaimId, Tier};
use boswell_domain::traits::{ClaimStore, ClaimQuery};
use crate::{GatekeeperError, ValidationConfig};

/// Result of claim validation
#[derive(Debug, Clone)]
pub struct ValidationResult {
    /// Whether the claim passed validation
    pub status: ValidationStatus,
    
    /// Rejection reasons (if any)
    pub reasons: Vec<RejectionReason>,
    
    /// Quality score (0.0-1.0)
    pub quality_score: f64,
}

/// Validation status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationStatus {
    /// Claim accepted
    Accepted,
    
    /// Claim rejected
    Rejected,
    
    /// Validation deferred (transient issues)
    Deferred,
}

/// Reasons for rejection
#[derive(Debug, Clone, PartialEq)]
pub enum RejectionReason {
    /// Invalid entity format (expected namespace:value)
    InvalidEntityFormat(String),
    
    /// Invalid confidence bounds
    InvalidConfidenceBounds {
        /// Lower bound
        lower: String,
        /// Upper bound
        upper: String,
        /// Description of the issue
        issue: String,
    },
    
    /// Duplicate claim detected
    Duplicate {
        /// ID of the existing claim
        existing_id: ClaimId,
    },
    
    /// Tier confidence requirement not met
    TierConfidenceRequirement {
        /// Required tier
        tier: String,
        /// Minimum confidence required
        required: f64,
        /// Actual confidence
        actual: f64,
    },
    
    /// Semantic duplicate detected
    SemanticDuplicate {
        /// ID of similar existing claim
        existing_id: ClaimId,
        /// Similarity score
        similarity: f64,
    },
}

/// The Gatekeeper validates claims before storage
pub struct Gatekeeper {
    config: ValidationConfig,
}

impl Gatekeeper {
    /// Create a new Gatekeeper with the given configuration
    pub fn new(config: ValidationConfig) -> Self {
        Self { config }
    }

    /// Create a Gatekeeper with default configuration
    pub fn default_config() -> Self {
        Self::new(ValidationConfig::default())
    }

    /// Validate a claim against the configured rules
    ///
    /// # Arguments
    ///
    /// * `claim` - The claim to validate
    /// * `store` - The claim store for duplicate detection (optional)
    ///
    /// # Returns
    ///
    /// A validation result indicating whether the claim was accepted or rejected
    pub fn validate<S: ClaimStore>(
        &self,
        claim: &Claim,
        store: Option<&S>,
    ) -> Result<ValidationResult, GatekeeperError>
    where
        S::Error: std::fmt::Display,
    {
        let mut reasons = Vec::new();
        let mut quality_score: f64 = 1.0;

        // 1. Entity format validation
        if self.config.validate_entity_format {
            if let Some(reason) = self.validate_entity_format(claim) {
                reasons.push(reason);
                quality_score -= 0.3;
            }
        }

        // 2. Confidence bounds validation
        if self.config.validate_confidence_bounds {
            if let Some(reason) = self.validate_confidence_bounds(claim) {
                reasons.push(reason);
                quality_score -= 0.4;
            }
        }

        // 3. Tier appropriateness
        if self.config.validate_tier_appropriateness {
            if let Some(reason) = self.validate_tier_confidence(claim) {
                reasons.push(reason);
                quality_score -= 0.2;
            }
        }

        // 4. Duplicate detection (if store available)
        if self.config.validate_duplicates {
            if let Some(store) = store {
                if let Some(reason) = self.check_duplicates(claim, store)? {
                    reasons.push(reason);
                    quality_score -= 0.5;
                }
            }
        }

        // Determine status
        let status = if reasons.is_empty() {
            ValidationStatus::Accepted
        } else {
            ValidationStatus::Rejected
        };

        // Ensure quality score is non-negative
        quality_score = quality_score.max(0.0);

        Ok(ValidationResult {
            status,
            reasons,
            quality_score,
        })
    }

    /// Validate entity format (namespace:value)
    fn validate_entity_format(&self, claim: &Claim) -> Option<RejectionReason> {
        let entities = [
            ("subject", &claim.subject),
            ("predicate", &claim.predicate),
            ("object", &claim.object),
        ];

        for (name, entity) in &entities {
            if !entity.contains(':') {
                return Some(RejectionReason::InvalidEntityFormat(
                    format!("{} '{}' does not match namespace:value format", name, entity)
                ));
            }

            // Check for valid namespace and value parts
            let parts: Vec<&str> = entity.split(':').collect();
            if parts.len() < 2 || parts[0].is_empty() || parts[1].is_empty() {
                return Some(RejectionReason::InvalidEntityFormat(
                    format!("{} '{}' has invalid namespace or value", name, entity)
                ));
            }
        }

        None
    }

    /// Validate confidence bounds (0.0 ≤ low < high ≤ 1.0)
    fn validate_confidence_bounds(&self, claim: &Claim) -> Option<RejectionReason> {
        let (lower, upper) = claim.confidence;

        // Check bounds
        if lower < 0.0 || lower > 1.0 {
            return Some(RejectionReason::InvalidConfidenceBounds {
                lower: lower.to_string(),
                upper: upper.to_string(),
                issue: format!("Lower bound {} is outside [0.0, 1.0]", lower),
            });
        }

        if upper < 0.0 || upper > 1.0 {
            return Some(RejectionReason::InvalidConfidenceBounds {
                lower: lower.to_string(),
                upper: upper.to_string(),
                issue: format!("Upper bound {} is outside [0.0, 1.0]", upper),
            });
        }

        // Check ordering
        if lower >= upper {
            return Some(RejectionReason::InvalidConfidenceBounds {
                lower: lower.to_string(),
                upper: upper.to_string(),
                issue: format!("Lower bound {} must be less than upper bound {}", lower, upper),
            });
        }

        None
    }

    /// Validate tier confidence requirements
    fn validate_tier_confidence(&self, claim: &Claim) -> Option<RejectionReason> {
        let tier = Tier::parse(&claim.tier)?;
        let (lower, _upper) = claim.confidence;

        let required = match tier {
            Tier::Ephemeral => self.config.ephemeral_min_confidence,
            Tier::Task => self.config.task_min_confidence,
            Tier::Project => self.config.project_min_confidence,
            Tier::Permanent => self.config.permanent_min_confidence,
        };

        if lower < required {
            return Some(RejectionReason::TierConfidenceRequirement {
                tier: claim.tier.clone(),
                required,
                actual: lower,
            });
        }

        None
    }

    /// Check for duplicate claims
    fn check_duplicates<S: ClaimStore>(
        &self,
        claim: &Claim,
        store: &S,
    ) -> Result<Option<RejectionReason>, GatekeeperError>
    where
        S::Error: std::fmt::Display,
    {
        // Query for claims with the same subject, predicate, and object
        let query = ClaimQuery {
            namespace: Some(claim.namespace.clone()),
            tier: Some(claim.tier.clone()),
            min_confidence: None,
            semantic_text: None,
            limit: Some(100), // Check up to 100 existing claims
        };

        let existing_claims = store.query_claims(&query)
            .map_err(|e| GatekeeperError::Store(format!("Failed to query claims: {}", e)))?;

        // Check for exact matches
        for existing in existing_claims {
            if existing.subject == claim.subject
                && existing.predicate == claim.predicate
                && existing.object == claim.object
            {
                return Ok(Some(RejectionReason::Duplicate {
                    existing_id: existing.id,
                }));
            }
        }

        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use boswell_domain::ClaimId;

    fn create_test_claim() -> Claim {
        Claim {
            id: ClaimId::new(),
            namespace: "test".to_string(),
            subject: "user:alice".to_string(),
            predicate: "likes:coffee".to_string(),
            object: "beverage:espresso".to_string(),
            confidence: (0.8, 0.9),
            tier: "task".to_string(),
            created_at: 1234567890,
            stale_at: None,
        }
    }

    #[test]
    fn test_valid_claim() {
        let gatekeeper = Gatekeeper::default_config();
        let claim = create_test_claim();
        let result = gatekeeper.validate::<MockStore>(&claim, None).unwrap();

        assert_eq!(result.status, ValidationStatus::Accepted);
        assert!(result.reasons.is_empty());
        assert!(result.quality_score > 0.9);
    }

    #[test]
    fn test_invalid_entity_format() {
        let gatekeeper = Gatekeeper::default_config();
        let mut claim = create_test_claim();
        claim.subject = "alice".to_string(); // Missing namespace

        let result = gatekeeper.validate::<MockStore>(&claim, None).unwrap();

        assert_eq!(result.status, ValidationStatus::Rejected);
        assert_eq!(result.reasons.len(), 1);
        match &result.reasons[0] {
            RejectionReason::InvalidEntityFormat(msg) => {
                assert!(msg.contains("subject"));
            }
            _ => panic!("Expected InvalidEntityFormat"),
        }
    }

    #[test]
    fn test_invalid_confidence_bounds_out_of_range() {
        let gatekeeper = Gatekeeper::default_config();
        let mut claim = create_test_claim();
        claim.confidence = (1.5, 2.0); // Out of range

        let result = gatekeeper.validate::<MockStore>(&claim, None).unwrap();

        assert_eq!(result.status, ValidationStatus::Rejected);
        assert_eq!(result.reasons.len(), 1);
        match &result.reasons[0] {
            RejectionReason::InvalidConfidenceBounds { .. } => {}
            _ => panic!("Expected InvalidConfidenceBounds"),
        }
    }

    #[test]
    fn test_invalid_confidence_bounds_ordering() {
        let gatekeeper = Gatekeeper::default_config();
        let mut claim = create_test_claim();
        claim.confidence = (0.9, 0.8); // Lower >= Upper

        let result = gatekeeper.validate::<MockStore>(&claim, None).unwrap();

        assert_eq!(result.status, ValidationStatus::Rejected);
        match &result.reasons[0] {
            RejectionReason::InvalidConfidenceBounds { issue, .. } => {
                assert!(issue.contains("must be less than"));
            }
            _ => panic!("Expected InvalidConfidenceBounds"),
        }
    }

    #[test]
    fn test_tier_confidence_requirement() {
        let gatekeeper = Gatekeeper::default_config();
        let mut claim = create_test_claim();
        claim.tier = "permanent".to_string();
        claim.confidence = (0.5, 0.6); // Too low for permanent

        let result = gatekeeper.validate::<MockStore>(&claim, None).unwrap();

        assert_eq!(result.status, ValidationStatus::Rejected);
        match &result.reasons[0] {
            RejectionReason::TierConfidenceRequirement { required, actual, .. } => {
                assert_eq!(*required, 0.8);
                assert_eq!(*actual, 0.5);
            }
            _ => panic!("Expected TierConfidenceRequirement"),
        }
    }

    #[test]
    fn test_permissive_config() {
        let config = ValidationConfig::permissive();
        let gatekeeper = Gatekeeper::new(config);
        let mut claim = create_test_claim();
        claim.confidence = (0.1, 0.2); // Low confidence

        let result = gatekeeper.validate::<MockStore>(&claim, None).unwrap();

        // Should pass with permissive config
        assert_eq!(result.status, ValidationStatus::Accepted);
    }

    #[test]
    fn test_multiple_validation_errors() {
        let gatekeeper = Gatekeeper::default_config();
        let mut claim = create_test_claim();
        claim.subject = "alice".to_string(); // Invalid format
        claim.confidence = (0.9, 0.8); // Invalid ordering

        let result = gatekeeper.validate::<MockStore>(&claim, None).unwrap();

        assert_eq!(result.status, ValidationStatus::Rejected);
        assert_eq!(result.reasons.len(), 2);
    }

    // Mock store for testing (no actual storage)
    struct MockStore;
    
    impl ClaimStore for MockStore {
        type Error = String;

        fn assert_claim(&mut self, _claim: Claim) -> Result<ClaimId, Self::Error> {
            Ok(ClaimId::new())
        }

        fn get_claim(&self, _id: ClaimId) -> Result<Option<Claim>, Self::Error> {
            Ok(None)
        }

        fn query_claims(&self, _query: &ClaimQuery) -> Result<Vec<Claim>, Self::Error> {
            Ok(vec![])
        }

        fn add_relationship(&mut self, _relationship: boswell_domain::Relationship) -> Result<(), Self::Error> {
            Ok(())
        }

        fn get_relationships(&self, _id: ClaimId) -> Result<Vec<boswell_domain::Relationship>, Self::Error> {
            Ok(vec![])
        }
    }
}
