//! Boswell CLI library.
//!
//! This library provides the core functionality for the Boswell command-line interface,
//! including configuration management, command execution, and output formatting.

pub mod cli;
pub mod commands;
pub mod config;
pub mod error;
pub mod output;
pub mod repl;

pub use cli::{Cli, Command};
pub use config::Config;
pub use error::{CliError, Result};
pub use output::Formatter;
