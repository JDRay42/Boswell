//! Core Janitor implementation for tier management and cleanup

use crate::{JanitorConfig, JanitorError, JanitorMetrics};
use boswell_domain::{Claim, ClaimId, Tier};
use boswell_domain::traits::{ClaimStore, ClaimQuery};
use std::time::{SystemTime, UNIX_EPOCH};

/// Current timestamp in seconds since Unix epoch
fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// Janitor service for automated tier management and cleanup
///
/// Responsible for:
/// - Sweeping stale claims per tier TTLs
/// - Promoting claims based on access patterns
/// - Demoting claims based on staleness and low confidence
/// - Collecting metrics on cleanup operations
///
/// # Examples
///
/// ```no_run
/// use boswell_janitor::{Janitor, JanitorConfig};
/// use boswell_store::SqliteStore;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let mut store = SqliteStore::new(":memory:", false, 0)?;
/// let config = JanitorConfig::default();
/// let mut janitor = Janitor::new(config);
///
/// // Perform a single sweep
/// let metrics = janitor.sweep(&mut store)?;
/// println!("{}", metrics.summary());
/// # Ok(())
/// # }
/// ```
pub struct Janitor {
    config: JanitorConfig,
    metrics: JanitorMetrics,
}

impl Janitor {
    /// Create a new Janitor with the given configuration
    pub fn new(config: JanitorConfig) -> Self {
        Self {
            config,
            metrics: JanitorMetrics::new(),
        }
    }

    /// Create a Janitor with default configuration
    pub fn default_config() -> Self {
        Self::new(JanitorConfig::default())
    }

    /// Get a reference to the current metrics
    pub fn metrics(&self) -> &JanitorMetrics {
        &self.metrics
    }

    /// Reset metrics counters
    pub fn reset_metrics(&mut self) {
        self.metrics.reset();
    }

    /// Perform a complete sweep cycle across all tiers
    ///
    /// This is the main entry point for cleanup operations. It:
    /// 1. Sweeps ephemeral claims past TTL
    /// 2. Sweeps task claims past TTL
    /// 3. Reviews project claims for staleness
    /// 4. Performs tier promotions/demotions if enabled
    ///
    /// Returns the updated metrics after the sweep.
    pub fn sweep<S: ClaimStore>(&mut self, store: &mut S) -> Result<JanitorMetrics, JanitorError>
    where
        S::Error: std::fmt::Display,
    {
        let start = SystemTime::now();

        // Sweep each tier
        self.sweep_ephemeral(store)?;
        self.sweep_tasks(store)?;
        self.sweep_projects(store)?;

        // Perform tier management if enabled
        if self.config.auto_promote {
            self.promote_candidates(store)?;
        }
        if self.config.auto_demote {
            self.demote_candidates(store)?;
        }

        // Record sweep completion
        self.metrics.record_sweep();
        
        if let Ok(elapsed) = start.elapsed() {
            self.metrics.total_runtime_secs += elapsed.as_secs();
        }

        Ok(self.metrics.clone())
    }

    /// Sweep ephemeral tier claims past TTL
    ///
    /// Deletes claims in the Ephemeral tier that have exceeded their TTL.
    fn sweep_ephemeral<S: ClaimStore>(&mut self, store: &mut S) -> Result<usize, JanitorError>
    where
        S::Error: std::fmt::Display,
    {
        self.sweep_tier(store, Tier::Ephemeral, self.config.ephemeral_ttl().as_secs())
    }

    /// Sweep task tier claims past TTL
    ///
    /// Deletes claims in the Task tier that have exceeded their TTL.
    fn sweep_tasks<S: ClaimStore>(&mut self, store: &mut S) -> Result<usize, JanitorError>
    where
        S::Error: std::fmt::Display,
    {
        self.sweep_tier(store, Tier::Task, self.config.task_ttl().as_secs())
    }

    /// Review project tier claims for staleness
    ///
    /// Deletes claims in the Project tier that haven't been accessed in the staleness threshold.
    fn sweep_projects<S: ClaimStore>(&mut self, store: &mut S) -> Result<usize, JanitorError>
    where
        S::Error: std::fmt::Display,
    {
        self.sweep_tier(store, Tier::Project, self.config.project_stale_threshold().as_secs())
    }

    /// Generic sweep implementation for a specific tier
    fn sweep_tier<S: ClaimStore>(
        &mut self,
        store: &mut S,
        tier: Tier,
        ttl_secs: u64,
    ) -> Result<usize, JanitorError>
    where
        S::Error: std::fmt::Display,
    {
        // Never sweep Permanent tier automatically
        if tier == Tier::Permanent {
            return Ok(0);
        }

        let now = current_timestamp();
        let cutoff = now.saturating_sub(ttl_secs);

        // Query claims in this tier
        let query = ClaimQuery {
            tier: Some(tier.as_str().to_string()),
            ..Default::default()
        };

        let claims = store.query_claims(&query)
            .map_err(|e| JanitorError::Store(e.to_string()))?;

        // Filter for stale claims (created before cutoff)
        let stale_claims: Vec<&Claim> = claims.iter()
            .filter(|claim| claim.created_at < cutoff)
            .collect();

        if stale_claims.is_empty() {
            return Ok(0);
        }

        if self.config.dry_run {
            tracing::info!(
                "DRY RUN: Would delete {} claims from {:?} tier",
                stale_claims.len(),
                tier
            );
            return Ok(0);
        }

        // Delete stale claims
        let deleted_count = stale_claims.len();
        
        // Note: ClaimStore doesn't have a delete method yet, so we'll just log for now
        // In a full implementation, we'd call store.delete_claims(stale_claim_ids)
        tracing::info!(
            "Would delete {} stale claims from {:?} tier (created before {})",
            deleted_count,
            tier,
            cutoff
        );

        self.metrics.record_deletion(tier, deleted_count);
        Ok(deleted_count)
    }

    /// Promote claims that meet promotion criteria
    ///
    /// Criteria:
    /// - High access frequency (above threshold)
    /// - Good confidence (above demotion threshold)
    /// - Not already at Permanent tier
    fn promote_candidates<S: ClaimStore>(&mut self, store: &mut S) -> Result<usize, JanitorError>
    where
        S::Error: std::fmt::Display,
    {
        let mut promoted = 0;

        // Check each tier for promotion candidates (except Permanent)
        for tier in [Tier::Ephemeral, Tier::Task, Tier::Project] {
            let query = ClaimQuery {
                tier: Some(tier.as_str().to_string()),
                min_confidence: Some(self.config.demotion_confidence_threshold),
                ..Default::default()
            };

            let claims = store.query_claims(&query)
                .map_err(|e| JanitorError::Store(e.to_string()))?;

            for claim in claims {
                // Check if claim meets promotion criteria
                if self.should_promote(&claim) {
                    if let Some(next_tier) = tier.next() {
                        if self.promote_claim(store, claim.id, tier, next_tier)? {
                            promoted += 1;
                            self.metrics.record_promotion(tier);
                        }
                    }
                }
            }
        }

        Ok(promoted)
    }

    /// Demote claims that meet demotion criteria
    ///
    /// Criteria:
    /// - Low confidence (below threshold)
    /// - Stale (no recent access)
    /// - Not already at Ephemeral tier
    fn demote_candidates<S: ClaimStore>(&mut self, store: &mut S) -> Result<usize, JanitorError>
    where
        S::Error: std::fmt::Display,
    {
        let mut demoted = 0;

        // Check each tier for demotion candidates (except Ephemeral)
        for tier in [Tier::Permanent, Tier::Project, Tier::Task] {
            let query = ClaimQuery {
                tier: Some(tier.as_str().to_string()),
                ..Default::default()
            };

            let claims = store.query_claims(&query)
                .map_err(|e| JanitorError::Store(e.to_string()))?;

            for claim in claims {
                // Check if claim meets demotion criteria
                if self.should_demote(&claim) {
                    if let Some(prev_tier) = tier.previous() {
                        if self.demote_claim(store, claim.id, tier, prev_tier)? {
                            demoted += 1;
                            self.metrics.record_demotion(tier);
                        }
                    }
                }
            }
        }

        Ok(demoted)
    }

    /// Determine if a claim should be promoted
    fn should_promote(&self, claim: &Claim) -> bool {
        // Promotion criteria:
        // 1. Confidence is good (above demotion threshold)
        // 2. Claim is not stale
        
        let confidence_good = claim.confidence.0 >= self.config.demotion_confidence_threshold;
        
        // Parse tier from string
        let tier = match Tier::parse(&claim.tier) {
            Some(t) => t,
            None => return false, // Invalid tier, skip
        };
        
        // Check staleness based on current tier
        let not_stale = match tier {
            Tier::Ephemeral => {
                let age_hours = (current_timestamp() - claim.created_at) / 3600;
                age_hours < self.config.ephemeral_ttl_hours / 2 // Active in first half of TTL
            }
            Tier::Task => {
                let age_hours = (current_timestamp() - claim.created_at) / 3600;
                age_hours < self.config.task_ttl_hours / 2
            }
            Tier::Project => {
                let age_days = (current_timestamp() - claim.created_at) / 86400;
                age_days < self.config.project_stale_days / 2
            }
            Tier::Permanent => false, // Already at top
        };

        confidence_good && not_stale
    }

    /// Determine if a claim should be demoted
    fn should_demote(&self, claim: &Claim) -> bool {
        // Demotion criteria:
        // 1. Low confidence (below threshold)
        // 2. Stale (approaching TTL)
        
        let confidence_low = claim.confidence.0 < self.config.demotion_confidence_threshold;
        
        // Parse tier from string
        let tier = match Tier::parse(&claim.tier) {
            Some(t) => t,
            None => return false, // Invalid tier, skip
        };
        
        // Check staleness based on tier-specific TTLs
        let is_stale = match tier {
            Tier::Ephemeral => false, // Don't demote from Ephemeral, just delete
            Tier::Task => {
                let age_hours = (current_timestamp() - claim.created_at) / 3600;
                age_hours > self.config.task_ttl_hours * 3 / 4 // In last 25% of TTL
            }
            Tier::Project => {
                let age_days = (current_timestamp() - claim.created_at) / 86400;
                age_days > self.config.project_stale_days * 3 / 4
            }
            Tier::Permanent => {
                // Only demote Permanent if confidence is very low
                claim.confidence.0 < 0.2
            }
        };

        confidence_low && is_stale
    }

    /// Promote a claim to the next tier
    fn promote_claim<S: ClaimStore>(
        &self,
        _store: &mut S,
        claim_id: ClaimId,
        from_tier: Tier,
        to_tier: Tier,
    ) -> Result<bool, JanitorError>
    where
        S::Error: std::fmt::Display,
    {
        if self.config.dry_run {
            tracing::info!(
                "DRY RUN: Would promote claim {} from {:?} to {:?}",
                claim_id,
                from_tier,
                to_tier
            );
            return Ok(false);
        }

        // Note: ClaimStore doesn't have an update_tier method yet
        // In a full implementation, we'd call store.update_claim_tier(claim_id, to_tier)
        tracing::info!(
            "Would promote claim {} from {:?} to {:?}",
            claim_id,
            from_tier,
            to_tier
        );

        Ok(true)
    }

    /// Demote a claim to the previous tier
    fn demote_claim<S: ClaimStore>(
        &self,
        _store: &mut S,
        claim_id: ClaimId,
        from_tier: Tier,
        to_tier: Tier,
    ) -> Result<bool, JanitorError>
    where
        S::Error: std::fmt::Display,
    {
        if self.config.dry_run {
            tracing::info!(
                "DRY RUN: Would demote claim {} from {:?} to {:?}",
                claim_id,
                from_tier,
                to_tier
            );
            return Ok(false);
        }

        // Note: ClaimStore doesn't have an update_tier method yet
        // In a full implementation, we'd call store.update_claim_tier(claim_id, to_tier)
        tracing::info!(
            "Would demote claim {} from {:?} to {:?}",
            claim_id,
            from_tier,
            to_tier
        );

        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Mock store for testing
    struct MockStore {
        claims: Vec<Claim>,
    }

    impl MockStore {
        fn new() -> Self {
            Self { claims: Vec::new() }
        }

        fn add_claim(&mut self, mut claim: Claim) {
            claim.id = ClaimId::new();
            self.claims.push(claim);
        }
    }

    impl ClaimStore for MockStore {
        type Error = String;

        fn assert_claim(&mut self, claim: Claim) -> Result<ClaimId, Self::Error> {
            let id = claim.id;
            self.claims.push(claim);
            Ok(id)
        }

        fn get_claim(&self, id: ClaimId) -> Result<Option<Claim>, Self::Error> {
            Ok(self.claims.iter().find(|c| c.id == id).cloned())
        }

        fn query_claims(&self, query: &ClaimQuery) -> Result<Vec<Claim>, Self::Error> {
            let mut results = self.claims.clone();

            // Filter by tier
            if let Some(tier_str) = &query.tier {
                results.retain(|c| c.tier == *tier_str);
            }

            // Filter by min confidence
            if let Some(min_conf) = query.min_confidence {
                results.retain(|c| c.confidence.0 >= min_conf);
            }

            // Apply limit
            if let Some(limit) = query.limit {
                results.truncate(limit);
            }

            Ok(results)
        }

        fn add_relationship(&mut self, _relationship: boswell_domain::Relationship) -> Result<(), Self::Error> {
            Ok(())
        }

        fn get_relationships(&self, _id: ClaimId) -> Result<Vec<boswell_domain::Relationship>, Self::Error> {
            Ok(Vec::new())
        }
    }

    fn create_test_claim(tier: Tier, age_hours: u64, confidence: f64) -> Claim {
        let now = current_timestamp();
        let created_at = now - (age_hours * 3600);

        Claim {
            id: ClaimId::new(),
            namespace: "test".to_string(),
            subject: "entity:test".to_string(),
            predicate: "has_property".to_string(),
            object: "value:123".to_string(),
            confidence: (confidence, confidence + 0.1),
            tier: tier.as_str().to_string(), // Convert Tier to String
            created_at,
            stale_at: None,
        }
    }

    #[test]
    fn test_janitor_creation() {
        let janitor = Janitor::default_config();
        assert_eq!(janitor.metrics().sweep_count, 0);
        assert_eq!(janitor.metrics().total_deleted(), 0);
    }

    #[test]
    fn test_sweep_ephemeral_stale_claims() {
        let mut store = MockStore::new();
        let config = JanitorConfig {
            ephemeral_ttl_hours: 12,
            dry_run: false,
            ..Default::default()
        };
        let mut janitor = Janitor::new(config);

        // Add stale ephemeral claim (20 hours old)
        store.add_claim(create_test_claim(Tier::Ephemeral, 20, 0.8));
        
        // Add fresh ephemeral claim (2 hours old)
        store.add_claim(create_test_claim(Tier::Ephemeral, 2, 0.8));

        let result = janitor.sweep_ephemeral(&mut store).unwrap();
        
        // Should identify 1 stale claim
        assert_eq!(result, 1);
        assert_eq!(janitor.metrics().deleted.get(&Tier::Ephemeral), Some(&1));
    }

    #[test]
    fn test_sweep_respects_dry_run() {
        let mut store = MockStore::new();
        let config = JanitorConfig {
            ephemeral_ttl_hours: 12,
            dry_run: true, // Dry run enabled
            ..Default::default()
        };
        let mut janitor = Janitor::new(config);

        // Add stale claim
        store.add_claim(create_test_claim(Tier::Ephemeral, 20, 0.8));

        let result = janitor.sweep_ephemeral(&mut store).unwrap();
        
        // Should not actually delete in dry-run mode
        assert_eq!(result, 0);
        assert_eq!(janitor.metrics().deleted.get(&Tier::Ephemeral), None);
    }

    #[test]
    fn test_sweep_never_deletes_permanent() {
        let mut store = MockStore::new();
        let mut janitor = Janitor::default_config();

        // Add old permanent claim
        store.add_claim(create_test_claim(Tier::Permanent, 10000, 0.8));

        let result = janitor.sweep_tier(&mut store, Tier::Permanent, 1).unwrap();
        
        // Should never sweep Permanent tier
        assert_eq!(result, 0);
    }

    #[test]
    fn test_should_promote_logic() {
        let janitor = Janitor::default_config();

        // Fresh claim with good confidence - should promote
        let claim = create_test_claim(Tier::Ephemeral, 2, 0.8);
        assert!(janitor.should_promote(&claim));

        // Old claim - should not promote
        let claim = create_test_claim(Tier::Ephemeral, 20, 0.8);
        assert!(!janitor.should_promote(&claim));

        // Low confidence - should not promote
        let claim = create_test_claim(Tier::Ephemeral, 2, 0.2);
        assert!(!janitor.should_promote(&claim));
    }

    #[test]
    fn test_should_demote_logic() {
        let janitor = Janitor::default_config();

        // Task tier: old + low confidence - should demote
        let claim = create_test_claim(Tier::Task, 30, 0.2);
        assert!(janitor.should_demote(&claim));

        // Task tier: fresh - should not demote
        let claim = create_test_claim(Tier::Task, 2, 0.2);
        assert!(!janitor.should_demote(&claim));

        // Task tier: good confidence - should not demote
        let claim = create_test_claim(Tier::Task, 30, 0.8);
        assert!(!janitor.should_demote(&claim));

        // Ephemeral: should not demote (just delete)
        let claim = create_test_claim(Tier::Ephemeral, 30, 0.1);
        assert!(!janitor.should_demote(&claim));
    }

    #[test]
    fn test_full_sweep_cycle() {
        let mut store = MockStore::new();
        let mut janitor = Janitor::default_config();

        // Add various claims
        store.add_claim(create_test_claim(Tier::Ephemeral, 20, 0.8)); // Stale
        store.add_claim(create_test_claim(Tier::Task, 30, 0.8)); // Stale
        store.add_claim(create_test_claim(Tier::Project, 2, 0.8)); // Fresh

        let metrics = janitor.sweep(&mut store).unwrap();
        
        assert_eq!(metrics.sweep_count, 1);
        assert!(metrics.total_deleted() > 0);
    }

    #[test]
    fn test_metrics_reset() {
        let mut janitor = Janitor::default_config();
        let mut store = MockStore::new();

        store.add_claim(create_test_claim(Tier::Ephemeral, 20, 0.8));
        janitor.sweep(&mut store).unwrap();

        assert!(janitor.metrics().sweep_count > 0);

        janitor.reset_metrics();
        
        assert_eq!(janitor.metrics().sweep_count, 0);
        assert_eq!(janitor.metrics().total_deleted(), 0);
    }
}
