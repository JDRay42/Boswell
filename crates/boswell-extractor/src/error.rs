//! Error types for the Extractor

use thiserror::Error;

/// Errors that can occur during extraction
#[derive(Error, Debug)]
pub enum ExtractorError {
    /// LLM provider error
    #[error("LLM error: {0}")]
    Llm(String),
    
    /// Claim store error
    #[error("Store error: {0}")]
    Store(String),
    
    /// Text exceeds maximum length
    #[error("Text too long: {0} chars (max: {1})")]
    TextTooLong(usize, usize),
    
    /// Extraction timeout
    #[error("Extraction timeout")]
    Timeout,
    
    /// Invalid claim format in LLM response
    #[error("Invalid claim format: {0}")]
    InvalidFormat(String),
    
    /// Validation error from Gatekeeper
    #[error("Validation error: {0}")]
    Validation(String),
    
    /// JSON parsing error
    #[error("JSON parse error: {0}")]
    JsonParse(String),
    
    /// Configuration error
    #[error("Configuration error: {0}")]
    Config(String),
}

impl From<serde_json::Error> for ExtractorError {
    fn from(e: serde_json::Error) -> Self {
        ExtractorError::JsonParse(e.to_string())
    }
}
