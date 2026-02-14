//! Configuration management for the CLI.

use crate::error::{CliError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

/// CLI configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Active profile name
    #[serde(default = "default_profile")]
    pub active_profile: String,

    /// Available profiles
    #[serde(default)]
    pub profiles: HashMap<String, Profile>,

    /// Global settings
    #[serde(default)]
    pub settings: Settings,
}

/// Connection profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    /// Router URL
    pub router_url: String,

    /// Instance ID
    pub instance_id: String,

    /// Optional namespace
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
}

/// Global CLI settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// Enable colored output
    #[serde(default = "default_true")]
    pub color: bool,

    /// Default output format
    #[serde(default = "default_format")]
    pub format: OutputFormat,

    /// Command history size
    #[serde(default = "default_history_size")]
    pub history_size: usize,
}

/// Output format.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    /// Table format
    Table,
    /// JSON format
    Json,
    /// Quiet (minimal) format
    Quiet,
}

impl Config {
    /// Get the configuration file path.
    pub fn path() -> Result<PathBuf> {
        let home = dirs::home_dir().ok_or_else(|| CliError::Config("Could not find home directory".into()))?;
        Ok(home.join(".boswell").join("config.toml"))
    }

    /// Load configuration from file or create default.
    pub fn load() -> Result<Self> {
        let path = Self::path()?;
        
        if path.exists() {
            let contents = fs::read_to_string(&path)?;
            let config: Config = toml::from_str(&contents)?;
            Ok(config)
        } else {
            Ok(Self::default())
        }
    }

    /// Save configuration to file.
    pub fn save(&self) -> Result<()> {
        let path = Self::path()?;
        
        // Create parent directory if it doesn't exist
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let contents = toml::to_string_pretty(self)
            .map_err(|e| CliError::Config(format!("Failed to serialize config: {}", e)))?;
        fs::write(&path, contents)?;
        Ok(())
    }

    /// Get the active profile.
    pub fn get_active_profile(&self) -> Result<&Profile> {
        self.profiles
            .get(&self.active_profile)
            .ok_or_else(|| CliError::Config(format!("Profile '{}' not found", self.active_profile)))
    }

    /// Add or update a profile.
    pub fn set_profile(&mut self, name: String, profile: Profile) {
        self.profiles.insert(name, profile);
    }

    /// Switch to a different profile.
    pub fn switch_profile(&mut self, name: String) -> Result<()> {
        if !self.profiles.contains_key(&name) {
            return Err(CliError::Config(format!("Profile '{}' does not exist", name)));
        }
        self.active_profile = name;
        Ok(())
    }
}

impl Default for Config {
    fn default() -> Self {
        let mut profiles = HashMap::new();
        profiles.insert(
            "default".to_string(),
            Profile {
                router_url: "http://localhost:8080".to_string(),
                instance_id: "default".to_string(),
                namespace: None,
            },
        );

        Self {
            active_profile: "default".to_string(),
            profiles,
            settings: Settings::default(),
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            color: true,
            format: OutputFormat::Table,
            history_size: 1000,
        }
    }
}

fn default_profile() -> String {
    "default".to_string()
}

fn default_true() -> bool {
    true
}

fn default_format() -> OutputFormat {
    OutputFormat::Table
}

fn default_history_size() -> usize {
    1000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.active_profile, "default");
        assert!(config.profiles.contains_key("default"));
        assert!(config.settings.color);
    }

    #[test]
    fn test_profile_management() {
        let mut config = Config::default();
        
        let profile = Profile {
            router_url: "http://example.com:8080".to_string(),
            instance_id: "test".to_string(),
            namespace: Some("test-ns".to_string()),
        };
        
        config.set_profile("test".to_string(), profile);
        assert!(config.profiles.contains_key("test"));
        
        config.switch_profile("test".to_string()).unwrap();
        assert_eq!(config.active_profile, "test");
    }

    #[test]
    fn test_switch_to_nonexistent_profile() {
        let mut config = Config::default();
        let result = config.switch_profile("nonexistent".to_string());
        assert!(result.is_err());
    }
}
