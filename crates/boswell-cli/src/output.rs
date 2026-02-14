//! Output formatting for the CLI.

use crate::config::OutputFormat;
use crate::error::Result;
use boswell_domain::{Claim, ClaimId, Tier};
use colored::*;
use serde_json;
use tabled::{
    builder::Builder,
    settings::{object::Rows, Alignment, Modify, Style},
};

/// Output formatter.
pub struct Formatter {
    format: OutputFormat,
    color_enabled: bool,
}

impl Formatter {
    /// Create a new formatter.
    pub fn new(format: OutputFormat, color_enabled: bool) -> Self {
        Self {
            format,
            color_enabled,
        }
    }

    /// Format claims output.
    pub fn format_claims(&self, claims: &[Claim]) -> Result<String> {
        match self.format {
            OutputFormat::Json => self.format_claims_json(claims),
            OutputFormat::Table => self.format_claims_table(claims),
            OutputFormat::Quiet => self.format_claims_quiet(claims),
        }
    }

    /// Format a single claim.
    pub fn format_claim(&self, claim: &Claim) -> Result<String> {
        self.format_claims(&[claim.clone()])
    }

    /// Format claims as JSON.
    fn format_claims_json(&self, claims: &[Claim]) -> Result<String> {
        // Create a serializable representation
        let json_claims: Vec<serde_json::Value> = claims
            .iter()
            .map(|c| {
                serde_json::json!({
                    "id": c.id.to_string(),
                    "namespace": c.namespace,
                    "subject": c.subject,
                    "predicate": c.predicate,
                    "object": c.object,
                    "confidence": {
                        "lower": c.confidence.0,
                        "upper": c.confidence.1
                    },
                    "tier": c.tier,
                    "created_at": c.created_at,
                    "stale_at": c.stale_at
                })
            })
            .collect();

        Ok(serde_json::to_string_pretty(&json_claims)?)
    }

    /// Format claims as a table.
    fn format_claims_table(&self, claims: &[Claim]) -> Result<String> {
        if claims.is_empty() {
            return Ok(self.colorize("No claims found.", "yellow"));
        }

        let mut builder = Builder::default();
        builder.push_record(["ID", "Namespace", "Subject", "Predicate", "Object", "Confidence", "Tier"]);

        for claim in claims {
            let confidence = format!(
                "[{:.2}, {:.2}]",
                claim.confidence.0, claim.confidence.1
            );
            builder.push_record([
                &claim.id.to_string()[..8], // Truncate ID for readability
                &claim.namespace,
                &claim.subject,
                &claim.predicate,
                &claim.object,
                &confidence,
                &claim.tier,
            ]);
        }

        let mut table = builder.build();
        table
            .with(Style::rounded())
            .with(Modify::new(Rows::first()).with(Alignment::center()));

        Ok(table.to_string())
    }

    /// Format claims in quiet mode (IDs only).
    fn format_claims_quiet(&self, claims: &[Claim]) -> Result<String> {
        let ids: Vec<String> = claims.iter().map(|c| c.id.to_string()).collect();
        Ok(ids.join("\n"))
    }

    /// Format a success message.
    pub fn success(&self, message: &str) -> String {
        self.colorize(&format!("✓ {}", message), "green")
    }

    /// Format an error message.
    pub fn error(&self, message: &str) -> String {
        self.colorize(&format!("✗ {}", message), "red")
    }

    /// Format an info message.
    pub fn info(&self, message: &str) -> String {
        self.colorize(&format!("ℹ {}", message), "blue")
    }

    /// Format a warning message.
    pub fn warning(&self, message: &str) -> String {
        self.colorize(&format!("⚠ {}", message), "yellow")
    }

    /// Format connection info.
    pub fn connection_info(&self, router_url: &str, instance_id: &str) -> String {
        let msg = format!(
            "Connected to {} (instance: {})",
            router_url, instance_id
        );
        self.success(&msg)
    }

    /// Format claim assertion result.
    pub fn claim_asserted(&self, claim_id: &ClaimId) -> String {
        self.success(&format!("Claim asserted: {}", claim_id))
    }

    /// Format bulk operation result.
    pub fn bulk_result(&self, operation: &str, count: usize) -> String {
        self.success(&format!("{} {} claim(s)", operation, count))
    }

    /// Colorize text if color is enabled.
    fn colorize(&self, text: &str, color: &str) -> String {
        if !self.color_enabled {
            return text.to_string();
        }

        match color {
            "red" => text.red().to_string(),
            "green" => text.green().to_string(),
            "blue" => text.blue().to_string(),
            "yellow" => text.yellow().to_string(),
            "cyan" => text.cyan().to_string(),
            "magenta" => text.magenta().to_string(),
            _ => text.to_string(),
        }
    }
}

/// Format a tier enum value for display.
pub fn format_tier(tier: &str) -> Option<Tier> {
    match tier.to_lowercase().as_str() {
        "ephemeral" => Some(Tier::Ephemeral),
        "task" => Some(Tier::Task),
        "project" => Some(Tier::Project),
        "permanent" => Some(Tier::Permanent),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use boswell_domain::ClaimId;

    fn create_test_claim() -> Claim {
        Claim {
            id: ClaimId::new(),
            namespace: "test".to_string(),
            subject: "user:alice".to_string(),
            predicate: "likes:coffee".to_string(),
            object: "beverage:espresso".to_string(),
            confidence: (0.8, 0.9),
            tier: "task".to_string(),
            created_at: 12345678,
            stale_at: None,
        }
    }

    #[test]
    fn test_json_format() {
        let formatter = Formatter::new(OutputFormat::Json, false);
        let claims = vec![create_test_claim()];
        let output = formatter.format_claims(&claims).unwrap();
        assert!(output.contains("subject"));
        assert!(output.contains("predicate"));
    }

    #[test]
    fn test_quiet_format() {
        let formatter = Formatter::new(OutputFormat::Quiet, false);
        let claims = vec![create_test_claim()];
        let output = formatter.format_claims(&claims).unwrap();
        // Should just be the ID
        assert!(!output.contains("subject"));
        assert!(output.len() > 20); // ULID length
    }

    #[test]
    fn test_table_format() {
        let formatter = Formatter::new(OutputFormat::Table, false);
        let claims = vec![create_test_claim()];
        let output = formatter.format_claims(&claims).unwrap();
        assert!(output.contains("Subject"));
        assert!(output.contains("Confidence"));
    }

    #[test]
    fn test_empty_claims() {
        let formatter = Formatter::new(OutputFormat::Table, false);
        let output = formatter.format_claims(&[]).unwrap();
        assert!(output.contains("No claims found"));
    }

    #[test]
    fn test_colorize_disabled() {
        let formatter = Formatter::new(OutputFormat::Table, false);
        let msg = formatter.success("test");
        assert_eq!(msg, "✓ test");
    }

    #[test]
    fn test_tier_parsing() {
        assert!(matches!(format_tier("ephemeral"), Some(Tier::Ephemeral)));
        assert!(matches!(format_tier("Task"), Some(Tier::Task)));
        assert!(matches!(format_tier("Project"), Some(Tier::Project)));
        assert!(matches!(format_tier("PERMANENT"), Some(Tier::Permanent)));
        assert!(format_tier("invalid").is_none());
    }
}
