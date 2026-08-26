#![warn(missing_docs)]

//! Boswell gRPC Service Layer
//!
//! Exposes the Boswell API surface via gRPC per Phase 2 requirements.
//! Implements:
//! - Assert/Query operations for single claims
//! - Learn operation for bulk insertion (ADR-012)
//! - Forget operation for eviction marking
//! - Health checks for instance monitoring

// Include generated protobuf code
#[allow(missing_docs)] // generated code; not subject to the crate's doc lint
pub mod proto {
    //! Generated protobuf types and service definitions
    tonic::include_proto!("boswell.v1");
}

pub mod conversions;
pub mod server;
pub mod service;

pub use server::{start_server, ServerConfig};
pub use service::BosWellServiceImpl;
