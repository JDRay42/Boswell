//! Boswell Synthesizer
//!
//! Discovers emergent patterns and creates higher-order derived claims
//! (per ADR-006 and `docs/architecture/06-synthesizer.md`).
//!
//! # Overview
//!
//! The Synthesizer is a background process that continuously examines the claim
//! graph and discovers emergent ideas — clusters of related claims that together
//! imply a higher-order insight no individual claim represents. It produces new
//! claims linked back to their constituents via `derived_from` relationships,
//! enabling organic abstraction layers (first-order → second-order → …).
//!
//! # How a pass works
//!
//! ```text
//! candidate claims → cluster → LLM analysis → insight → gatekeeper → store
//!                                    │                        │
//!                              "no insight" ok          derived_from edges
//! ```
//!
//! 1. **Candidate selection** — fetch claims at or above a minimum tier,
//!    optionally scoped by namespace and recency.
//! 2. **Clustering** — group candidates by relationship edges and shared
//!    subjects (transitive union-find).
//! 3. **Analysis** — ask the LLM whether each cluster implies a single
//!    higher-order insight ("no insight" is a valid, common answer).
//! 4. **Confidence propagation** — the derived claim is never more certain than
//!    its weakest constituent; uncertainty widens outward.
//! 5. **Depth limiting** — clusters whose constituents already sit at the
//!    maximum `derived_from` depth are skipped, preventing runaway meta-synthesis.
//! 6. **Validation & persistence** — the Gatekeeper validates the insight, then
//!    it is stored with `derived_from` relationships to each constituent.
//!
//! # Example
//!
//! ```no_run
//! use boswell_synthesizer::{Synthesizer, SynthesizerConfig, SynthesisScope};
//! use boswell_llm::MockProvider;
//! use boswell_store::SqliteStore;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let llm = MockProvider::new(r#"{"insight": false, "rationale": "n/a"}"#);
//! let mut store = SqliteStore::new(":memory:", false, 0)?;
//! let synthesizer = Synthesizer::new(llm, SynthesizerConfig::default());
//!
//! let scope = SynthesisScope::all("task", 50);
//! let report = synthesizer.run_pass(&mut store, scope).await?;
//! println!("{}", report.summary());
//! # Ok(())
//! # }
//! ```
//!
//! # Background operation
//!
//! Use [`SynthesizerWorker`] to run passes on a schedule:
//!
//! ```no_run
//! use boswell_synthesizer::{SynthesizerWorker, SynthesizerConfig};
//! use boswell_llm::MockProvider;
//! use boswell_store::SqliteStore;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let store = SqliteStore::new("boswell.db", false, 0)?;
//! let llm = MockProvider::new(r#"{"insight": false}"#);
//! let worker = SynthesizerWorker::new(llm, SynthesizerConfig::default());
//! worker.run(store).await?; // runs until Ctrl+C
//! # Ok(())
//! # }
//! ```

#![warn(missing_docs)]

mod clustering;
mod confidence;
mod config;
mod error;
mod parser;
mod prompt;
mod synthesizer;
mod types;
mod worker;

#[cfg(test)]
mod tests;

pub use config::SynthesizerConfig;
pub use error::SynthesizerError;
pub use synthesizer::Synthesizer;
pub use types::{
    ClaimCluster, InsightCandidate, SynthesisReport, SynthesisScope, SynthesizedInsight,
};
pub use worker::SynthesizerWorker;

// Re-export the confidence-propagation helper for downstream reuse/testing.
pub use confidence::propagate_confidence;
