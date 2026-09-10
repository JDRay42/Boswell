//! Error types for Janitor operations

use thiserror::Error;

/// Errors that can occur during Janitor operations
#[derive(Error, Debug)]
pub enum JanitorError {
    /// Storage layer error
    #[error("Storage error: {0}")]
    Store(String),

    /// A provenance-stamped operation was refused because the principal's
    /// authority does not permit it — the endorse op or the namespace is out of
    /// scope (design §5, §6). Kept distinct from [`JanitorError::Store`] so a
    /// caller can answer "you may not" rather than "the store broke".
    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    /// Configuration error
    #[error("Configuration error: {0}")]
    Config(String),

    /// Invalid tier transition
    #[error("Invalid tier transition: {0}")]
    InvalidTransition(String),

    /// Worker error (tokio runtime issues)
    #[error("Worker error: {0}")]
    Worker(String),

    /// LLM provider error (contradiction detection)
    #[error("LLM error: {0}")]
    Llm(String),

    /// LLM call timed out
    #[error("LLM call timed out")]
    Timeout,
}
