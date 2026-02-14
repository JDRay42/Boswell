//! Forget command implementation.

use crate::cli::ForgetArgs;
use crate::error::{CliError, Result};
use crate::output::Formatter;
use boswell_domain::ClaimId;
use boswell_sdk::BoswellClient;
use std::fs;
use std::io::{self, BufRead};

/// Execute the forget command.
pub async fn execute_forget(
    args: ForgetArgs,
    client: &mut BoswellClient,
    formatter: &Formatter,
) -> Result<()> {
    // Collect IDs from various sources
    let mut ids = args.ids.clone();

    // Read from file if specified
    if let Some(file_path) = &args.file {
        let file_ids = read_ids_from_file(file_path)?;
        ids.extend(file_ids);
    }

    // Read from stdin if specified
    if args.stdin {
        let stdin_ids = read_ids_from_stdin()?;
        ids.extend(stdin_ids);
    }

    if ids.is_empty() {
        return Err(CliError::InvalidInput("No claim IDs provided".to_string()));
    }

    // Parse IDs
    let claim_ids: Vec<ClaimId> = ids
        .iter()
        .map(|id| ClaimId::from_string(id).map_err(|e| CliError::InvalidInput(format!("Invalid ID '{}': {}", id, e))))
        .collect::<Result<Vec<_>>>()?;

    // Confirm deletion unless --yes is specified
    if !args.yes {
        println!("About to delete {} claim(s):", claim_ids.len());
        for id in &claim_ids {
            println!("  - {}", id);
        }
        print!("Continue? [y/N] ");
        
        let mut response = String::new();
        io::stdin().read_line(&mut response)?;
        
        if !response.trim().eq_ignore_ascii_case("y") {
            println!("{}", formatter.info("Operation cancelled"));
            return Ok(());
        }
    }

    // Delete claims
    let success = client.forget(claim_ids).await?;
    
    if success {
        println!("{}", formatter.bulk_result("Deleted", ids.len()));
    } else {
        println!("{}", formatter.warning("Some claims could not be deleted"));
    }

    Ok(())
}

/// Read IDs from a file (one per line).
fn read_ids_from_file(path: &str) -> Result<Vec<String>> {
    let content = fs::read_to_string(path)?;
    Ok(content.lines().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
}

/// Read IDs from stdin (one per line).
fn read_ids_from_stdin() -> Result<Vec<String>> {
    let stdin = io::stdin();
    let mut ids = Vec::new();
    
    for line in stdin.lock().lines() {
        let line = line?;
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            ids.push(trimmed.to_string());
        }
    }
    
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_read_ids_from_file() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "01HN5Z3K8QYWG9V2B1MXFK8RWE").unwrap();
        writeln!(file, "01HN5Z3K8QYWG9V2B1MXFK8RWF").unwrap();
        writeln!(file, "").unwrap(); // Empty line should be ignored
        writeln!(file, "  01HN5Z3K8QYWG9V2B1MXFK8RWG  ").unwrap(); // Whitespace should be trimmed
        
        let ids = read_ids_from_file(file.path().to_str().unwrap()).unwrap();
        assert_eq!(ids.len(), 3);
        assert_eq!(ids[2], "01HN5Z3K8QYWG9V2B1MXFK8RWG");
    }
}
