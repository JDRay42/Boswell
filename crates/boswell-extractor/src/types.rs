//! Request and response types for extraction

use boswell_domain::ClaimId;
use serde::{Deserialize, Serialize};

/// Request to extract claims from text
#[derive(Debug, Clone)]
pub struct ExtractionRequest {
    /// Text to extract claims from
    pub text: String,
    
    /// Target namespace for claims
    pub namespace: String,
    
    /// Tier for the extracted claims
    pub tier: String,
    
    /// Source identifier (hash or user-provided)
    pub source_id: String,
    
    /// Optional existing claims for deduplication hints
    pub existing_context: Option<Vec<ClaimSummary>>,
}

/// Summary of an existing claim for context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimSummary {
    /// Subject of the claim
    pub subject: String,
    
    /// Predicate/relationship
    pub predicate: String,
    
    /// Object of the claim
    pub object: String,
    
    /// Confidence interval
    pub confidence: (f64, f64),
}

/// Result of an extraction operation
#[derive(Debug, Clone)]
pub struct ExtractionResult {
    /// Claims that were newly created
    pub claims_created: Vec<ClaimResult>,
    
    /// Claims that corroborated existing claims
    pub claims_corroborated: Vec<ClaimResult>,
    
    /// Claims that failed to be created
    pub failures: Vec<ExtractionFailure>,
    
    /// Metadata about the extraction
    pub metadata: ExtractionMetadata,
}

/// Information about a successfully extracted claim
#[derive(Debug, Clone)]
pub struct ClaimResult {
    /// ID of the claim
    pub claim_id: ClaimId,
    
    /// Subject entity
    pub subject: String,
    
    /// Predicate/relationship
    pub predicate: String,
    
    /// Object entity or value
    pub object: String,
    
    /// Confidence interval
    pub confidence: (f64, f64),
    
    /// Original text that led to this claim
    pub raw_expression: String,
}

/// Information about a claim that failed to be extracted
#[derive(Debug, Clone)]
pub struct ExtractionFailure {
    /// Reason for failure
    pub reason: String,
    
    /// Text fragment that failed to parse
    pub raw_text: String,
}

/// Metadata about an extraction operation
#[derive(Debug, Clone)]
pub struct ExtractionMetadata {
    /// Source identifier
    pub source_id: String,
    
    /// Timestamp when extraction occurred
    pub timestamp: u64,
    
    /// Name of the LLM model used
    pub model_name: String,
    
    /// Total number of claims attempted
    pub total_claims_attempted: usize,
    
    /// Processing time in milliseconds
    pub processing_time_ms: u64,
}

/// Internal representation of a claim candidate from LLM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ClaimCandidate {
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub confidence_lower: f64,
    pub confidence_upper: f64,
    pub raw_expression: String,
}

impl ClaimCandidate {
    /// Validate that the candidate has all required fields
    pub fn validate(&self) -> Result<(), String> {
        if self.subject.is_empty() {
            return Err("subject is empty".to_string());
        }
        if self.predicate.is_empty() {
            return Err("predicate is empty".to_string());
        }
        if self.object.is_empty() {
            return Err("object is empty".to_string());
        }
        if self.raw_expression.is_empty() {
            return Err("raw_expression is empty".to_string());
        }
        if self.confidence_lower < 0.0 || self.confidence_lower > 1.0 {
            return Err(format!("confidence_lower {} out of range [0.0, 1.0]", self.confidence_lower));
        }
        if self.confidence_upper < 0.0 || self.confidence_upper > 1.0 {
            return Err(format!("confidence_upper {} out of range [0.0, 1.0]", self.confidence_upper));
        }
        if self.confidence_lower > self.confidence_upper {
            return Err(format!(
                "confidence_lower {} > confidence_upper {}",
                self.confidence_lower, self.confidence_upper
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_claim_candidate() {
        let candidate = ClaimCandidate {
            subject: "person:alice".to_string(),
            predicate: "works_at".to_string(),
            object: "company:acme".to_string(),
            confidence_lower: 0.8,
            confidence_upper: 0.95,
            raw_expression: "Alice works at Acme".to_string(),
        };
        assert!(candidate.validate().is_ok());
    }

    #[test]
    fn test_empty_subject() {
        let candidate = ClaimCandidate {
            subject: "".to_string(),
            predicate: "works_at".to_string(),
            object: "company:acme".to_string(),
            confidence_lower: 0.8,
            confidence_upper: 0.95,
            raw_expression: "Alice works at Acme".to_string(),
        };
        assert!(candidate.validate().is_err());
    }

    #[test]
    fn test_invalid_confidence_range() {
        let candidate = ClaimCandidate {
            subject: "person:alice".to_string(),
            predicate: "works_at".to_string(),
            object: "company:acme".to_string(),
            confidence_lower: 0.95,
            confidence_upper: 0.8,
            raw_expression: "Alice works at Acme".to_string(),
        };
        assert!(candidate.validate().is_err());
    }

    #[test]
    fn test_confidence_out_of_bounds() {
        let candidate = ClaimCandidate {
            subject: "person:alice".to_string(),
            predicate: "works_at".to_string(),
            object: "company:acme".to_string(),
            confidence_lower: -0.1,
            confidence_upper: 0.95,
            raw_expression: "Alice works at Acme".to_string(),
        };
        assert!(candidate.validate().is_err());
    }
}
