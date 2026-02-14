//! Error types for Janitor operations

use thiserror::Error;

/// Errors that can occur during Janitor operations
#[derive(Error, Debug)]
pub enum JanitorError {
    /// Storage layer error
    #[error("Storage error: {0}")]
    Store(String),

    /// Configuration error
    #[error("Configuration error: {0}")]
    Config(String),

    /// Invalid tier transition
    #[error("Invalid tier transition: {0}")]
    InvalidTransition(String),

    /// Worker error (tokio runtime issues)
    #[error("Worker error: {0}")]
    Worker(String),
}
