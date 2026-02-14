//! Background worker for continuous Janitor operation

use crate::{Janitor, JanitorConfig, JanitorError};
use boswell_domain::traits::ClaimStore;
use tokio::time::{interval, Duration};

/// Background worker that runs Janitor on a schedule
///
/// This worker provides continuous automated cleanup and tier management.
/// It runs the Janitor sweep cycle at regular intervals defined by the configuration.
///
/// # Examples
///
/// ```no_run
/// use boswell_janitor::{JanitorWorker, JanitorConfig};
/// use boswell_store::SqliteStore;
/// use tokio;
///
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     let store = SqliteStore::new("boswell.db", false, 0)?;
///     let config = JanitorConfig::default();
///     let mut worker = JanitorWorker::new(config);
///
///     // Run indefinitely (until Ctrl+C)
///     worker.run(store).await?;
///     Ok(())
/// }
/// ```
pub struct JanitorWorker {
    janitor: Janitor,
    interval: Duration,
}

impl JanitorWorker {
    /// Create a new background worker with the given configuration
    pub fn new(config: JanitorConfig) -> Self {
        let interval = config.sweep_interval();
        Self {
            janitor: Janitor::new(config),
            interval,
        }
    }

    /// Create a worker with default configuration
    pub fn default_config() -> Self {
        Self::new(JanitorConfig::default())
    }

    /// Run the worker indefinitely
    ///
    /// This method will run the Janitor sweep cycle at the configured interval
    /// until a shutdown signal (Ctrl+C) is received.
    ///
    /// # Errors
    ///
    /// Returns an error if the sweep operation fails or if there's a tokio runtime error.
    pub async fn run<S>(&mut self, mut store: S) -> Result<(), JanitorError>
    where
        S: ClaimStore,
        S::Error: std::fmt::Display,
    {
        let mut ticker = interval(self.interval);
        
        tracing::info!(
            "Janitor worker started (interval: {:?})",
            self.interval
        );

        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    tracing::debug!("Starting sweep cycle");
                    
                    match self.janitor.sweep(&mut store) {
                        Ok(metrics) => {
                            tracing::info!(
                                "Sweep completed: {} deleted, {} promoted, {} demoted",
                                metrics.total_deleted(),
                                metrics.total_promoted(),
                                metrics.total_demoted()
                            );
                        }
                        Err(e) => {
                            tracing::error!("Sweep failed: {}", e);
                        }
                    }
                }
                _ = tokio::signal::ctrl_c() => {
                    tracing::info!("Shutdown signal received, stopping janitor");
                    break;
                }
            }
        }

        // Print final metrics
        let metrics = self.janitor.metrics();
        tracing::info!("Janitor stopped. Final metrics:\n{}", metrics.summary());

        Ok(())
    }

    /// Run for a specific number of cycles (useful for testing)
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use boswell_janitor::{JanitorWorker, JanitorConfig};
    /// use boswell_store::SqliteStore;
    /// use tokio;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     let store = SqliteStore::new(":memory:", false, 0)?;
    ///     let config = JanitorConfig::default();
    ///     let mut worker = JanitorWorker::new(config);
    ///
    ///     // Run for 3 cycles then stop
    ///     worker.run_cycles(store, 3).await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn run_cycles<S>(&mut self, mut store: S, cycles: usize) -> Result<(), JanitorError>
    where
        S: ClaimStore,
        S::Error: std::fmt::Display,
    {
        let mut ticker = interval(self.interval);
        
        tracing::info!(
            "Janitor worker started for {} cycles (interval: {:?})",
            cycles,
            self.interval
        );

        for cycle in 0..cycles {
            ticker.tick().await;
            
            tracing::debug!("Starting sweep cycle {}/{}", cycle + 1, cycles);
            
            match self.janitor.sweep(&mut store) {
                Ok(metrics) => {
                    tracing::info!(
                        "Sweep {}/{} completed: {} deleted, {} promoted, {} demoted",
                        cycle + 1,
                        cycles,
                        metrics.total_deleted(),
                        metrics.total_promoted(),
                        metrics.total_demoted()
                    );
                }
                Err(e) => {
                    tracing::error!("Sweep {}/{} failed: {}", cycle + 1, cycles, e);
                    return Err(e);
                }
            }
        }

        // Print final metrics
        let metrics = self.janitor.metrics();
        tracing::info!("Janitor finished {} cycles. Final metrics:\n{}", cycles, metrics.summary());

        Ok(())
    }

    /// Get a reference to the janitor's current metrics
    pub fn metrics(&self) -> &crate::JanitorMetrics {
        self.janitor.metrics()
    }

    /// Reset the janitor's metrics counters
    pub fn reset_metrics(&mut self) {
        self.janitor.reset_metrics();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::JanitorConfig;
    use boswell_domain::{Claim, ClaimId, Tier};
    use boswell_domain::traits::ClaimQuery;
    use std::time::{SystemTime, UNIX_EPOCH};

    // Mock store for testing
    struct MockStore {
        claims: Vec<Claim>,
    }

    impl MockStore {
        fn new() -> Self {
            Self { claims: Vec::new() }
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

            if let Some(tier_str) = &query.tier {
                results.retain(|c| c.tier == *tier_str);
            }

            if let Some(min_conf) = query.min_confidence {
                results.retain(|c| c.confidence.0 >= min_conf);
            }

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

    fn current_timestamp() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    fn create_test_claim(tier: Tier, age_hours: u64) -> Claim {
        let now = current_timestamp();
        let created_at = now - (age_hours * 3600);

        Claim {
            id: ClaimId::new(),
            namespace: "test".to_string(),
            subject: "entity:test".to_string(),
            predicate: "has_property".to_string(),
            object: "value:123".to_string(),
            confidence: (0.8, 0.9),
            tier: tier.as_str().to_string(), // Convert Tier to String
            created_at,
            stale_at: None,
        }
    }

    #[tokio::test]
    async fn test_worker_creation() {
        let worker = JanitorWorker::default_config();
        assert_eq!(worker.metrics().sweep_count, 0);
    }

    #[tokio::test]
    async fn test_run_cycles() {
        let mut store = MockStore::new();
        store.claims.push(create_test_claim(Tier::Ephemeral, 20));

        let config = JanitorConfig {
            ephemeral_ttl_hours: 12,
            sweep_interval_minutes: 1, // 1 minute minimum (1 ms would panic)
            dry_run: false,
            ..Default::default()
        };
        let mut worker = JanitorWorker::new(config);

        // Run for 2 cycles
        worker.run_cycles(store, 2).await.unwrap();

        // Should have completed 2 sweeps
        assert_eq!(worker.metrics().sweep_count, 2);
    }

    #[tokio::test]
    async fn test_metrics_tracking() {
        let mut store = MockStore::new();
        
        // Add stale claim
        store.claims.push(create_test_claim(Tier::Ephemeral, 20));

        let config = JanitorConfig {
            ephemeral_ttl_hours: 12,
            sweep_interval_minutes: 1, // 1 minute minimum
            dry_run: false,
            ..Default::default()
        };
        let mut worker = JanitorWorker::new(config);

        worker.run_cycles(store, 1).await.unwrap();

        let metrics = worker.metrics();
        assert_eq!(metrics.sweep_count, 1);
        // Check that metrics are being updated (at least one value should be non-default)
        assert!(metrics.total_deleted() > 0 || metrics.total_runtime_secs > 0);
    }

    #[tokio::test]
    async fn test_reset_metrics() {
        let store = MockStore::new();
        let config = JanitorConfig {
            sweep_interval_minutes: 1, // 1 minute minimum
            ..Default::default()
        };
        let mut worker = JanitorWorker::new(config);

        worker.run_cycles(store, 1).await.unwrap();
        assert_eq!(worker.metrics().sweep_count, 1);

        worker.reset_metrics();
        assert_eq!(worker.metrics().sweep_count, 0);
    }
}
