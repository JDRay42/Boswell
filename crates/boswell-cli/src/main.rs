//! Boswell CLI - Command-line interface for the Boswell cognitive memory system.

use boswell_cli::{Cli, Command, Config, Formatter};
use boswell_cli::commands;
use boswell_cli::repl;
use clap::Parser;

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

async fn run() -> boswell_cli::Result<()> {
    // Parse CLI arguments
    let cli = Cli::parse();

    // Load or create config
    let mut config = Config::load().unwrap_or_else(|_| {
        let cfg = Config::default();
        cfg.save().ok();
        cfg
    });

    // Override profile if specified
    if let Some(profile_name) = cli.profile {
        config.switch_profile(profile_name)?;
    }

    // Determine output format
    let format = cli
        .format
        .map(Into::into)
        .unwrap_or(config.settings.format);

    // Determine color setting
    let color_enabled = !cli.no_color && config.settings.color;

    // Create formatter
    let formatter = Formatter::new(format, color_enabled);

    // Handle commands
    match cli.command {
        None | Some(Command::Repl) => {
            // Enter REPL mode
            repl::run_repl(&mut config, &formatter).await?;
        }
        Some(Command::Connect(args)) => {
            commands::execute_connect(args, &mut config, &formatter).await?;
        }
        Some(Command::Profile(args)) => {
            commands::execute_profile(args, &mut config, &formatter).await?;
        }
        Some(cmd) => {
            // Commands that require a connection
            let profile = config.get_active_profile()?;
            let mut client = boswell_sdk::BoswellClient::new(&profile.router_url);
            client.connect().await?;

            match cmd {
                Command::Assert(args) => {
                    commands::execute_assert(args, &mut client, &formatter).await?;
                }
                Command::Query(args) => {
                    commands::execute_query(args, &mut client, &formatter).await?;
                }
                Command::Learn(args) => {
                    commands::execute_learn(args, &mut client, &formatter).await?;
                }
                Command::Forget(args) => {
                    commands::execute_forget(args, &mut client, &formatter).await?;
                }
                Command::Search(args) => {
                    commands::execute_search(args, &mut client, &formatter).await?;
                }
                _ => unreachable!(),
            }
        }
    }

    Ok(())
}
