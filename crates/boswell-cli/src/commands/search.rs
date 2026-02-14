//! Search command implementation.

use crate::cli::SearchArgs;
use crate::error::{CliError, Result};
use crate::output::Formatter;
use boswell_sdk::BoswellClient;

/// Execute the search command.
pub async fn execute_search(
    args: SearchArgs,
    _client: &mut BoswellClient,
    formatter: &Formatter,
) -> Result<()> {
    // Validate parameters
    if args.threshold < 0.0 || args.threshold > 1.0 {
        return Err(CliError::InvalidInput(
            "Threshold must be between 0.0 and 1.0".to_string(),
        ));
    }

    // Note: Semantic search is not yet implemented in the SDK
    // This is a placeholder that will be implemented when the SDK exposes HNSW search
    println!(
        "{}",
        formatter.warning("Semantic search is not yet available in the SDK")
    );
    println!("{}", formatter.info("Use the 'query' command for exact filtering"));
    println!();
    println!("Once implemented, this will search for:");
    println!("  Query: {}", args.query);
    println!("  Limit: {}", args.limit);
    println!("  Threshold: {}", args.threshold);

    Ok(())
}
