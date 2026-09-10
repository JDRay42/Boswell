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

    /// Validate a learn JSON file offline (no server connection needed)
    Validate(ValidateArgs),

    /// Forget (delete) claims
    Forget(ForgetArgs),

    /// Semantic search for claims
    Search(SearchArgs),

    /// Manage configuration profiles
    Profile(ProfileArgs),

    /// Obtain a provider token with the OAuth device grant (ADR-022)
    Login(LoginArgs),

    /// Discard the stored provider token
    Logout,

    /// Navigate goal decompositions (procedural memory)
    Goal(GoalArgs),

    /// Retrieve and report on procedures (procedural memory)
    Procedure(ProcedureArgs),

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
    /// No short form: `-p` is the global --profile flag, and claiming it here
    /// made `boswell query` panic on every invocation.
    #[arg(long)]
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
    /// JSON file containing claims to assert.
    ///
    /// Positional, matching `boswell validate <FILE>` — the two are documented
    /// as a pair (`boswell validate f.json && boswell learn f.json`), so they
    /// take their file the same way.
    pub path: Option<String>,

    /// JSON file containing claims to assert (long form of the positional).
    ///
    /// No short form: `-f` is the global --format flag, and claiming it here
    /// made `boswell learn` panic on every invocation.
    #[arg(long)]
    pub file: Option<String>,

    /// JSON array of claims from stdin
    #[arg(long)]
    pub stdin: bool,

    /// Default tier for claims without explicit tier
    #[arg(short, long, value_enum, default_value = "task")]
    pub tier: TierArg,
}

impl LearnArgs {
    /// The file to read claims from, however it was given.
    ///
    /// Passing both forms is refused rather than silently preferring one: if
    /// they name different files, quietly loading one of them would assert a
    /// batch of claims the operator did not ask for.
    pub fn source_file(&self) -> Result<Option<&str>, &'static str> {
        match (self.path.as_deref(), self.file.as_deref()) {
            (Some(_), Some(_)) => Err("pass the file once: either as an argument or with --file"),
            (Some(p), None) | (None, Some(p)) => Ok(Some(p)),
            (None, None) => Ok(None),
        }
    }
}

/// Arguments for the validate command.
#[derive(Debug, Parser)]
pub struct ValidateArgs {
    /// JSON file to validate (a JSON array of claim definitions)
    pub file: Option<String>,

    /// Read the JSON array from stdin instead of a file
    #[arg(long)]
    pub stdin: bool,
}

/// Arguments for the forget command.
#[derive(Debug, Parser)]
pub struct ForgetArgs {
    /// Claim IDs to delete
    pub ids: Vec<String>,

    /// Read IDs from file (one per line)
    /// No short form: `-f` is the global --format flag, and claiming it here
    /// made `boswell forget` panic on every invocation.
    #[arg(long)]
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

    /// Restrict results to a namespace prefix
    #[arg(short, long)]
    pub namespace: Option<String>,

    /// Maximum number of results
    #[arg(short, long, default_value = "10")]
    pub limit: usize,

    /// Minimum similarity threshold (0.0-1.0)
    ///
    /// Defaults to 0.0: results come back ranked by similarity and capped by
    /// `--limit`, with each score shown. A non-zero floor is a model-specific
    /// tuning decision — cosine scores for short entity triples sit well below
    /// what the value intuitively suggests — so filtering is opt-in rather than
    /// silently hiding every result. Matches the gateway's `min_similarity`.
    #[arg(short, long, default_value = "0.0")]
    pub threshold: f64,
}

/// Arguments for the login command.
///
/// No short forms anywhere: `-c`, `-p`, `-f` are all taken by global flags, and
/// a subcommand that claims one panics at runtime rather than failing to
/// compile (see `the_command_tree_has_no_conflicting_flags`).
#[derive(Debug, Parser)]
pub struct LoginArgs {
    /// Issuer URL of the identity provider, overriding the config's [oidc]
    /// section (e.g. https://id.example.com)
    #[arg(long)]
    pub issuer: Option<String>,

    /// OAuth client id registered with the provider for this CLI
    #[arg(long)]
    pub client_id: Option<String>,

    /// Scope to request, repeatable or comma-separated. Replaces the
    /// configured scopes rather than adding to them.
    #[arg(long, value_delimiter = ',')]
    pub scope: Vec<String>,

    /// Describe the stored token instead of obtaining a new one. Never prints
    /// the token itself.
    #[arg(long)]
    pub status: bool,
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

    /// `--help` must be handled as a clap "error" we inspect, never with
    /// `parse_from`: on `--help` clap prints and calls `process::exit`, which
    /// tears down the whole test binary. Every test the harness had not yet run
    /// was then silently skipped while the suite still reported success.
    #[test]
    fn test_cli_parsing() {
        let err = Cli::try_parse_from(["boswell", "--help"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::DisplayHelp);

        // And a bare invocation really does parse to "no subcommand".
        let cli = Cli::try_parse_from(["boswell"]).expect("bare invocation parses");
        assert!(cli.command.is_none());
    }

    /// clap only validates a subcommand's arguments when that subcommand's
    /// parser is built, so a short-flag collision between a subcommand flag and
    /// a `global = true` one compiles fine and panics at runtime the first time
    /// someone runs it. `debug_assert` walks the whole tree, turning that into a
    /// test failure instead of a user's crash. Both `procedure report` and
    /// `goal expand` shipped such a collision before this test existed.
    #[test]
    fn the_command_tree_has_no_conflicting_flags() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }

    /// `docs/importing-personal-memory.md` documents `boswell learn f.json` and
    /// pairs it with `boswell validate f.json` in a single command line, so the
    /// documented form has to parse. It did not: `learn` took only `--file`,
    /// which was masked for a long time by the command panicking outright.
    #[test]
    fn learn_accepts_the_documented_positional_file() {
        let cli = Cli::try_parse_from(["boswell", "learn", "memories.json"])
            .expect("the documented invocation must parse");
        let Some(Command::Learn(args)) = cli.command else {
            panic!("expected a learn command");
        };
        assert_eq!(args.source_file().unwrap(), Some("memories.json"));
    }

    /// The long form keeps working, so anything scripted against it still runs.
    #[test]
    fn learn_still_accepts_the_long_form() {
        let cli = Cli::try_parse_from(["boswell", "learn", "--file", "memories.json"]).unwrap();
        let Some(Command::Learn(args)) = cli.command else {
            panic!("expected a learn command");
        };
        assert_eq!(args.source_file().unwrap(), Some("memories.json"));
    }

    /// Given both, `learn` refuses rather than picking one. If they name
    /// different files, quietly loading either would assert a batch of claims
    /// the operator never asked for.
    #[test]
    fn learn_refuses_two_files() {
        let cli = Cli::try_parse_from(["boswell", "learn", "a.json", "--file", "b.json"]).unwrap();
        let Some(Command::Learn(args)) = cli.command else {
            panic!("expected a learn command");
        };
        assert!(args.source_file().is_err());
    }

    /// A failure attribution only means something alongside a failure, and
    /// getting it wrong mis-files the outcome, so the enum spellings the
    /// instance expects are pinned here (design §3.3).
    #[test]
    fn outcome_and_failure_modes_use_the_wire_spellings() {
        let cli = Cli::parse_from([
            "boswell",
            "procedure",
            "report",
            "01890000-0000-7000-8000-000000000000",
            "--outcome",
            "failure",
            "--failure-mode",
            "executor-error",
        ]);
        let Some(Command::Procedure(args)) = cli.command else {
            panic!("expected a procedure command");
        };
        let ProcedureAction::Report {
            outcome,
            failure_mode,
            ..
        } = args.action
        else {
            panic!("expected a report action");
        };
        assert_eq!(outcome.as_str(), "failure");
        assert_eq!(failure_mode.unwrap().as_str(), "executor_error");
    }

    /// Context tags rank a hop; they are accepted both repeated and
    /// comma-separated so an operator can pass a situation either way.
    #[test]
    fn expand_accepts_context_tags_either_way() {
        let split = Cli::parse_from([
            "boswell",
            "goal",
            "expand",
            "01890000-0000-7000-8000-000000000000",
            "--context",
            "time:quick,ldl:low",
        ]);
        let Some(Command::Goal(args)) = split.command else {
            panic!("expected a goal command");
        };
        let GoalAction::Expand { context, .. } = args.action else {
            panic!("expected an expand action");
        };
        assert_eq!(context, vec!["time:quick", "ldl:low"]);

        let repeated = Cli::parse_from([
            "boswell",
            "goal",
            "expand",
            "01890000-0000-7000-8000-000000000000",
            "--context",
            "time:quick",
            "--context",
            "ldl:low",
        ]);
        let Some(Command::Goal(args)) = repeated.command else {
            panic!("expected a goal command");
        };
        let GoalAction::Expand { context, .. } = args.action else {
            panic!("expected an expand action");
        };
        assert_eq!(context, vec!["time:quick", "ldl:low"]);
    }

    /// `search` must not apply a similarity floor by default. A non-zero default
    /// silently returned "No matching claims found" for every query, because
    /// cosine scores on short entity triples sit below it.
    #[test]
    fn test_search_has_no_default_similarity_floor() {
        let cli = Cli::parse_from(["boswell", "search", "anything"]);
        let Some(Command::Search(args)) = cli.command else {
            panic!("expected a search command");
        };
        assert_eq!(args.threshold, 0.0);
        assert_eq!(args.limit, 10);
    }

    /// An explicit threshold is still honored.
    #[test]
    fn test_search_threshold_is_overridable() {
        let cli = Cli::parse_from(["boswell", "search", "q", "--threshold", "0.42"]);
        let Some(Command::Search(args)) = cli.command else {
            panic!("expected a search command");
        };
        assert_eq!(args.threshold, 0.42);
    }

    /// `assert` must carry both bounds so the write path can store a real
    /// interval rather than a collapsed point (ADR-003).
    #[test]
    fn test_assert_preserves_both_confidence_bounds() {
        let cli = Cli::parse_from([
            "boswell",
            "assert",
            "person:jd",
            "rel:uses",
            "lang:rust",
            "-l",
            "0.8",
            "-u",
            "0.95",
        ]);
        let Some(Command::Assert(args)) = cli.command else {
            panic!("expected an assert command");
        };
        assert_eq!(args.confidence_lower, 0.8);
        assert_eq!(args.confidence_upper, 0.95);
        assert!(
            args.confidence_lower < args.confidence_upper,
            "the bounds must stay distinct, not be averaged into a point"
        );
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

/// Arguments for the goal command.
#[derive(Debug, Parser)]
pub struct GoalArgs {
    #[command(subcommand)]
    pub action: GoalAction,
}

/// Goal-traversal actions (design 15 §3.2, §4.1).
///
/// Traversal is stateless: you hold the cursor. `list` finds an entry goal,
/// `expand` shows one level, and you re-run `expand` on whichever child you
/// pick until a candidate is a procedure. None of it issues a receipt.
#[derive(Debug, Subcommand)]
pub enum GoalAction {
    /// Find goals by namespace or intent — the entry hop into a decomposition
    List {
        /// Filter by namespace prefix
        #[arg(short, long)]
        namespace: Option<String>,

        /// Filter by a case-insensitive substring of the goal's intent
        #[arg(short, long)]
        intent_contains: Option<String>,

        /// Maximum results
        #[arg(short, long)]
        limit: Option<u32>,
    },

    /// Show one goal by id
    Show {
        /// Goal id (UUIDv7)
        id: String,
    },

    /// Expand one goal into its ranked candidates — a single traversal hop
    Expand {
        /// Goal id (UUIDv7)
        id: String,

        /// Situational context tags, repeatable or comma-separated
        /// (e.g. --context time:quick --context ldl:low).
        /// No short form: `-c` is the global --config flag.
        #[arg(long, value_delimiter = ',')]
        context: Vec<String>,
    },
}

/// Arguments for the procedure command.
#[derive(Debug, Parser)]
pub struct ProcedureArgs {
    #[command(subcommand)]
    pub action: ProcedureAction,
}

/// Procedure retrieval and outcome reporting (design 15 §3.3).
///
/// Retrieval is **not free**: every procedure handed out carries an execution
/// receipt, and the principal it was issued to is obliged to answer it with
/// `procedure report` before it expires. An unanswered receipt counts as
/// `unknown` against the procedure — silence is not success.
#[derive(Debug, Subcommand)]
pub enum ProcedureAction {
    /// Retrieve procedures for a goal or intent — ISSUES A RECEIPT FOR EACH
    List {
        /// Filter by exact goal grouping key
        #[arg(short, long)]
        goal: Option<String>,

        /// Filter by namespace prefix
        #[arg(short, long)]
        namespace: Option<String>,

        /// Filter by a case-insensitive substring of the procedure's intent
        #[arg(short, long)]
        intent_contains: Option<String>,

        /// Include superseded (non-current) versions
        #[arg(long)]
        include_superseded: bool,

        /// Maximum results
        #[arg(short, long)]
        limit: Option<u32>,

        /// The principal the receipts are issued to — who is on the hook to
        /// report. Defaults to the active profile's instance id.
        #[arg(long = "as")]
        as_principal: Option<String>,

        /// Correlation: task id, stamped onto the issued receipts
        #[arg(long)]
        task_id: Option<String>,

        /// Correlation: session id, stamped onto the issued receipts
        #[arg(long)]
        session_id: Option<String>,
    },

    /// Fetch one procedure by id — ISSUES A RECEIPT
    Show {
        /// Procedure id (UUIDv7)
        id: String,

        /// The principal the receipt is issued to
        #[arg(long = "as")]
        as_principal: Option<String>,

        /// Correlation: task id
        #[arg(long)]
        task_id: Option<String>,

        /// Correlation: session id
        #[arg(long)]
        session_id: Option<String>,
    },

    /// Answer an outstanding execution receipt
    Report {
        /// Receipt id (UUIDv7) being answered
        receipt_id: String,

        /// How the run ended
        #[arg(short, long, value_enum)]
        outcome: OutcomeArg,

        /// Failure attribution. Valid only with --outcome failure.
        /// No short form: `-f` is the global --format flag.
        #[arg(long, value_enum)]
        failure_mode: Option<FailureModeArg>,

        /// Names the step that failed (use with --failure-mode step-failed)
        #[arg(long)]
        failed_step: Option<String>,

        /// How confident the executor is in this report (0.0-1.0)
        #[arg(long)]
        executor_confidence: Option<f64>,

        /// What the run cost
        #[arg(long)]
        cost: Option<f64>,

        /// Free-text notes
        #[arg(long)]
        notes: Option<String>,
    },
}

/// How an execution ended.
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum OutcomeArg {
    /// The procedure achieved its postconditions
    Success,
    /// The procedure was run and did not achieve them
    Failure,
    /// The run was given up before reaching an outcome
    Abandoned,
}

impl OutcomeArg {
    /// The wire form.
    pub fn as_str(&self) -> &'static str {
        match self {
            OutcomeArg::Success => "success",
            OutcomeArg::Failure => "failure",
            OutcomeArg::Abandoned => "abandoned",
        }
    }
}

/// Who or what a failure is attributed to (design §3.3). The attribution
/// matters: `executor-error` leaves the procedure's counters alone, and
/// `preconditions-stale` flags the precondition check rather than the body.
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum FailureModeArg {
    /// The preconditions no longer held — blames the check, not the body
    PreconditionsStale,
    /// A step of the procedure failed
    StepFailed,
    /// The procedure ran but produced a bad result
    BadResult,
    /// The executor got it wrong — does NOT demote the procedure
    ExecutorError,
}

impl FailureModeArg {
    /// The wire form.
    pub fn as_str(&self) -> &'static str {
        match self {
            FailureModeArg::PreconditionsStale => "preconditions_stale",
            FailureModeArg::StepFailed => "step_failed",
            FailureModeArg::BadResult => "bad_result",
            FailureModeArg::ExecutorError => "executor_error",
        }
    }
}
