//! Configuration file parsing for the Router.
//!
//! Loads settings from TOML files including bind address, JWT secret,
//! token expiry, and registered instances.

use serde::Deserialize;
use std::path::Path;
use thiserror::Error;

/// Router configuration error
#[derive(Debug, Error)]
pub enum ConfigError {
    /// Failed to read config file
    #[error("Failed to read config file: {0}")]
    FileRead(#[from] std::io::Error),

    /// Failed to parse TOML
    #[error("Failed to parse config TOML: {0}")]
    TomlParse(#[from] toml::de::Error),

    /// Missing required field
    #[error("Missing required configuration field: {0}")]
    MissingField(String),
}

/// Router configuration loaded from TOML
#[derive(Debug, Clone, Deserialize)]
pub struct RouterConfig {
    /// Bind address (e.g., "127.0.0.1")
    pub bind_address: String,

    /// Bind port (e.g., 8080)
    pub bind_port: u16,

    /// JWT secret for signing tokens
    pub jwt_secret: String,

    /// Token expiry in seconds (default: 3600 = 1 hour)
    #[serde(default = "default_token_expiry")]
    pub token_expiry_secs: u64,

    /// Registered instances
    #[serde(default)]
    pub instances: Vec<InstanceConfig>,
}

/// Instance configuration
#[derive(Debug, Clone, Deserialize)]
pub struct InstanceConfig {
    /// Instance identifier (e.g., "default")
    pub id: String,

    /// gRPC endpoint (e.g., "http://localhost:50051")
    pub endpoint: String,

    /// Expertise profile (namespaces this instance handles)
    #[serde(default)]
    pub expertise: Vec<String>,
}

/// Default token expiry: 1 hour
fn default_token_expiry() -> u64 {
    3600
}

impl RouterConfig {
    /// Load configuration from a TOML file
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let contents = std::fs::read_to_string(path)?;
        let config: RouterConfig = toml::from_str(&contents)?;

        // Validate required fields
        if config.jwt_secret.is_empty() {
            return Err(ConfigError::MissingField("jwt_secret".to_string()));
        }

        Ok(config)
    }

    /// Create a default configuration for testing
    pub fn default_test_config() -> Self {
        RouterConfig {
            bind_address: "127.0.0.1".to_string(),
            bind_port: 8080,
            jwt_secret: "test-secret-key-do-not-use-in-production".to_string(),
            token_expiry_secs: 3600,
            instances: vec![InstanceConfig {
                id: "default".to_string(),
                endpoint: "http://localhost:50051".to_string(),
                expertise: vec!["*".to_string()],
            }],
        }
    }

    /// Get the full bind address (address:port)
    pub fn bind_addr(&self) -> String {
        format!("{}:{}", self.bind_address, self.bind_port)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = RouterConfig::default_test_config();
        assert_eq!(config.bind_address, "127.0.0.1");
        assert_eq!(config.bind_port, 8080);
        assert_eq!(config.token_expiry_secs, 3600);
        assert_eq!(config.instances.len(), 1);
        assert_eq!(config.instances[0].id, "default");
    }

    #[test]
    fn test_bind_addr() {
        let config = RouterConfig::default_test_config();
        assert_eq!(config.bind_addr(), "127.0.0.1:8080");
    }

    #[test]
    fn test_parse_toml() {
        let toml = r#"
            bind_address = "0.0.0.0"
            bind_port = 9000
            jwt_secret = "my-secret"
            token_expiry_secs = 7200

            [[instances]]
            id = "instance1"
            endpoint = "http://localhost:50051"
            expertise = ["domain1", "domain2"]

            [[instances]]
            id = "instance2"
            endpoint = "http://localhost:50052"
            expertise = ["domain3"]
        "#;

        let config: RouterConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.bind_address, "0.0.0.0");
        assert_eq!(config.bind_port, 9000);
        assert_eq!(config.jwt_secret, "my-secret");
        assert_eq!(config.token_expiry_secs, 7200);
        assert_eq!(config.instances.len(), 2);
        assert_eq!(config.instances[0].expertise, vec!["domain1", "domain2"]);
    }
}
