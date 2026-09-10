//! Boswell Rust SDK
//!
//! Client library for interacting with Boswell instances via the Router.
//!
//! # Example
//!
//! ```no_run
//! use boswell_sdk::{BoswellClient, QueryFilter};
//!
//! #[tokio::main]
//! async fn main() {
//!     let mut client = BoswellClient::new("http://localhost:8080");
//!     client.connect().await.expect("Failed to connect");
//!
//!     let claim_id = client.assert(
//!         "personal",
//!         "John",
//!         "knows",
//!         "Rust",
//!         Some((0.9, 0.95)),
//!         None
//!     ).await.expect("Failed to assert claim");
//! }
//! ```

#![warn(missing_docs)]

mod client;
mod error;
mod retry;
mod session;

pub use client::{
    BoswellClient, ExtractResult, GoalQuerySpec, HealthStatus, IssuedProcedure, OutcomeReportSpec,
    ProcedureQuerySpec, QueryFilter,
};
pub use error::SdkError;
pub use retry::{Idempotency, RetryPolicy};

/// The response to an [`report_outcome`](BoswellClient::report_outcome) call.
///
/// Re-exported so callers can name what the client hands back without taking a
/// direct dependency on the gRPC crate.
pub use boswell_grpc::proto::ReportOutcomeResponse;
