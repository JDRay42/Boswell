//! Error types for the Synthesizer

use thiserror::Error;

/// Errors that can occur during a synthesis pass
#[derive(Error, Debug)]
pub enum SynthesizerError {
    /// LLM provider error
    #[error("LLM error: {0}")]
    Llm(String),

    /// Claim store error
    #[error("Store error: {0}")]
    Store(String),

    /// The LLM response could not be parsed into an insight
    #[error("Invalid insight format: {0}")]
    InvalidFormat(String),

    /// JSON parsing error
    #[error("JSON parse error: {0}")]
    JsonParse(String),

    /// Validation error from the Gatekeeper
    #[error("Validation error: {0}")]
    Validation(String),

    /// Configuration error
    #[error("Configuration error: {0}")]
    Config(String),

    /// Synthesis pass timed out
    #[error("Synthesis timeout")]
    Timeout,
}

impl From<serde_json::Error> for SynthesizerError {
    fn from(e: serde_json::Error) -> Self {
        SynthesizerError::JsonParse(e.to_string())
    }
}
