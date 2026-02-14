//! Error types for the CLI application.

use thiserror::Error;

/// Result type alias for CLI operations.
pub type Result<T> = std::result::Result<T, CliError>;

/// CLI-specific errors.
#[derive(Debug, Error)]
pub enum CliError {
    /// Configuration error
    #[error("Configuration error: {0}")]
    Config(String),

    /// Connection error
    #[error("Connection error: {0}")]
    Connection(String),

    /// SDK error
    #[error("SDK error: {0}")]
    Sdk(#[from] boswell_sdk::SdkError),

    /// I/O error
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Serialization error
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// TOML parsing error
    #[error("TOML parsing error: {0}")]
    Toml(#[from] toml::de::Error),

    /// Invalid input
    #[error("Invalid input: {0}")]
    InvalidInput(String),

    /// Operation not permitted
    #[error("Operation not permitted: {0}")]
    NotPermitted(String),

    /// No active connection
    #[error("No active connection. Use 'connect' command first.")]
    NotConnected,
}
