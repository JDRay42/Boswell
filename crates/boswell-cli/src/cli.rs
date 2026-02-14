//! CLI command definitions and argument parsing.

use clap::{Parser, Subcommand};

/// Boswell CLI - Interact with the Boswell cognitive memory system.
#[derive(Debug, Parser)]
#[command(name = "boswell")]
#[command(version, about, long_about = None)]
pub struct Cli {
    /// Output format
    #[arg(short, long, value_enum, global = true)]
    pub format: Option<CliFormat>,

    /// Disable colored output
    #[arg(long, global = true)]
    pub no_color: bool,

    /// Configuration file path
    #[arg(short, long, global = true)]
    pub config: Option<String>,

    /// Profile to use
    #[arg(short, long, global = true)]
    pub profile: Option<String>,

    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Output format options.
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum CliFormat {
    /// Table format (default)
    Table,
    /// JSON format
    Json,
    /// Quiet format (IDs only)
    Quiet,
}

/// CLI commands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Connect to Boswell router
    Connect(ConnectArgs),

    /// Assert a new claim
    Assert(AssertArgs),

    /// Query claims
    Query(QueryArgs),

    /// Learn (batch assert) multiple claims
    Learn(LearnArgs),

    /// Forget (delete) claims
    Forget(ForgetArgs),

    /// Semantic search for claims
    Search(SearchArgs),

    /// Manage configuration profiles
    Profile(ProfileArgs),

    /// Enter interactive REPL mode
    Repl,
}

/// Arguments for the connect command.
#[derive(Debug, Parser)]
pub struct ConnectArgs {
    /// Router URL (e.g., http://localhost:8080)
    #[arg(short, long)]
    pub url: Option<String>,

    /// Instance ID
    #[arg(short, long)]
    pub instance: Option<String>,

    /// Namespace
    #[arg(short, long)]
    pub namespace: Option<String>,

    /// Save connection as a profile
    #[arg(long)]
    pub save_as: Option<String>,
}

/// Arguments for the assert command.
#[derive(Debug, Parser)]
pub struct AssertArgs {
    /// Subject (format: namespace:value)
    pub subject: String,

    /// Predicate (format: namespace:value)
    pub predicate: String,

    /// Object (format: namespace:value)
    pub object: String,

    /// Confidence lower bound (0.0-1.0)
    #[arg(short = 'l', long, default_value = "0.5")]
    pub confidence_lower: f64,

    /// Confidence upper bound (0.0-1.0)
    #[arg(short = 'u', long, default_value = "1.0")]
    pub confidence_upper: f64,

    /// Claim tier
    #[arg(short, long, value_enum, default_value = "task")]
    pub tier: TierArg,
}

/// Arguments for the query command.
#[derive(Debug, Parser)]
pub struct QueryArgs {
    /// Filter by subject (format: namespace:value or namespace:*)
    #[arg(short, long)]
    pub subject: Option<String>,

    /// Filter by predicate (format: namespace:value or namespace:*)
    #[arg(short, long)]
    pub predicate: Option<String>,

    /// Filter by object (format: namespace:value or namespace:*)
    #[arg(short, long)]
    pub object: Option<String>,

    /// Filter by tier
    #[arg(short, long, value_enum)]
    pub tier: Option<TierArg>,

    /// Minimum confidence lower bound
    #[arg(long)]
    pub min_confidence: Option<f64>,

    /// Maximum number of results
    #[arg(short, long)]
    pub limit: Option<usize>,
}

/// Arguments for the learn command.
#[derive(Debug, Parser)]
pub struct LearnArgs {
    /// JSON file containing claims to assert
    #[arg(short, long)]
    pub file: Option<String>,

    /// JSON array of claims from stdin
    #[arg(long)]
    pub stdin: bool,

    /// Default tier for claims without explicit tier
    #[arg(short, long, value_enum, default_value = "task")]
    pub tier: TierArg,
}

/// Arguments for the forget command.
#[derive(Debug, Parser)]
pub struct ForgetArgs {
    /// Claim IDs to delete
    pub ids: Vec<String>,

    /// Read IDs from file (one per line)
    #[arg(short, long)]
    pub file: Option<String>,

    /// Read IDs from stdin (one per line)
    #[arg(long)]
    pub stdin: bool,

    /// Skip confirmation prompt
    #[arg(short = 'y', long)]
    pub yes: bool,
}

/// Arguments for the search command.
#[derive(Debug, Parser)]
pub struct SearchArgs {
    /// Search query text
    pub query: String,

    /// Maximum number of results
    #[arg(short, long, default_value = "10")]
    pub limit: usize,

    /// Minimum similarity threshold (0.0-1.0)
    #[arg(short, long, default_value = "0.7")]
    pub threshold: f64,
}

/// Arguments for profile management.
#[derive(Debug, Parser)]
pub struct ProfileArgs {
    #[command(subcommand)]
    pub action: ProfileAction,
}

/// Profile management actions.
#[derive(Debug, Subcommand)]
pub enum ProfileAction {
    /// List all profiles
    List,

    /// Show active profile
    Show,

    /// Switch to a different profile
    Switch {
        /// Profile name
        name: String,
    },

    /// Create or update a profile
    Set {
        /// Profile name
        name: String,
        /// Router URL
        #[arg(short, long)]
        url: String,
        /// Instance ID
        #[arg(short, long)]
        instance: String,
        /// Namespace
        #[arg(short, long)]
        namespace: Option<String>,
    },

    /// Delete a profile
    Delete {
        /// Profile name
        name: String,
    },
}

/// Tier argument.
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum TierArg {
    /// Ephemeral tier (short-lived)
    Ephemeral,
    /// Task tier (medium-term)
    Task,
    /// Project tier (long-term)
    Project,
    /// Permanent tier (core knowledge)
    Permanent,
}

impl From<CliFormat> for crate::config::OutputFormat {
    fn from(format: CliFormat) -> Self {
        match format {
            CliFormat::Table => crate::config::OutputFormat::Table,
            CliFormat::Json => crate::config::OutputFormat::Json,
            CliFormat::Quiet => crate::config::OutputFormat::Quiet,
        }
    }
}

impl From<TierArg> for boswell_domain::Tier {
    fn from(tier: TierArg) -> Self {
        match tier {
            TierArg::Ephemeral => boswell_domain::Tier::Ephemeral,
            TierArg::Task => boswell_domain::Tier::Task,
            TierArg::Project => boswell_domain::Tier::Project,
            TierArg::Permanent => boswell_domain::Tier::Permanent,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_parsing() {
        let cli = Cli::parse_from(["boswell", "--help"]);
        assert!(cli.command.is_none());
    }

    #[test]
    fn test_assert_command() {
        let cli = Cli::parse_from([
            "boswell",
            "assert",
            "user:alice",
            "likes:coffee",
            "beverage:espresso",
        ]);
        match cli.command {
            Some(Command::Assert(_)) => (),
            _ => panic!("Expected Assert command"),
        }
    }

    #[test]
    fn test_tier_conversion() {
        let tier: boswell_domain::Tier = TierArg::Task.into();
        assert!(matches!(tier, boswell_domain::Tier::Task));
    }
}
