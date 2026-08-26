//! Boswell Gatekeeper
//!
//! Evaluates and validates claims for quality control per ADR-008.
//!
//! The Gatekeeper provides:
//! - Claim validation (entity format, confidence bounds)
//! - Duplicate detection
//! - Tier appropriateness checking
//! - Quality scoring
//!
//! # Examples
//!
//! ```no_run
//! use boswell_gatekeeper::{Gatekeeper, ValidationConfig};
//!
//! let config = ValidationConfig::default();
//! let gatekeeper = Gatekeeper::new(config);
//!
//! // Validate a claim before storing
//! // let result = gatekeeper.validate(&claim, &store);
//! ```

#![warn(missing_docs)]

mod config;
mod error;
mod validator;

pub use config::ValidationConfig;
pub use error::GatekeeperError;
pub use validator::{Gatekeeper, RejectionReason, ValidationResult, ValidationStatus};
