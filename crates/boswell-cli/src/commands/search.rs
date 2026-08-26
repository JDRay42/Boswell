//! Search command implementation.

use crate::cli::SearchArgs;
use crate::error::{CliError, Result};
use crate::output::Formatter;
use boswell_sdk::BoswellClient;

/// Execute the search command.
pub async fn execute_search(
    args: SearchArgs,
    client: &mut BoswellClient,
    formatter: &Formatter,
) -> Result<()> {
    // Validate parameters
    if args.threshold < 0.0 || args.threshold > 1.0 {
        return Err(CliError::InvalidInput(
            "Threshold must be between 0.0 and 1.0".to_string(),
        ));
    }

    let hits = client
        .search(
            &args.query,
            args.namespace.clone(),
            args.limit,
            args.threshold,
        )
        .await?;

    println!("{}", formatter.format_search_results(&hits)?);
    Ok(())
}
