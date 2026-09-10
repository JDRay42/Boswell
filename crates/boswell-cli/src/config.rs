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

    /// Identity provider used by `boswell login` (ADR-022).
    ///
    /// Absent by default, and absent from a written config until someone fills
    /// it in: a CLI with no `[oidc]` section behaves exactly as it did before
    /// `login` existed, and `boswell login --issuer ... --client-id ...` works
    /// without one.
    ///
    /// It is deliberately *not* read from the gateway's config. The device
    /// grant runs between this CLI and the provider with the gateway not in it,
    /// and the CLI is usually not on the gateway's host, so there is nothing to
    /// read. The two must name the same issuer; nothing enforces that, and a
    /// mismatch shows up as a 401 on the first request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oidc: Option<LoginConfig>,
}

/// Identity-provider settings for `boswell login`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginConfig {
    /// Issuer URL. A trailing slash is ignored, as at the gateway.
    pub issuer: String,

    /// The OAuth client id registered with the provider for this CLI. Public
    /// clients have no secret, which is the whole reason the device grant
    /// exists — do not add one here.
    pub client_id: String,

    /// Scopes to request. Empty asks for the provider's default.
    #[serde(default)]
    pub scopes: Vec<String>,
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
    /// Directory holding the CLI configuration.
    ///
    /// Honors `BOSWELL_CONFIG_DIR` so the location can be redirected. Under
    /// `cfg(test)` it defaults to a process-unique temporary directory: the unit
    /// tests exercise the real save path, and must never write over a
    /// developer's own `~/.boswell/config.toml`.
    fn config_dir() -> Result<PathBuf> {
        if let Ok(dir) = std::env::var("BOSWELL_CONFIG_DIR") {
            return Ok(PathBuf::from(dir));
        }

        #[cfg(test)]
        {
            Ok(std::env::temp_dir().join(format!("boswell-cli-test-{}", std::process::id())))
        }

        #[cfg(not(test))]
        {
            let home = dirs::home_dir()
                .ok_or_else(|| CliError::Config("Could not find home directory".into()))?;
            Ok(home.join(".boswell"))
        }
    }

    /// Get the configuration file path.
    pub fn path() -> Result<PathBuf> {
        Ok(Self::config_dir()?.join("config.toml"))
    }

    /// Where `boswell login` stores the token it obtains.
    ///
    /// A separate file from `config.toml` on purpose: the config is a thing an
    /// operator edits, shares and checks into dotfiles, and the token is a
    /// bearer credential written `0600`. Putting the second inside the first
    /// invites the two habits to collide.
    pub fn token_path() -> Result<PathBuf> {
        Ok(Self::config_dir()?.join("token.json"))
    }

    /// Load configuration from file or create default.
    ///
    /// A file that exists but carries no profiles is treated as absent. Such a
    /// file parses as perfectly valid TOML (every field has a serde default),
    /// but leaves nothing to connect with, so it would otherwise surface much
    /// later as a baffling "Profile 'default' not found" on the first command.
    pub fn load() -> Result<Self> {
        let path = Self::path()?;

        if !path.exists() {
            return Ok(Self::default());
        }

        let contents = fs::read_to_string(&path)?;
        if contents.trim().is_empty() {
            return Ok(Self::default());
        }

        let config: Config = toml::from_str(&contents)?;
        if config.profiles.is_empty() {
            return Ok(Self::default());
        }

        Ok(config)
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
            return Err(CliError::Config(format!(
                "Profile '{}' does not exist",
                name
            )));
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
            oidc: None,
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

    /// The unit tests must never resolve configuration to the real home
    /// directory: `save()` there would overwrite a developer's own profiles.
    #[test]
    fn test_test_config_path_is_not_in_home() {
        let path = Config::path().unwrap();
        if let Some(home) = dirs::home_dir() {
            assert!(
                !path.starts_with(home.join(".boswell")),
                "tests must not write to ~/.boswell, got {}",
                path.display()
            );
        }
    }

    /// An empty config file (e.g. left by an interrupted or racing write) must
    /// fall back to the defaults rather than yielding a profile-less config.
    #[test]
    fn test_empty_config_file_falls_back_to_defaults() {
        let dir = std::env::temp_dir().join(format!("boswell-empty-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        fs::write(&path, "").unwrap();

        // Read it back the way `load` does.
        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.trim().is_empty());

        // A profile-less parse must not be accepted as usable configuration.
        let parsed: Config = toml::from_str("active_profile = \"default\"").unwrap();
        assert!(parsed.profiles.is_empty());
        assert!(Config::default().get_active_profile().is_ok());

        fs::remove_dir_all(&dir).ok();
    }
}
