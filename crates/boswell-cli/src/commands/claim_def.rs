//! Shared claim-definition schema for the `learn` and `validate` commands.
//!
//! This is the JSON shape accepted by `boswell learn <file>` and checked by
//! `boswell validate <file>`: a JSON array of these objects.

use crate::error::{CliError, Result};
use serde::Deserialize;

/// A claim as provided in a JSON file for `learn` / `validate`.
#[derive(Debug, Deserialize)]
pub struct ClaimDefinition {
    /// Subject entity, formatted `namespace:value`.
    pub subject: String,
    /// Predicate entity, formatted `namespace:value`.
    pub predicate: String,
    /// Object entity, formatted `namespace:value`.
    pub object: String,
    /// Confidence interval; defaults to `{ lower: 0.5, upper: 1.0 }` when omitted.
    #[serde(default = "default_confidence")]
    pub confidence: ConfidenceDef,
    /// Tier name; when omitted, the command's default tier is used.
    #[serde(default)]
    pub tier: Option<String>,
}

/// Confidence interval for a [`ClaimDefinition`].
#[derive(Debug, Deserialize)]
pub struct ConfidenceDef {
    /// Lower bound, in `[0.0, 1.0]`.
    #[serde(default = "default_lower")]
    pub lower: f64,
    /// Upper bound, in `[0.0, 1.0]`.
    #[serde(default = "default_upper")]
    pub upper: f64,
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

/// Tier names accepted in a claim definition.
pub const VALID_TIERS: [&str; 4] = ["ephemeral", "task", "project", "permanent"];

impl ClaimDefinition {
    /// Convert into a domain [`Claim`](boswell_domain::Claim), applying
    /// `default_tier` when no tier is specified. Fails on the first problem.
    pub fn into_claim(self, default_tier: boswell_domain::Tier) -> Result<boswell_domain::Claim> {
        let (subject_ns, subject_val) = parse_entity(&self.subject)?;
        let (predicate_ns, predicate_val) = parse_entity(&self.predicate)?;
        let (object_ns, object_val) = parse_entity(&self.object)?;

        let tier_str = self
            .tier
            .unwrap_or_else(|| default_tier.as_str().to_string());

        let created_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Re-join as canonical "namespace:value".
        let subject = format!("{}:{}", subject_ns, subject_val);
        let predicate = format!("{}:{}", predicate_ns, predicate_val);
        let object = format!("{}:{}", object_ns, object_val);

        Ok(boswell_domain::Claim {
            id: boswell_domain::ClaimId::new(),
            namespace: subject_ns,
            subject,
            predicate,
            object,
            source_type: boswell_domain::Claim::SOURCE_ASSERTION.to_string(),
            confidence: (self.confidence.lower, self.confidence.upper),
            tier: tier_str,
            created_at,
            stale_at: None,
        })
    }

    /// Collect *all* validation problems with this claim, returning an empty
    /// vector when it is valid. Used by `validate` to report every issue at once
    /// rather than failing on the first.
    pub fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();

        for (name, value) in [
            ("subject", &self.subject),
            ("predicate", &self.predicate),
            ("object", &self.object),
        ] {
            if let Err(msg) = validate_entity(value) {
                problems.push(format!("{name} {msg}"));
            }
        }

        let (lower, upper) = (self.confidence.lower, self.confidence.upper);
        if !(0.0..=1.0).contains(&lower) {
            problems.push(format!("confidence.lower {lower} is outside [0.0, 1.0]"));
        }
        if !(0.0..=1.0).contains(&upper) {
            problems.push(format!("confidence.upper {upper} is outside [0.0, 1.0]"));
        }
        if lower > upper {
            problems.push(format!(
                "confidence.lower {lower} is greater than confidence.upper {upper}"
            ));
        }

        if let Some(tier) = &self.tier {
            if !VALID_TIERS.contains(&tier.as_str()) {
                problems.push(format!("tier '{tier}' is not one of {VALID_TIERS:?}"));
            }
        }

        problems
    }
}

/// Check that an entity string is in `namespace:value` form with non-empty parts.
pub fn validate_entity(input: &str) -> std::result::Result<(), String> {
    let parts: Vec<&str> = input.splitn(2, ':').collect();
    if parts.len() != 2 {
        return Err(format!(
            "'{input}' is not in namespace:value format (missing ':')"
        ));
    }
    if parts[0].is_empty() || parts[1].is_empty() {
        return Err(format!("'{input}' has an empty namespace or value"));
    }
    Ok(())
}

/// Parse an entity string into `(namespace, value)`; errors if malformed.
fn parse_entity(input: &str) -> Result<(String, String)> {
    validate_entity(input).map_err(CliError::InvalidInput)?;
    let parts: Vec<&str> = input.splitn(2, ':').collect();
    Ok((parts[0].to_string(), parts[1].to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_entity() {
        assert!(validate_entity("user:alice").is_ok());
        assert!(validate_entity("no-colon").is_err());
        assert!(validate_entity(":empty-ns").is_err());
        assert!(validate_entity("empty-val:").is_err());
    }

    #[test]
    fn test_parse_entity() {
        let (ns, val) = parse_entity("user:alice").unwrap();
        assert_eq!(ns, "user");
        assert_eq!(val, "alice");
    }

    #[test]
    fn test_claim_definition_parsing_defaults() {
        let json = r#"{ "subject": "user:alice", "predicate": "likes:coffee", "object": "beverage:espresso" }"#;
        let def: ClaimDefinition = serde_json::from_str(json).unwrap();
        assert_eq!(def.subject, "user:alice");
        assert_eq!(def.confidence.lower, 0.5);
        assert_eq!(def.confidence.upper, 1.0);
        assert!(def.tier.is_none());
    }

    #[test]
    fn test_problems_on_valid_claim_is_empty() {
        let def = ClaimDefinition {
            subject: "person:jd".to_string(),
            predicate: "rel:uses".to_string(),
            object: "lang:rust".to_string(),
            confidence: ConfidenceDef {
                lower: 0.8,
                upper: 0.95,
            },
            tier: Some("project".to_string()),
        };
        assert!(def.problems().is_empty());
    }

    #[test]
    fn test_problems_flags_all_issues() {
        let def = ClaimDefinition {
            subject: "no-colon".to_string(), // bad entity
            predicate: "rel:uses".to_string(),
            object: "lang:rust".to_string(),
            confidence: ConfidenceDef {
                lower: 0.9,
                upper: 0.4,
            }, // lower > upper
            tier: Some("bogus".to_string()), // bad tier
        };
        let problems = def.problems();
        assert_eq!(problems.len(), 3, "problems: {problems:?}");
    }

    #[test]
    fn test_confidence_out_of_range_flagged() {
        let def = ClaimDefinition {
            subject: "person:jd".to_string(),
            predicate: "rel:uses".to_string(),
            object: "lang:rust".to_string(),
            confidence: ConfidenceDef {
                lower: -0.1,
                upper: 1.5,
            },
            tier: None,
        };
        assert_eq!(def.problems().len(), 2);
    }
}
