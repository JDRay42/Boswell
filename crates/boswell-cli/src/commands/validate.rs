//! Validate command: offline check of a `learn` JSON file before loading it.

use crate::cli::ValidateArgs;
use crate::commands::claim_def::ClaimDefinition;
use crate::error::{CliError, Result};
use crate::output::Formatter;
use std::fs;
use std::io::{self, Read};

/// Execute the validate command.
///
/// Reads a JSON array of claim definitions (from a file or stdin), checks each
/// against the `learn` schema, and reports every problem. Requires no server
/// connection. Returns an error (non-zero exit) if any claim is invalid, so it
/// can gate a load in a script.
pub async fn execute_validate(args: ValidateArgs, formatter: &Formatter) -> Result<()> {
    let json_data = if args.stdin {
        let mut buffer = String::new();
        io::stdin().read_to_string(&mut buffer)?;
        buffer
    } else if let Some(file_path) = &args.file {
        fs::read_to_string(file_path)?
    } else {
        return Err(CliError::InvalidInput(
            "Must specify a file path or --stdin".to_string(),
        ));
    };

    // Parse the array; give a precise message on malformed JSON.
    let defs: Vec<ClaimDefinition> = match serde_json::from_str(&json_data) {
        Ok(defs) => defs,
        Err(e) => {
            println!(
                "{}",
                formatter.error(&format!(
                    "Invalid JSON at line {}, column {}: {}",
                    e.line(),
                    e.column(),
                    e
                ))
            );
            return Err(CliError::InvalidInput(
                "file is not a valid JSON array of claims".to_string(),
            ));
        }
    };

    if defs.is_empty() {
        println!("{}", formatter.warning("No claims found in the file."));
        return Ok(());
    }

    let mut invalid = 0;
    for (i, def) in defs.iter().enumerate() {
        let problems = def.problems();
        if problems.is_empty() {
            continue;
        }
        invalid += 1;
        println!(
            "{}",
            formatter.error(&format!(
                "claim #{i} ({} {} {}):",
                def.subject, def.predicate, def.object
            ))
        );
        for problem in problems {
            println!("    - {problem}");
        }
    }

    let total = defs.len();
    let valid = total - invalid;
    println!();

    if invalid == 0 {
        println!(
            "{}",
            formatter.success(&format!("All {total} claims are valid and ready to learn."))
        );
        Ok(())
    } else {
        println!(
            "{}",
            formatter.error(&format!(
                "{invalid} of {total} claims have problems ({valid} valid)."
            ))
        );
        Err(CliError::InvalidInput(format!(
            "{invalid} invalid claim(s)"
        )))
    }
}
