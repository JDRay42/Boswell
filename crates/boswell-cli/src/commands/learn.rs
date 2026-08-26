//! Learn command implementation.

use crate::cli::LearnArgs;
use crate::commands::claim_def::ClaimDefinition;
use crate::error::{CliError, Result};
use crate::output::Formatter;
use boswell_sdk::BoswellClient;
use std::fs;
use std::io::{self, Read};

/// Execute the learn command.
pub async fn execute_learn(
    args: LearnArgs,
    client: &mut BoswellClient,
    formatter: &Formatter,
) -> Result<()> {
    // Read claims from file or stdin
    let json_data = if args.stdin {
        let mut buffer = String::new();
        io::stdin().read_to_string(&mut buffer)?;
        buffer
    } else if let Some(file_path) = args.file {
        fs::read_to_string(file_path)?
    } else {
        return Err(CliError::InvalidInput(
            "Must specify either --file or --stdin".to_string(),
        ));
    };

    // Parse claims
    let claim_defs: Vec<ClaimDefinition> = serde_json::from_str(&json_data)?;

    if claim_defs.is_empty() {
        return Err(CliError::InvalidInput("No claims provided".to_string()));
    }

    // Convert to domain claims
    let default_tier: boswell_domain::Tier = args.tier.into();
    let claims: Vec<boswell_domain::Claim> = claim_defs
        .into_iter()
        .map(|def| def.into_claim(default_tier))
        .collect::<Result<Vec<_>>>()?;

    let claim_count = claims.len();

    // Assert all claims
    let _response = client.learn(claims).await?;

    println!("{}", formatter.bulk_result("Learned", claim_count));

    Ok(())
}
