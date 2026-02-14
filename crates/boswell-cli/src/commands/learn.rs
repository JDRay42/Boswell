//! Learn command implementation.

use crate::cli::LearnArgs;
use crate::error::{CliError, Result};
use crate::output::Formatter;
use boswell_sdk::BoswellClient;
use serde::Deserialize;
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
        .map(|def| def.to_claim(default_tier))
        .collect::<Result<Vec<_>>>()?;

    let claim_count = claims.len();

    // Assert all claims
    let _response = client.learn(claims).await?;

    println!("{}", formatter.bulk_result("Learned", claim_count));

    Ok(())
}

/// Simplified claim definition for JSON input.
#[derive(Debug, Deserialize)]
struct ClaimDefinition {
    subject: String,
    predicate: String,
    object: String,
    #[serde(default = "default_confidence")]
    confidence: ConfidenceDef,
    #[serde(default)]
    tier: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ConfidenceDef {
    #[serde(default = "default_lower")]
    lower: f64,
    #[serde(default = "default_upper")]
    upper: f64,
}

impl ClaimDefinition {
    fn to_claim(self, default_tier: boswell_domain::Tier) -> Result<boswell_domain::Claim> {
        let (subject_ns, subject_val) = parse_entity(&self.subject)?;
        let (predicate_ns, predicate_val) = parse_entity(&self.predicate)?;
        let (object_ns, object_val) = parse_entity(&self.object)?;

        let tier_str = if let Some(tier_str) = self.tier {
            tier_str
        } else {
            default_tier.as_str().to_string()
        };

        let created_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Format as "namespace:value"
        let subject = format!("{}:{}", subject_ns, subject_val);
        let predicate = format!("{}:{}", predicate_ns, predicate_val);
        let object = format!("{}:{}", object_ns, object_val);

        Ok(boswell_domain::Claim {
            id: boswell_domain::ClaimId::new(),
            namespace: subject_ns,
            subject,
            predicate,
            object,
            confidence: (self.confidence.lower, self.confidence.upper),
            tier: tier_str,
            created_at,
            stale_at: None,
        })
    }
}

fn parse_entity(input: &str) -> Result<(String, String)> {
    let parts: Vec<&str> = input.splitn(2, ':').collect();
    if parts.len() != 2 {
        return Err(CliError::InvalidInput(format!(
            "Invalid entity format '{}'. Expected 'namespace:value'",
            input
        )));
    }

    Ok((parts[0].to_string(), parts[1].to_string()))
}

fn default_confidence() -> ConfidenceDef {
    ConfidenceDef {
        lower: 0.5,
        upper: 1.0,
    }
}

fn default_lower() -> f64 {
    0.5
}

fn default_upper() -> f64 {
    1.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_entity() {
        let (ns, val) = parse_entity("user:alice").unwrap();
        assert_eq!(ns, "user");
        assert_eq!(val, "alice");
    }

    #[test]
    fn test_claim_definition_parsing() {
        let json = r#"
        {
            "subject": "user:alice",
            "predicate": "likes:coffee",
            "object": "beverage:espresso"
        }
        "#;
        
        let def: ClaimDefinition = serde_json::from_str(json).unwrap();
        assert_eq!(def.subject, "user:alice");
        assert_eq!(def.confidence.lower, 0.5);
        assert_eq!(def.confidence.upper, 1.0);
    }
}
