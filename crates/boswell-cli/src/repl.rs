//! Interactive REPL (Read-Eval-Print Loop) mode.

use crate::cli::{AssertArgs, Command, ConnectArgs, ForgetArgs, LearnArgs, ProfileAction, ProfileArgs, QueryArgs, SearchArgs, TierArg};
use crate::commands;
use crate::config::Config;
use crate::error::{CliError, Result};
use crate::output::Formatter;
use boswell_sdk::BoswellClient;
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;
use std::path::PathBuf;

/// Run the interactive REPL.
pub async fn run_repl(config: &mut Config, formatter: &Formatter) -> Result<()> {
    println!("{}", formatter.info("Boswell REPL - Type 'help' for commands, 'exit' to quit"));
    println!();

    // Initialize readline editor
    let mut editor = DefaultEditor::new().map_err(|e| CliError::Io(std::io::Error::new(
        std::io::ErrorKind::Other,
        format!("Failed to initialize editor: {}", e),
    )))?;

    // Load history
    let history_path = get_history_path()?;
    let _ = editor.load_history(&history_path);

    let mut client: Option<BoswellClient> = None;

    loop {
        let prompt = if client.is_some() {
            "boswell> "
        } else {
            "boswell (disconnected)> "
        };

        match editor.readline(prompt) {
            Ok(line) => {
                let line = line.trim();
                
                if line.is_empty() {
                    continue;
                }

                editor.add_history_entry(line).ok();

                // Parse command
                match parse_repl_command(line) {
                    Ok(ReplCommand::Exit) => {
                        println!("{}", formatter.info("Goodbye!"));
                        break;
                    }
                    Ok(ReplCommand::Help) => {
                        print_help(formatter);
                    }
                    Ok(ReplCommand::Command(cmd)) => {
                        if let Err(e) = execute_repl_command(cmd, &mut client, config, formatter).await {
                            eprintln!("{}", formatter.error(&e.to_string()));
                        }
                    }
                    Err(e) => {
                        eprintln!("{}", formatter.error(&e.to_string()));
                    }
                }
            }
            Err(ReadlineError::Interrupted) => {
                println!("{}", formatter.info("Use 'exit' to quit"));
            }
            Err(ReadlineError::Eof) => {
                break;
            }
            Err(err) => {
                eprintln!("{}", formatter.error(&format!("Error: {}", err)));
                break;
            }
        }
    }

    // Save history
    editor.save_history(&history_path).ok();

    Ok(())
}

/// REPL command type.
enum ReplCommand {
    Exit,
    Help,
    Command(Command),
}

/// Parse a REPL command line.
fn parse_repl_command(line: &str) -> Result<ReplCommand> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    
    if parts.is_empty() {
        return Err(CliError::InvalidInput("Empty command".to_string()));
    }

    match parts[0] {
        "exit" | "quit" | "q" => Ok(ReplCommand::Exit),
        "help" | "?" => Ok(ReplCommand::Help),
        "connect" => parse_connect_command(&parts[1..]),
        "assert" => parse_assert_command(&parts[1..]),
        "query" => parse_query_command(&parts[1..]),
        "learn" => parse_learn_command(&parts[1..]),
        "forget" => parse_forget_command(&parts[1..]),
        "search" => parse_search_command(&parts[1..]),
        "profile" => parse_profile_command(&parts[1..]),
        _ => Err(CliError::InvalidInput(format!(
            "Unknown command: {}. Type 'help' for available commands.",
            parts[0]
        ))),
    }
}

/// Execute a REPL command.
async fn execute_repl_command(
    cmd: Command,
    client: &mut Option<BoswellClient>,
    config: &mut Config,
    formatter: &Formatter,
) -> Result<()> {
    match cmd {
        Command::Connect(args) => {
            let new_client = commands::execute_connect(args, config, formatter).await?;
            *client = Some(new_client);
        }
        Command::Profile(args) => {
            commands::execute_profile(args, config, formatter).await?;
        }
        _ => {
            let client_ref = client.as_mut().ok_or(CliError::NotConnected)?;
            
            match cmd {
                Command::Assert(args) => {
                    commands::execute_assert(args, client_ref, formatter).await?;
                }
                Command::Query(args) => {
                    commands::execute_query(args, client_ref, formatter).await?;
                }
                Command::Learn(args) => {
                    commands::execute_learn(args, client_ref, formatter).await?;
                }
                Command::Forget(args) => {
                    commands::execute_forget(args, client_ref, formatter).await?;
                }
                Command::Search(args) => {
                    commands::execute_search(args, client_ref, formatter).await?;
                }
                _ => unreachable!(),
            }
        }
    }
    
    Ok(())
}

// Simple command parsers for REPL (minimal argument parsing)

fn parse_connect_command(args: &[&str]) -> Result<ReplCommand> {
    let url = args.get(0).map(|s| s.to_string());
    let instance = args.get(1).map(|s| s.to_string());
    
    Ok(ReplCommand::Command(Command::Connect(ConnectArgs {
        url,
        instance,
        namespace: None,
        save_as: None,
    })))
}

fn parse_assert_command(args: &[&str]) -> Result<ReplCommand> {
    if args.len() < 3 {
        return Err(CliError::InvalidInput(
            "Usage: assert <subject> <predicate> <object> [confidence_lower] [confidence_upper] [tier]".to_string(),
        ));
    }

    let confidence_lower = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0.5);
    let confidence_upper = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(1.0);
    let tier = args.get(5).and_then(parse_tier_arg).unwrap_or(TierArg::Task);

    Ok(ReplCommand::Command(Command::Assert(AssertArgs {
        subject: args[0].to_string(),
        predicate: args[1].to_string(),
        object: args[2].to_string(),
        confidence_lower,
        confidence_upper,
        tier,
    })))
}

fn parse_query_command(args: &[&str]) -> Result<ReplCommand> {
    // Simple query - just subject filter for now
    // Format: query [subject:value]
    let subject = args.get(0).map(|s| s.to_string());

    Ok(ReplCommand::Command(Command::Query(QueryArgs {
        subject,
        predicate: None,
        object: None,
        tier: None,
        min_confidence: None,
        limit: Some(20),
    })))
}

fn parse_learn_command(args: &[&str]) -> Result<ReplCommand> {
    if args.is_empty() {
        return Err(CliError::InvalidInput("Usage: learn <file>".to_string()));
    }

    Ok(ReplCommand::Command(Command::Learn(LearnArgs {
        file: Some(args[0].to_string()),
        stdin: false,
        tier: TierArg::Task,
    })))
}

fn parse_forget_command(args: &[&str]) -> Result<ReplCommand> {
    if args.is_empty() {
        return Err(CliError::InvalidInput("Usage: forget <id1> [id2] [id3] ...".to_string()));
    }

    Ok(ReplCommand::Command(Command::Forget(ForgetArgs {
        ids: args.iter().map(|s| s.to_string()).collect(),
        file: None,
        stdin: false,
        yes: false,
    })))
}

fn parse_search_command(args: &[&str]) -> Result<ReplCommand> {
    if args.is_empty() {
        return Err(CliError::InvalidInput("Usage: search <query>".to_string()));
    }

    Ok(ReplCommand::Command(Command::Search(SearchArgs {
        query: args.join(" "),
        limit: 10,
        threshold: 0.7,
    })))
}

fn parse_profile_command(args: &[&str]) -> Result<ReplCommand> {
    if args.is_empty() {
        return Ok(ReplCommand::Command(Command::Profile(ProfileArgs {
            action: ProfileAction::Show,
        })));
    }

    let action = match args[0] {
        "list" => ProfileAction::List,
        "show" => ProfileAction::Show,
        "switch" => {
            if args.len() < 2 {
                return Err(CliError::InvalidInput("Usage: profile switch <name>".to_string()));
            }
            ProfileAction::Switch {
                name: args[1].to_string(),
            }
        }
        _ => return Err(CliError::InvalidInput(format!("Unknown profile action: {}", args[0]))),
    };

    Ok(ReplCommand::Command(Command::Profile(ProfileArgs { action })))
}

fn parse_tier_arg(s: &&str) -> Option<TierArg> {
    match s.to_lowercase().as_str() {
        "ephemeral" => Some(TierArg::Ephemeral),
        "task" => Some(TierArg::Task),
        "project" => Some(TierArg::Project),
        "permanent" => Some(TierArg::Permanent),
        _ => None,
    }
}

fn get_history_path() -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| CliError::Config("Could not find home directory".into()))?;
    let boswell_dir = home.join(".boswell");
    std::fs::create_dir_all(&boswell_dir)?;
    Ok(boswell_dir.join("history.txt"))
}

fn print_help(formatter: &Formatter) {
    println!("{}", formatter.info("Available commands:"));
    println!();
    println!("  connect [url] [instance]      - Connect to Boswell router");
    println!("  assert <s> <p> <o> [l] [u] [t] - Assert a claim");
    println!("    s: subject (namespace:value)");
    println!("    p: predicate (namespace:value)");
    println!("    o: object (namespace:value)");
    println!("    l: confidence lower (default: 0.5)");
    println!("    u: confidence upper (default: 1.0)");
    println!("    t: tier (ephemeral|task|project|permanent, default: task)");
    println!("  query [subject]                - Query claims");
    println!("  learn <file>                   - Learn claims from JSON file");
    println!("  forget <id> [id2] [id3]        - Delete claims by ID");
    println!("  search <query>                 - Semantic search (not yet implemented)");
    println!("  profile [list|show|switch]     - Manage profiles");
    println!("  help, ?                        - Show this help");
    println!("  exit, quit, q                  - Exit REPL");
    println!();
}
