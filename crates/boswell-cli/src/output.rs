//! Output formatting for the CLI.

use crate::config::OutputFormat;
use crate::error::Result;
use boswell_domain::{ChildRef, Claim, ClaimId, ExpandResult, ExpandedCandidate, Goal, Tier};
use boswell_sdk::{IssuedProcedure, ReportOutcomeResponse};
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
        self.format_claims(std::slice::from_ref(claim))
    }

    /// Format semantic-search results (claims paired with similarity scores).
    pub fn format_search_results(&self, hits: &[(Claim, f32)]) -> Result<String> {
        match self.format {
            OutputFormat::Json => self.format_search_json(hits),
            OutputFormat::Table => self.format_search_table(hits),
            OutputFormat::Quiet => {
                let ids: Vec<String> = hits.iter().map(|(c, _)| c.id.to_string()).collect();
                Ok(ids.join("\n"))
            }
        }
    }

    /// Format search results as JSON.
    fn format_search_json(&self, hits: &[(Claim, f32)]) -> Result<String> {
        let json: Vec<serde_json::Value> = hits
            .iter()
            .map(|(c, similarity)| {
                serde_json::json!({
                    "id": c.id.to_string(),
                    "namespace": c.namespace,
                    "subject": c.subject,
                    "predicate": c.predicate,
                    "object": c.object,
                    "confidence": { "lower": c.confidence.0, "upper": c.confidence.1 },
                    "tier": c.tier,
                    "source_type": c.source_type,
                    "similarity": similarity,
                })
            })
            .collect();
        Ok(serde_json::to_string_pretty(&json)?)
    }

    /// Format search results as a table with a Similarity column.
    fn format_search_table(&self, hits: &[(Claim, f32)]) -> Result<String> {
        if hits.is_empty() {
            return Ok(self.colorize("No matching claims found.", "yellow"));
        }

        let mut builder = Builder::default();
        builder.push_record([
            "Similarity",
            "ID",
            "Namespace",
            "Subject",
            "Predicate",
            "Object",
            "Tier",
        ]);

        for (claim, similarity) in hits {
            builder.push_record([
                &format!("{:.3}", similarity),
                &claim.id.to_string()[..8],
                &claim.namespace,
                &claim.subject,
                &claim.predicate,
                &claim.object,
                &claim.tier,
            ]);
        }

        let mut table = builder.build();
        table
            .with(Style::rounded())
            .with(Modify::new(Rows::first()).with(Alignment::center()));

        Ok(table.to_string())
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
        builder.push_record([
            "ID",
            "Namespace",
            "Subject",
            "Predicate",
            "Object",
            "Confidence",
            "Tier",
        ]);

        for claim in claims {
            let confidence = format!("[{:.2}, {:.2}]", claim.confidence.0, claim.confidence.1);
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

    // ---- Procedural memory (design 15 §3.2, §3.3, §4.1) ----

    /// Format a list of goals.
    pub fn format_goals(&self, goals: &[Goal]) -> Result<String> {
        match self.format {
            OutputFormat::Json => {
                let json: Vec<serde_json::Value> = goals.iter().map(goal_json).collect();
                Ok(serde_json::to_string_pretty(&json)?)
            }
            OutputFormat::Quiet => Ok(goals
                .iter()
                .map(|g| g.id.to_string())
                .collect::<Vec<_>>()
                .join("\n")),
            OutputFormat::Table => {
                if goals.is_empty() {
                    return Ok(self.colorize("No goals found.", "yellow"));
                }
                let mut builder = Builder::default();
                builder.push_record(["ID", "Namespace", "Name", "Intent", "Tier"]);
                for g in goals {
                    builder.push_record([
                        &g.id.to_string()[..8],
                        &g.namespace,
                        &g.name,
                        &g.intent,
                        g.tier.as_str(),
                    ]);
                }
                let mut table = builder.build();
                table
                    .with(Style::rounded())
                    .with(Modify::new(Rows::first()).with(Alignment::center()));
                Ok(table.to_string())
            }
        }
    }

    /// Format one traversal hop.
    ///
    /// Decision aids and factor readings are shown alongside the candidates
    /// rather than folded into them: the store surfaces and ranks, but the
    /// weighting is the operator's (§4.1). The readings are what let them see
    /// *why* a candidate surfaced instead of taking the ranking on faith.
    pub fn format_expansion(&self, goal_id: &str, result: &ExpandResult) -> Result<String> {
        if let OutputFormat::Json = self.format {
            return Ok(serde_json::to_string_pretty(&serde_json::json!({
                "goal_id": goal_id,
                "candidates": result.candidates.iter().map(candidate_json).collect::<Vec<_>>(),
                "decision_aids": result.decision_aids.iter().map(candidate_json).collect::<Vec<_>>(),
                "factor_readings": result.factor_readings.iter().map(|f| serde_json::json!({
                    "subject": f.subject,
                    "predicate": f.predicate,
                    "object": f.object,
                    "confidence": { "lower": f.confidence.0, "upper": f.confidence.1 },
                })).collect::<Vec<_>>(),
            }))?);
        }

        if let OutputFormat::Quiet = self.format {
            return Ok(result
                .candidates
                .iter()
                .map(child_id_string)
                .collect::<Vec<_>>()
                .join("\n"));
        }

        let mut out = String::new();

        if result.candidates.is_empty() {
            out.push_str(&self.colorize(
                "No candidates: every child was filtered out, or this goal is a leaf.",
                "yellow",
            ));
        } else {
            out.push_str(&self.colorize("Candidates", "green"));
            out.push('\n');
            out.push_str(&candidate_table(&result.candidates));
        }

        if !result.decision_aids.is_empty() {
            out.push_str("\n\n");
            out.push_str(
                &self.colorize("Decision aids (procedures that help you choose)", "green"),
            );
            out.push('\n');
            out.push_str(&candidate_table(&result.decision_aids));
        }

        if !result.factor_readings.is_empty() {
            out.push_str("\n\n");
            out.push_str(&self.colorize("Factors read", "green"));
            out.push('\n');
            let mut builder = Builder::default();
            builder.push_record(["Subject", "Predicate", "Object", "Confidence"]);
            for f in &result.factor_readings {
                builder.push_record([
                    f.subject.clone(),
                    f.predicate.clone(),
                    f.object.clone(),
                    format!("[{:.2}, {:.2}]", f.confidence.0, f.confidence.1),
                ]);
            }
            let mut table = builder.build();
            table
                .with(Style::rounded())
                .with(Modify::new(Rows::first()).with(Alignment::center()));
            out.push_str(&table.to_string());
        }

        Ok(out)
    }

    /// Format issued procedures together with the receipts they oblige.
    ///
    /// The receipt is printed as loudly as the procedure: retrieving a
    /// procedure creates an obligation to report, and an operator who walks
    /// away silently has the run counted as `unknown` against it (§3.3).
    pub fn format_issued_procedures(&self, issued: &[IssuedProcedure]) -> Result<String> {
        match self.format {
            OutputFormat::Json => {
                let json: Vec<serde_json::Value> = issued
                    .iter()
                    .map(|d| {
                        serde_json::json!({
                            "procedure": procedure_json(&d.procedure),
                            "receipt": {
                                "receipt_id": d.receipt.receipt_id.to_string(),
                                "procedure_id": d.receipt.procedure_id.to_string(),
                                "version": d.receipt.version,
                                "issued_to": d.receipt.issued_to,
                                "task_id": d.receipt.task_id,
                                "session_id": d.receipt.session_id,
                                "issued_at": d.receipt.issued_at,
                                "expires_at": d.receipt.expires_at,
                                "required": ["outcome"],
                                "optional": ["failure_mode", "executor_confidence", "cost", "notes"],
                            },
                        })
                    })
                    .collect();
                Ok(serde_json::to_string_pretty(&json)?)
            }
            OutputFormat::Quiet => Ok(issued
                .iter()
                .map(|d| d.receipt.receipt_id.to_string())
                .collect::<Vec<_>>()
                .join("\n")),
            OutputFormat::Table => {
                if issued.is_empty() {
                    return Ok(self.colorize("No procedures found.", "yellow"));
                }

                let mut builder = Builder::default();
                builder.push_record([
                    "Receipt",
                    "Procedure",
                    "Name",
                    "Ver",
                    "Tier",
                    "Effectiveness",
                    "Expires (unix ms)",
                ]);
                for d in issued {
                    let p = &d.procedure;
                    builder.push_record([
                        d.receipt.receipt_id.to_string(),
                        p.id.to_string()[..8].to_string(),
                        p.name.clone(),
                        p.version.to_string(),
                        p.tier.as_str().to_string(),
                        format!("{}/{} ok", p.success_count, p.use_count),
                        d.receipt.expires_at.to_string(),
                    ]);
                }
                let mut table = builder.build();
                table
                    .with(Style::rounded())
                    .with(Modify::new(Rows::first()).with(Alignment::center()));

                let issued_to = &issued[0].receipt.issued_to;
                Ok(format!(
                    "{}\n{}",
                    table,
                    self.colorize(
                        &format!(
                            "! {} receipt(s) issued to '{}'. You are obliged to answer each one with\n                               `boswell procedure report <receipt> --outcome ...` before it expires.\n                               An unanswered receipt counts as `unknown` — silence is not success.",
                            issued.len(),
                            issued_to
                        ),
                        "yellow"
                    )
                ))
            }
        }
    }

    /// Format the result of answering a receipt.
    pub fn format_report_outcome(&self, resp: &ReportOutcomeResponse) -> Result<String> {
        if let OutputFormat::Json = self.format {
            return Ok(serde_json::to_string_pretty(&serde_json::json!({
                "accepted": resp.accepted,
                "already_final": resp.already_final,
                "applied": resp.applied,
                "quarantined": resp.quarantined,
                "effect": {
                    "counted_as_success": resp.counted_as_success,
                    "counted_as_failure": resp.counted_as_failure,
                    "attributed_to_executor": resp.attributed_to_executor,
                    "flagged_precondition_stale": resp.flagged_precondition_stale,
                },
                "message": resp.message,
            }))?);
        }

        if let OutputFormat::Quiet = self.format {
            return Ok(if resp.applied {
                "applied"
            } else {
                "not-applied"
            }
            .to_string());
        }

        let mut out = if resp.already_final {
            self.warning("Receipt was already answered or has expired; nothing was applied.")
        } else if resp.quarantined {
            // Not an error: this is the trust model working. A self-reported
            // failure against a shared procedure needs corroboration before it
            // can move that procedure's counters (§3.3).
            self.warning(
                "Report recorded but QUARANTINED: the reporter's assurance is too low for this \n                 procedure's tier, so one executor cannot tank a shared how-to.",
            )
        } else if resp.applied {
            self.success("Report applied.")
        } else {
            self.warning(&resp.message)
        };

        let mut effects = Vec::new();
        if resp.counted_as_success {
            effects.push("counted as success");
        }
        if resp.counted_as_failure {
            effects.push("counted as failure");
        }
        if resp.attributed_to_executor {
            effects.push("attributed to the executor, not the procedure");
        }
        if resp.flagged_precondition_stale {
            effects.push("flagged the precondition check, not the body");
        }
        if !effects.is_empty() {
            out.push('\n');
            out.push_str(&self.info(&effects.join("; ")));
        }

        Ok(out)
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
        let msg = format!("Connected to {} (instance: {})", router_url, instance_id);
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

// ---- Procedural-memory formatting helpers ----

/// The child's id as a string, whichever kind it is.
fn child_id_string(c: &ExpandedCandidate) -> String {
    match c.child {
        ChildRef::Goal(id) => id.to_string(),
        ChildRef::Procedure(id) => id.to_string(),
    }
}

fn goal_json(g: &Goal) -> serde_json::Value {
    serde_json::json!({
        "id": g.id.to_string(),
        "namespace": g.namespace,
        "name": g.name,
        "intent": g.intent,
        "definition_of_done": g.definition_of_done,
        "tier": g.tier.as_str(),
        "created_at": g.created_at,
        "updated_at": g.updated_at,
        "stale_at": g.stale_at,
    })
}

fn candidate_json(c: &ExpandedCandidate) -> serde_json::Value {
    serde_json::json!({
        "child_kind": c.child.kind().as_str(),
        "child_id": child_id_string(c),
        "role": c.role.as_str(),
        "context_tags": c.context_tags,
        "usage_notes": c.usage_notes,
        "effectiveness": c.effectiveness,
        "context_match": c.context_match,
    })
}

fn procedure_json(p: &boswell_domain::Procedure) -> serde_json::Value {
    serde_json::json!({
        "id": p.id.to_string(),
        "namespace": p.namespace,
        "name": p.name,
        "version": p.version,
        "goal": p.goal,
        "intent": p.intent,
        "body_format": p.body_format.as_str(),
        "body": p.body,
        "parameters": p.parameters.iter().map(|param| serde_json::json!({
            "name": param.name,
            "type": param.type_name,
            "default": param.default,
            "desc": param.desc,
        })).collect::<Vec<_>>(),
        "preconditions": p.preconditions.iter().map(|pc| serde_json::json!({
            "kind": pc.kind,
            "description": pc.description,
        })).collect::<Vec<_>>(),
        "required_tools": p.required_tools,
        "postconditions": p.postconditions,
        "usage_notes": p.usage_notes,
        "tier": p.tier.as_str(),
        "effectiveness": {
            "use_count": p.use_count,
            "success_count": p.success_count,
            "failure_count": p.failure_count,
            "unknown_count": p.unknown_count,
        },
    })
}

/// Render candidates as a table.
///
/// `Kind` is the column that tells the operator what to do next: `goal` means
/// expand again, `procedure` means this is a runnable leaf.
fn candidate_table(candidates: &[ExpandedCandidate]) -> String {
    let mut builder = Builder::default();
    builder.push_record([
        "Kind",
        "Child",
        "Effectiveness",
        "Ctx",
        "Tags",
        "Usage notes",
    ]);
    for c in candidates {
        builder.push_record([
            c.child.kind().as_str().to_string(),
            child_id_string(c),
            format!("{:.2}", c.effectiveness),
            c.context_match.to_string(),
            c.context_tags.join(", "),
            c.usage_notes.clone(),
        ]);
    }
    let mut table = builder.build();
    table
        .with(Style::rounded())
        .with(Modify::new(Rows::first()).with(Alignment::center()));
    table.to_string()
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
            source_type: "assertion".to_string(),
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
