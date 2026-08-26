//! Core Janitor implementation for tier management and cleanup

use crate::{JanitorConfig, JanitorError, JanitorMetrics};
use boswell_domain::traits::{ClaimQuery, ClaimStore};
use boswell_domain::{decayed_confidence, Claim, ClaimId, DecayConfig, Tier};
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
    decay: DecayConfig,
    metrics: JanitorMetrics,
}

impl Janitor {
    /// Create a new Janitor with the given configuration and default decay model.
    pub fn new(config: JanitorConfig) -> Self {
        Self {
            config,
            decay: DecayConfig::default(),
            metrics: JanitorMetrics::new(),
        }
    }

    /// Create a Janitor with an explicit decay configuration.
    ///
    /// Tier decisions use age-decayed effective confidence (per ADR-007), so the
    /// decay model determines how quickly claims become demotion candidates.
    pub fn with_decay(config: JanitorConfig, decay: DecayConfig) -> Self {
        Self {
            config,
            decay,
            metrics: JanitorMetrics::new(),
        }
    }

    /// Create a Janitor with default configuration
    pub fn default_config() -> Self {
        Self::new(JanitorConfig::default())
    }

    /// Effective (age-decayed) lower confidence bound for a claim, as of `now`.
    fn effective_lower(&self, claim: &Claim, now: u64) -> f64 {
        decayed_confidence(
            claim.confidence,
            &claim.tier,
            claim.created_at,
            now,
            &self.decay,
        )
        .0
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
        self.sweep_tier(
            store,
            Tier::Ephemeral,
            self.config.ephemeral_ttl().as_secs(),
        )
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
        self.sweep_tier(
            store,
            Tier::Project,
            self.config.project_stale_threshold().as_secs(),
        )
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

        let claims = store
            .query_claims(&query)
            .map_err(|e| JanitorError::Store(e.to_string()))?;

        // Filter for stale claims (created before cutoff)
        let stale_claims: Vec<&Claim> = claims
            .iter()
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

        // Delete stale claims from the store.
        let stale_ids: Vec<ClaimId> = stale_claims.iter().map(|c| c.id).collect();
        let mut deleted_count = 0;
        for id in stale_ids {
            if store
                .delete_claim(id)
                .map_err(|e| JanitorError::Store(e.to_string()))?
            {
                deleted_count += 1;
            }
        }

        tracing::info!(
            "Deleted {} stale claims from {:?} tier (created before {})",
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

            let claims = store
                .query_claims(&query)
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

            let claims = store
                .query_claims(&query)
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
        // 1. Effective (decayed) confidence is good (above demotion threshold)
        // 2. Claim is not stale
        let now = current_timestamp();
        let confidence_good =
            self.effective_lower(claim, now) >= self.config.demotion_confidence_threshold;

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
        // 1. Low effective (decayed) confidence (below threshold)
        // 2. Stale (approaching TTL)
        let now = current_timestamp();
        let effective_lower = self.effective_lower(claim, now);
        let confidence_low = effective_lower < self.config.demotion_confidence_threshold;

        // Parse tier from string
        let tier = match Tier::parse(&claim.tier) {
            Some(t) => t,
            None => return false, // Invalid tier, skip
        };

        // Check staleness based on tier-specific TTLs
        let is_stale = match tier {
            Tier::Ephemeral => false, // Don't demote from Ephemeral, just delete
            Tier::Task => {
                let age_hours = (now - claim.created_at) / 3600;
                age_hours > self.config.task_ttl_hours * 3 / 4 // In last 25% of TTL
            }
            Tier::Project => {
                let age_days = (now - claim.created_at) / 86400;
                age_days > self.config.project_stale_days * 3 / 4
            }
            Tier::Permanent => {
                // Only demote Permanent if effective confidence is very low.
                effective_lower < 0.2
            }
        };

        confidence_low && is_stale
    }

    /// Promote a claim to the next tier
    fn promote_claim<S: ClaimStore>(
        &self,
        store: &mut S,
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

        let updated = store
            .update_claim_tier(claim_id, to_tier.as_str())
            .map_err(|e| JanitorError::Store(e.to_string()))?;
        if updated {
            tracing::info!(
                "Promoted claim {} from {:?} to {:?}",
                claim_id,
                from_tier,
                to_tier
            );
        }
        Ok(updated)
    }

    /// Demote a claim to the previous tier
    fn demote_claim<S: ClaimStore>(
        &self,
        store: &mut S,
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

        let updated = store
            .update_claim_tier(claim_id, to_tier.as_str())
            .map_err(|e| JanitorError::Store(e.to_string()))?;
        if updated {
            tracing::info!(
                "Demoted claim {} from {:?} to {:?}",
                claim_id,
                from_tier,
                to_tier
            );
        }
        Ok(updated)
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

        fn add_relationship(
            &mut self,
            _relationship: boswell_domain::Relationship,
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        fn get_relationships(
            &self,
            _id: ClaimId,
        ) -> Result<Vec<boswell_domain::Relationship>, Self::Error> {
            Ok(Vec::new())
        }

        fn delete_claim(&mut self, id: ClaimId) -> Result<bool, Self::Error> {
            let before = self.claims.len();
            self.claims.retain(|c| c.id != id);
            Ok(self.claims.len() < before)
        }

        fn update_claim_tier(&mut self, id: ClaimId, new_tier: &str) -> Result<bool, Self::Error> {
            if let Some(claim) = self.claims.iter_mut().find(|c| c.id == id) {
                claim.tier = new_tier.to_string();
                Ok(true)
            } else {
                Ok(false)
            }
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
            source_type: "assertion".to_string(),
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

        // Task tier: fresh - should not demote (not stale)
        let claim = create_test_claim(Tier::Task, 2, 0.2);
        assert!(!janitor.should_demote(&claim));

        // Task tier: stale, and high base confidence that has decayed below the
        // threshold (task half-life 12h; at 30h a 0.8 base decays to ~0.14).
        let claim = create_test_claim(Tier::Task, 30, 0.8);
        assert!(janitor.should_demote(&claim));

        // Task tier: stale, but base confidence high enough that the decayed
        // value stays above the threshold (~0.33 at 19h) - should not demote.
        let claim = create_test_claim(Tier::Task, 19, 0.99);
        assert!(!janitor.should_demote(&claim));

        // Ephemeral: should not demote (just delete)
        let claim = create_test_claim(Tier::Ephemeral, 30, 0.1);
        assert!(!janitor.should_demote(&claim));
    }

    #[test]
    fn test_demote_is_decay_aware() {
        // A task claim whose *base* confidence is well above the demotion
        // threshold but whose *decayed* confidence has fallen below it must be
        // demoted — this is the behavior that base-confidence logic would miss.
        let janitor = Janitor::default_config();
        let claim = create_test_claim(Tier::Task, 30, 0.8);

        assert!(
            claim.confidence.0 >= janitor.config.demotion_confidence_threshold,
            "base confidence should be above the threshold"
        );
        assert!(
            janitor.effective_lower(&claim, current_timestamp())
                < janitor.config.demotion_confidence_threshold,
            "decayed confidence should be below the threshold"
        );
        assert!(janitor.should_demote(&claim));
    }

    #[test]
    fn test_sweep_actually_deletes_from_store() {
        let mut store = MockStore::new();
        let config = JanitorConfig {
            ephemeral_ttl_hours: 12,
            dry_run: false,
            ..Default::default()
        };
        let mut janitor = Janitor::new(config);

        store.add_claim(create_test_claim(Tier::Ephemeral, 20, 0.8)); // stale
        store.add_claim(create_test_claim(Tier::Ephemeral, 2, 0.8)); // fresh
        assert_eq!(store.claims.len(), 2);

        let deleted = janitor.sweep_ephemeral(&mut store).unwrap();
        assert_eq!(deleted, 1);
        // The stale claim is really gone; the fresh one remains.
        assert_eq!(store.claims.len(), 1);
        assert!(store
            .claims
            .iter()
            .all(|c| (current_timestamp() - c.created_at) / 3600 < 12));
    }

    #[test]
    fn test_demote_updates_tier_in_store() {
        let mut store = MockStore::new();
        let mut janitor = Janitor::default_config();

        // Stale task claim with decayed-low confidence → demoted to ephemeral.
        store.add_claim(create_test_claim(Tier::Task, 30, 0.8));
        janitor.demote_candidates(&mut store).unwrap();

        assert_eq!(store.claims.len(), 1);
        assert_eq!(store.claims[0].tier, Tier::Ephemeral.as_str());
        assert_eq!(janitor.metrics().total_demoted(), 1);
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
