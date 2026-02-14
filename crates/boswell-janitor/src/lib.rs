//! Boswell Janitor
//!
//! Background maintenance service for automated tier management and claim lifecycle operations.
//!
//! # Overview
//!
//! The Janitor is responsible for:
//! - **Tier management**: Promoting and demoting claims based on access patterns and confidence
//! - **Stale claim detection**: Identifying claims past their TTL (time-to-live)
//! - **Garbage collection**: Removing stale claims to reclaim storage
//! - **Metrics collection**: Tracking cleanup operations for monitoring
//!
//! # Architecture
//!
//! Per ADR-007 (Hybrid Confidence Computation) and ADR-012 (Learn Operation), the Janitor
//! operates on a scheduled basis to maintain the health of the claim graph.
//!
//! ## Tier Lifecycle (per ADR-007)
//!
//! | Tier | TTL | Promotion Trigger | Demotion Trigger |
//! |------|-----|-------------------|------------------|
//! | **Ephemeral** | 12 hours | Access frequency > threshold | Session end or TTL |
//! | **Task** | 24 hours (if idle) | Reinforcement (2+ supporting claims) | Task complete or timeout |
//! | **Project** | 90 days (if unused) | Explicit promotion or high access | No access for 90 days |
//! | **Permanent** | Never | Explicit promotion only | Manual only (never automatic) |
//!
//! # Usage
//!
//! ## One-time Sweep
//!
//! ```no_run
//! use boswell_janitor::{Janitor, JanitorConfig};
//! use boswell_store::SqliteStore;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let mut store = SqliteStore::new("boswell.db", false, 0)?;
//! let mut janitor = Janitor::default_config();
//!
//! // Perform a single sweep
//! let metrics = janitor.sweep(&mut store)?;
//! println!("{}", metrics.summary());
//! # Ok(())
//! # }
//! ```
//!
//! ## Background Worker
//!
//! ```no_run
//! use boswell_janitor::{JanitorWorker, JanitorConfig};
//! use boswell_store::SqliteStore;
//! use tokio;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let store = SqliteStore::new("boswell.db", false, 0)?;
//!     let config = JanitorConfig::default();
//!     let mut worker = JanitorWorker::new(config);
//!
//!     // Run indefinitely (until Ctrl+C)
//!     worker.run(store).await?;
//!     Ok(())
//! }
//! ```
//!
//! ## Configuration Presets
//!
//! ```
//! use boswell_janitor::JanitorConfig;
//!
//! // Default: Balanced cleanup (12h ephemeral, 24h task, 90d project)
//! let config = JanitorConfig::default();
//!
//! // Aggressive: Shorter TTLs for resource-constrained environments
//! let config = JanitorConfig::aggressive();
//!
//! // Lenient: Longer TTLs for development or when keeping claims longer
//! let config = JanitorConfig::lenient();
//! ```
//!
//! # Configuration
//!
//! The Janitor can be configured via TOML:
//!
//! ```toml
//! [janitor]
//! ephemeral_ttl_hours = 12
//! task_ttl_hours = 24
//! project_stale_days = 90
//! sweep_interval_minutes = 60
//! demotion_confidence_threshold = 0.3
//! promotion_access_threshold = 7
//! dry_run = false
//! auto_promote = true
//! auto_demote = true
//! ```
//!
//! # Metrics
//!
//! The Janitor collects detailed metrics on operations:
//!
//! ```no_run
//! # use boswell_janitor::{Janitor, JanitorConfig};
//! # use boswell_store::SqliteStore;
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! # let mut store = SqliteStore::new(":memory:", false, 0)?;
//! # let mut janitor = Janitor::default_config();
//! let metrics = janitor.sweep(&mut store)?;
//!
//! println!("Deleted: {}", metrics.total_deleted());
//! println!("Promoted: {}", metrics.total_promoted());
//! println!("Demoted: {}", metrics.total_demoted());
//! println!("Sweep cycles: {}", metrics.sweep_count);
//! println!("\n{}", metrics.summary());
//! # Ok(())
//! # }
//! ```

#![warn(missing_docs)]

mod error;
mod config;
mod metrics;
mod janitor;
mod worker;

pub use error::JanitorError;
pub use config::JanitorConfig;
pub use metrics::JanitorMetrics;
pub use janitor::Janitor;
pub use worker::JanitorWorker;

