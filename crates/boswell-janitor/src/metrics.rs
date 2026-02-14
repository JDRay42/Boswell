//! Metrics collection for Janitor operations

use boswell_domain::Tier;
use std::collections::HashMap;

/// Metrics collected during Janitor operations
///
/// Tracks claims deleted, promoted, demoted per tier, and storage statistics.
#[derive(Debug, Clone, Default)]
pub struct JanitorMetrics {
    /// Claims deleted per tier
    pub deleted: HashMap<Tier, usize>,
    
    /// Claims promoted per tier (from → to)
    pub promoted: HashMap<Tier, usize>,
    
    /// Claims demoted per tier (from → to)
    pub demoted: HashMap<Tier, usize>,
    
    /// Total sweep iterations completed
    pub sweep_count: usize,
    
    /// Total runtime in seconds
    pub total_runtime_secs: u64,
}

impl JanitorMetrics {
    /// Create new empty metrics
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a claim deletion
    pub fn record_deletion(&mut self, tier: Tier, count: usize) {
        *self.deleted.entry(tier).or_insert(0) += count;
    }

    /// Record a tier promotion
    pub fn record_promotion(&mut self, from_tier: Tier) {
        *self.promoted.entry(from_tier).or_insert(0) += 1;
    }

    /// Record a tier demotion
    pub fn record_demotion(&mut self, from_tier: Tier) {
        *self.demoted.entry(from_tier).or_insert(0) += 1;
    }

    /// Record a sweep cycle completion
    pub fn record_sweep(&mut self) {
        self.sweep_count += 1;
    }

    /// Get total claims deleted across all tiers
    pub fn total_deleted(&self) -> usize {
        self.deleted.values().sum()
    }

    /// Get total promotions across all tiers
    pub fn total_promoted(&self) -> usize {
        self.promoted.values().sum()
    }

    /// Get total demotions across all tiers
    pub fn total_demoted(&self) -> usize {
        self.demoted.values().sum()
    }

    /// Reset all metrics
    pub fn reset(&mut self) {
        self.deleted.clear();
        self.promoted.clear();
        self.demoted.clear();
        self.sweep_count = 0;
        self.total_runtime_secs = 0;
    }

    /// Generate a summary report of metrics
    pub fn summary(&self) -> String {
        let mut lines = vec![
            format!("Janitor Metrics Summary"),
            format!("======================"),
            format!("Sweep cycles: {}", self.sweep_count),
            format!("Total runtime: {}s", self.total_runtime_secs),
            format!(""),
        ];

        if !self.deleted.is_empty() {
            lines.push(format!("Deletions by tier:"));
            for (tier, count) in &self.deleted {
                lines.push(format!("  {:?}: {}", tier, count));
            }
            lines.push(format!("  Total: {}", self.total_deleted()));
            lines.push(format!(""));
        }

        if !self.promoted.is_empty() {
            lines.push(format!("Promotions from tier:"));
            for (tier, count) in &self.promoted {
                lines.push(format!("  {:?}: {}", tier, count));
            }
            lines.push(format!("  Total: {}", self.total_promoted()));
            lines.push(format!(""));
        }

        if !self.demoted.is_empty() {
            lines.push(format!("Demotions from tier:"));
            for (tier, count) in &self.demoted {
                lines.push(format!("  {:?}: {}", tier, count));
            }
            lines.push(format!("  Total: {}", self.total_demoted()));
        }

        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_creation() {
        let metrics = JanitorMetrics::new();
        assert_eq!(metrics.total_deleted(), 0);
        assert_eq!(metrics.total_promoted(), 0);
        assert_eq!(metrics.total_demoted(), 0);
        assert_eq!(metrics.sweep_count, 0);
    }

    #[test]
    fn test_record_deletion() {
        let mut metrics = JanitorMetrics::new();
        metrics.record_deletion(Tier::Ephemeral, 5);
        metrics.record_deletion(Tier::Task, 3);
        metrics.record_deletion(Tier::Ephemeral, 2);

        assert_eq!(*metrics.deleted.get(&Tier::Ephemeral).unwrap(), 7);
        assert_eq!(*metrics.deleted.get(&Tier::Task).unwrap(), 3);
        assert_eq!(metrics.total_deleted(), 10);
    }

    #[test]
    fn test_record_promotion() {
        let mut metrics = JanitorMetrics::new();
        metrics.record_promotion(Tier::Ephemeral);
        metrics.record_promotion(Tier::Task);
        metrics.record_promotion(Tier::Ephemeral);

        assert_eq!(*metrics.promoted.get(&Tier::Ephemeral).unwrap(), 2);
        assert_eq!(*metrics.promoted.get(&Tier::Task).unwrap(), 1);
        assert_eq!(metrics.total_promoted(), 3);
    }

    #[test]
    fn test_record_demotion() {
        let mut metrics = JanitorMetrics::new();
        metrics.record_demotion(Tier::Project);
        metrics.record_demotion(Tier::Permanent);

        assert_eq!(*metrics.demoted.get(&Tier::Project).unwrap(), 1);
        assert_eq!(*metrics.demoted.get(&Tier::Permanent).unwrap(), 1);
        assert_eq!(metrics.total_demoted(), 2);
    }

    #[test]
    fn test_sweep_count() {
        let mut metrics = JanitorMetrics::new();
        metrics.record_sweep();
        metrics.record_sweep();
        metrics.record_sweep();

        assert_eq!(metrics.sweep_count, 3);
    }

    #[test]
    fn test_reset() {
        let mut metrics = JanitorMetrics::new();
        metrics.record_deletion(Tier::Ephemeral, 10);
        metrics.record_promotion(Tier::Task);
        metrics.record_sweep();

        metrics.reset();

        assert_eq!(metrics.total_deleted(), 0);
        assert_eq!(metrics.total_promoted(), 0);
        assert_eq!(metrics.sweep_count, 0);
    }

    #[test]
    fn test_summary() {
        let mut metrics = JanitorMetrics::new();
        metrics.record_deletion(Tier::Ephemeral, 5);
        metrics.record_promotion(Tier::Task);
        metrics.record_demotion(Tier::Project);
        metrics.record_sweep();
        metrics.total_runtime_secs = 120;

        let summary = metrics.summary();
        assert!(summary.contains("Sweep cycles: 1"));
        assert!(summary.contains("Total runtime: 120s"));
        assert!(summary.contains("Ephemeral: 5"));
        assert!(summary.contains("Task: 1"));
        assert!(summary.contains("Project: 1"));
    }
}
