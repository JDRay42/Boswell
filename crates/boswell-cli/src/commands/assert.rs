//! Assert command implementation.

use crate::cli::AssertArgs;
use crate::error::{CliError, Result};
use crate::output::Formatter;
use boswell_sdk::BoswellClient;

/// Execute the assert command.
pub async fn execute_assert(
    args: AssertArgs,
    client: &mut BoswellClient,
    formatter: &Formatter,
) -> Result<()> {
    // Parse entities
    let (subject_ns, subject_val) = parse_entity_parts(&args.subject)?;
    let (predicate_ns, predicate_val) = parse_entity_parts(&args.predicate)?;
    let (object_ns, object_val) = parse_entity_parts(&args.object)?;

    // Validate confidence
    if args.confidence_lower > args.confidence_upper {
        return Err(CliError::InvalidInput(
            "Lower confidence must be <= upper confidence".to_string(),
        ));
    }
    if args.confidence_lower < 0.0 || args.confidence_upper > 1.0 {
        return Err(CliError::InvalidInput(
            "Confidence values must be between 0.0 and 1.0".to_string(),
        ));
    }

    // Use the subject namespace as the overall namespace
    let namespace = &subject_ns;
    let confidence = (args.confidence_lower + args.confidence_upper) / 2.0;
    let tier: boswell_domain::Tier = args.tier.into();

    // Format subject, predicate, object as "namespace:value"
    let subject_str = format!("{}:{}", subject_ns, subject_val);
    let predicate_str = format!("{}:{}", predicate_ns, predicate_val);
    let object_str = format!("{}:{}", object_ns, object_val);

    // Assert claim using SDK
    let claim_id = client.assert(
        namespace,
        &subject_str,
        &predicate_str,
        &object_str,
        Some(confidence),
        Some(tier),
    ).await?;

    println!("{}", formatter.claim_asserted(&claim_id));

    Ok(())
}

/// Parse an entity from string format "namespace:value" and return parts.
fn parse_entity_parts(input: &str) -> Result<(String, String)> {
    let parts: Vec<&str> = input.splitn(2, ':').collect();
    if parts.len() != 2 {
        return Err(CliError::InvalidInput(format!(
            "Invalid entity format '{}'. Expected 'namespace:value'",
            input
        )));
    }

    Ok((parts[0].to_string(), parts[1].to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_entity_parts() {
        let (ns, val) = parse_entity_parts("user:alice").unwrap();
        assert_eq!(ns, "user");
        assert_eq!(val, "alice");
    }

    #[test]
    fn test_parse_entity_with_colon_in_value() {
        let (ns, val) = parse_entity_parts("url:http://example.com").unwrap();
        assert_eq!(ns, "url");
        assert_eq!(val, "http://example.com");
    }

    #[test]
    fn test_parse_entity_invalid() {
        let result = parse_entity_parts("invalid");
        assert!(result.is_err());
    }
}
