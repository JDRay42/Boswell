//! Boswell Rust SDK
//!
//! Client library for interacting with Boswell instances via the Router.
//!
//! # Example
//!
//! ```no_run
//! use boswell_sdk::{BoswellClient, QueryFilter};
//!
//! let mut client = BoswellClient::new("http://localhost:8080");
//! client.connect().expect("Failed to connect");
//!
//! let claim_id = client.assert(
//!     "personal",
//!     "John",
//!     "knows",
//!     "Rust",
//!     Some(0.95),
//!     None
//! ).expect("Failed to assert claim");
//! ```

#![warn(missing_docs)]

mod client;
mod error;
mod session;

pub use client::{BoswellClient, QueryFilter};
pub use error::SdkError;

