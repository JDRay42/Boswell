//! Gatekeeper error types

use thiserror::Error;

/// Errors that can occur during gatekeeper operations
#[derive(Error, Debug)]
pub enum GatekeeperError {
    /// Store error during validation
    #[error("Store error: {0}")]
    Store(String),
    
    /// LLM provider error
    #[error("LLM error: {0}")]
    Llm(String),
    
    /// Configuration error
    #[error("Configuration error: {0}")]
    Config(String),
    
    /// Validation error (internal)
    #[error("Validation error: {0}")]
    Validation(String),
}
