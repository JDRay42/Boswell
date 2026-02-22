//! Boswell Extractor
//!
//! Converts unstructured text to structured claims using LLM per ADR-015.
//!
//! # Overview
//!
//! The Extractor is the primary pathway for ingesting knowledge from documents,
//! transcripts, articles, and other text sources. It uses an LLM to analyze text
//! and produce structured claims in Boswell's triple format.
//!
//! # Architecture
//!
//! ```text
//! Text → Extractor → LLM → Claims → Gatekeeper → ClaimStore
//! ```
//!
//! # Key Features
//!
//! - **Text-to-Claims Conversion**: Accept text blocks and produce structured claims
//! - **LLM Integration**: Prompt engineering for reliable claim extraction
//! - **Provenance Tracking**: Link extracted claims back to source
//! - **Duplicate Handling**: Corroborate existing claims when duplicates found
//! - **Batch Processing**: Handle large documents via chunking
//!
//! # Example Usage
//!
//! ```no_run
//! use boswell_extractor::{Extractor, ExtractorConfig, ExtractionRequest};
//! use boswell_llm::MockProvider;
//! use boswell_store::SqliteStore;
//! use boswell_gatekeeper::Gatekeeper;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Setup
//! let llm = MockProvider::new("[]");
//! let store = SqliteStore::new(":memory:", false, 0)?;
//! let gatekeeper = Gatekeeper::default_config();
//! let config = ExtractorConfig::default();
//!
//! let extractor = Extractor::new(llm, store, gatekeeper, config);
//!
//! // Extract claims from text
//! let request = ExtractionRequest {
//!     text: "Alice works at Acme Corp.".to_string(),
//!     namespace: "engineering:team".to_string(),
//!     tier: "project".to_string(),
//!     source_id: "doc_001".to_string(),
//!     existing_context: None,
//! };
//!
//! let result = extractor.extract(request).await?;
//!
//! println!("Created: {} claims", result.claims_created.len());
//! println!("Corroborated: {} claims", result.claims_corroborated.len());
//! println!("Failures: {} claims", result.failures.len());
//! # Ok(())
//! # }
//! ```

#![warn(missing_docs)]

mod error;
mod config;
mod types;
mod prompt;
mod chunking;
mod parser;
mod extractor;

#[cfg(test)]
mod tests;

pub use error::ExtractorError;
pub use config::{ExtractorConfig, ChunkStrategy};
pub use types::{
    ExtractionRequest, ExtractionResult, ClaimResult, ClaimSummary,
    ExtractionFailure, ExtractionMetadata,
};
pub use extractor::Extractor;

