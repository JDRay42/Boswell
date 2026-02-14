//! Configuration for Janitor operations
//!
//! Defines TTLs (Time-To-Live) per tier and sweep intervals.

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Configuration for the Janitor service
///
/// Controls tier-specific TTLs, sweep intervals, and operational modes.
///
/// # Examples
///
/// ```
/// use boswell_janitor::JanitorConfig;
///
/// // Default configuration (balanced)
/// let config = JanitorConfig::default();
/// assert_eq!(config.ephemeral_ttl_hours, 12);
///
/// // Aggressive cleanup
/// let config = JanitorConfig::aggressive();
/// assert_eq!(config.ephemeral_ttl_hours, 6);
///
/// // Lenient cleanup
/// let config = JanitorConfig::lenient();
/// assert_eq!(config.ephemeral_ttl_hours, 24);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JanitorConfig {
    /// TTL for Ephemeral tier claims (in hours)
    /// Default: 12 hours (session lifetime baseline per ADR-019)
    pub ephemeral_ttl_hours: u64,

    /// TTL for Task tier claims (in hours, if idle)
    /// Default: 24 hours of inactivity
    pub task_ttl_hours: u64,

    /// Staleness threshold for Project tier (in days)
    /// Default: 90 days of no access
    pub project_stale_days: u64,

    /// How often to run the sweep cycle (in minutes)
    /// Default: Every 60 minutes (hourly)
    pub sweep_interval_minutes: u64,

    /// Minimum confidence threshold for demotion consideration
    /// Claims below this confidence (lower bound) are candidates for demotion
    /// Default: 0.3
    pub demotion_confidence_threshold: f64,

    /// Access frequency threshold for promotion (accesses per week)
    /// Claims accessed more frequently are candidates for promotion
    /// Default: 7 (daily access)
    pub promotion_access_threshold: u64,

    /// Dry-run mode: Log what would be deleted without actually deleting
    /// Default: false
    #[serde(default)]
    pub dry_run: bool,

    /// Enable automatic tier promotion based on access patterns
    /// Default: true
    #[serde(default = "default_auto_promote")]
    pub auto_promote: bool,

    /// Enable automatic tier demotion based on staleness
    /// Default: true
    #[serde(default = "default_auto_demote")]
    pub auto_demote: bool,
}

fn default_auto_promote() -> bool {
    true
}

fn default_auto_demote() -> bool {
    true
}

impl Default for JanitorConfig {
    /// Create default configuration with balanced cleanup policies
    ///
    /// - Ephemeral: 12 hours (ADR-019 session lifetime)
    /// - Task: 24 hours of inactivity
    /// - Project: 90 days of no access
    /// - Sweep interval: 60 minutes (hourly)
    /// - Demotion threshold: 0.3 confidence
    /// - Promotion threshold: 7 accesses/week (daily)
    fn default() -> Self {
        Self {
            ephemeral_ttl_hours: 12,
            task_ttl_hours: 24,
            project_stale_days: 90,
            sweep_interval_minutes: 60,
            demotion_confidence_threshold: 0.3,
            promotion_access_threshold: 7,
            dry_run: false,
            auto_promote: true,
            auto_demote: true,
        }
    }
}

impl JanitorConfig {
    /// Aggressive cleanup configuration (shorter TTLs, frequent sweeps)
    ///
    /// Suitable for resource-constrained environments or when storage is at premium.
    ///
    /// - Ephemeral: 6 hours
    /// - Task: 12 hours
    /// - Project: 30 days
    /// - Sweep interval: 30 minutes
    pub fn aggressive() -> Self {
        Self {
            ephemeral_ttl_hours: 6,
            task_ttl_hours: 12,
            project_stale_days: 30,
            sweep_interval_minutes: 30,
            demotion_confidence_threshold: 0.4,
            promotion_access_threshold: 14, // Twice daily
            dry_run: false,
            auto_promote: true,
            auto_demote: true,
        }
    }

    /// Lenient cleanup configuration (longer TTLs, infrequent sweeps)
    ///
    /// Suitable for development or when you want to keep claims around longer.
    ///
    /// - Ephemeral: 24 hours
    /// - Task: 72 hours (3 days)
    /// - Project: 180 days (6 months)
    /// - Sweep interval: 120 minutes (2 hours)
    pub fn lenient() -> Self {
        Self {
            ephemeral_ttl_hours: 24,
            task_ttl_hours: 72,
            project_stale_days: 180,
            sweep_interval_minutes: 120,
            demotion_confidence_threshold: 0.2,
            promotion_access_threshold: 3, // Every other day
            dry_run: false,
            auto_promote: true,
            auto_demote: true,
        }
    }

    /// Get sweep interval as Duration
    pub fn sweep_interval(&self) -> Duration {
        Duration::from_secs(self.sweep_interval_minutes * 60)
    }

    /// Get ephemeral TTL as Duration
    pub fn ephemeral_ttl(&self) -> Duration {
        Duration::from_secs(self.ephemeral_ttl_hours * 3600)
    }

    /// Get task TTL as Duration
    pub fn task_ttl(&self) -> Duration {
        Duration::from_secs(self.task_ttl_hours * 3600)
    }

    /// Get project stale threshold as Duration
    pub fn project_stale_threshold(&self) -> Duration {
        Duration::from_secs(self.project_stale_days * 86400)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = JanitorConfig::default();
        assert_eq!(config.ephemeral_ttl_hours, 12);
        assert_eq!(config.task_ttl_hours, 24);
        assert_eq!(config.project_stale_days, 90);
        assert_eq!(config.sweep_interval_minutes, 60);
        assert_eq!(config.demotion_confidence_threshold, 0.3);
        assert_eq!(config.promotion_access_threshold, 7);
        assert!(!config.dry_run);
        assert!(config.auto_promote);
        assert!(config.auto_demote);
    }

    #[test]
    fn test_aggressive_config() {
        let config = JanitorConfig::aggressive();
        assert_eq!(config.ephemeral_ttl_hours, 6);
        assert_eq!(config.task_ttl_hours, 12);
        assert_eq!(config.project_stale_days, 30);
        assert_eq!(config.sweep_interval_minutes, 30);
        assert!(config.ephemeral_ttl_hours < JanitorConfig::default().ephemeral_ttl_hours);
    }

    #[test]
    fn test_lenient_config() {
        let config = JanitorConfig::lenient();
        assert_eq!(config.ephemeral_ttl_hours, 24);
        assert_eq!(config.task_ttl_hours, 72);
        assert_eq!(config.project_stale_days, 180);
        assert_eq!(config.sweep_interval_minutes, 120);
        assert!(config.ephemeral_ttl_hours > JanitorConfig::default().ephemeral_ttl_hours);
    }

    #[test]
    fn test_duration_conversions() {
        let config = JanitorConfig::default();
        
        assert_eq!(config.sweep_interval(), Duration::from_secs(60 * 60));
        assert_eq!(config.ephemeral_ttl(), Duration::from_secs(12 * 3600));
        assert_eq!(config.task_ttl(), Duration::from_secs(24 * 3600));
        assert_eq!(config.project_stale_threshold(), Duration::from_secs(90 * 86400));
    }

    #[test]
    fn test_serde_roundtrip() {
        let config = JanitorConfig::default();
        let serialized = serde_json::to_string(&config).unwrap();
        let deserialized: JanitorConfig = serde_json::from_str(&serialized).unwrap();
        
        assert_eq!(config.ephemeral_ttl_hours, deserialized.ephemeral_ttl_hours);
        assert_eq!(config.task_ttl_hours, deserialized.task_ttl_hours);
        assert_eq!(config.dry_run, deserialized.dry_run);
    }
}
