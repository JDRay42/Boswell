#![warn(missing_docs)]

//! Boswell as the shared memory of an [OpenRig](https://github.com/mvschwarz/openrig)
//! team. The guide is `docs/integrations/openrig.md`.
//!
//! OpenRig already answers what is being worked and who owns it. This crate
//! adds what is known and what exists to reuse:
//!
//! - [`hook`] — the commands an OpenRig seat runs as hooks: a width dossier at
//!   session start, a digest of what changed in memory at each prompt.
//! - [`bridge`] — a long-running process that announces OpenRig queue work in
//!   Boswell and posts Boswell's notable changes to OpenRig's stream.
//!
//! Both talk to the Boswell gateway over HTTP and to the OpenRig daemon over
//! HTTP; neither needs anything from either project's internals.

pub mod boswell;
pub mod bridge;
pub mod config;
pub mod digest;
pub mod hook;
pub mod sse;
