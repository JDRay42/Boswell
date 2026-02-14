//! Query command implementation.

use crate::cli::QueryArgs;
use crate::error::{CliError, Result};
use crate::output::Formatter;
use boswell_sdk::{BoswellClient, QueryFilter};

/// Execute the query command.
pub async fn execute_query(
    args: QueryArgs,
    client: &mut BoswellClient,
    formatter: &Formatter,
) -> Result<()> {
    let mut filter = QueryFilter::default();

    // Apply subject filter
    if let Some(subject) = args.subject {
        filter.subject = Some(subject);
    }

    // Apply predicate filter
    if let Some(predicate) = args.predicate {
        filter.predicate = Some(predicate);
    }

    // Apply object filter
    if let Some(object) = args.object {
        filter.object = Some(object);
    }

    // Apply tier filter
    if let Some(tier) = args.tier {
        filter.tier = Some(tier.into());
    }

    // Apply confidence filter
    if let Some(min_conf) = args.min_confidence {
        if min_conf < 0.0 || min_conf > 1.0 {
            return Err(CliError::InvalidInput(
                "Confidence must be between 0.0 and 1.0".to_string(),
            ));
        }
        filter.min_confidence = Some(min_conf);
    }

    // Execute query
    let claims = client.query(filter).await?;

    // Display results
    println!("{}", formatter.format_claims(&claims)?);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_query_filter_construction() {
        let filter = QueryFilter::default();
        assert!(filter.subject.is_none());
        assert!(filter.predicate.is_none());
    }
}
