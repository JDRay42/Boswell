//! Prompt construction for cluster analysis.

use boswell_domain::Claim;

/// Builds the LLM prompt that asks whether a cluster of claims implies a
/// higher-order insight.
///
/// The prompt deliberately emphasises that "no insight" is a valid and common
/// outcome, favouring quality over quantity (per the architecture doc).
pub struct PromptBuilder<'a> {
    namespace: &'a str,
    claims: &'a [Claim],
}

impl<'a> PromptBuilder<'a> {
    /// Create a new prompt builder for a cluster of claims.
    pub fn new(namespace: &'a str, claims: &'a [Claim]) -> Self {
        Self { namespace, claims }
    }

    /// Render the full prompt string.
    pub fn build(&self) -> String {
        let mut prompt = String::new();

        prompt.push_str(
            "You are the Synthesizer for a cognitive memory system. You examine a \
cluster of related claims and decide whether they TOGETHER imply a single \
higher-order insight that no individual claim states on its own.\n\n",
        );

        prompt.push_str(
            "A claim is a (subject, predicate, object) triple with a confidence \
interval. An insight is a NEW claim that generalises, connects, or abstracts the \
cluster — a pattern, trend, or principle.\n\n",
        );

        prompt.push_str("Rules:\n");
        prompt.push_str("- \"No insight\" is a valid and common answer. Only report an insight if it is genuinely supported by the cluster.\n");
        prompt.push_str("- The insight must be a single triple, not a restatement of one of the input claims.\n");
        prompt.push_str("- Assess your confidence in the INFERENCE (how strongly the cluster implies the insight), as an interval [lower, upper] within [0.0, 1.0].\n");
        prompt.push_str("- Keep subject/predicate/object concise. Reuse the namespace's entity style where possible.\n\n");

        prompt.push_str(&format!("Namespace: {}\n\n", self.namespace));
        prompt.push_str("Cluster claims:\n");
        for (i, claim) in self.claims.iter().enumerate() {
            prompt.push_str(&format!(
                "{}. ({}) {} — {} — {}  [confidence {:.2}–{:.2}]\n",
                i + 1,
                claim.tier,
                claim.subject,
                claim.predicate,
                claim.object,
                claim.confidence.0,
                claim.confidence.1,
            ));
        }

        prompt.push_str(
            "\nRespond with a single JSON object and nothing else.\n\
If there is a genuine insight:\n\
{\"insight\": true, \"subject\": \"...\", \"predicate\": \"...\", \"object\": \"...\", \
\"confidence_lower\": 0.0, \"confidence_upper\": 0.0, \"rationale\": \"...\"}\n\
If there is no insight:\n\
{\"insight\": false, \"rationale\": \"...\"}\n",
        );

        prompt
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use boswell_domain::{Claim, ClaimId};

    fn claim(subject: &str, predicate: &str, object: &str) -> Claim {
        Claim {
            id: ClaimId::new(),
            namespace: "eng:team".to_string(),
            subject: subject.to_string(),
            predicate: predicate.to_string(),
            object: object.to_string(),
            source_type: "assertion".to_string(),
            confidence: (0.7, 0.9),
            tier: "project".to_string(),
            created_at: 0,
            stale_at: None,
        }
    }

    #[test]
    fn test_prompt_includes_all_claims() {
        let claims = vec![
            claim("person:alice", "works_on", "project:atlas"),
            claim("person:bob", "works_on", "project:atlas"),
        ];
        let prompt = PromptBuilder::new("eng:team", &claims).build();

        assert!(prompt.contains("person:alice"));
        assert!(prompt.contains("person:bob"));
        assert!(prompt.contains("project:atlas"));
        assert!(prompt.contains("eng:team"));
    }

    #[test]
    fn test_prompt_mentions_no_insight_is_valid() {
        let claims = vec![claim("a", "b", "c")];
        let prompt = PromptBuilder::new("ns", &claims).build();
        assert!(prompt.to_lowercase().contains("no insight"));
        assert!(prompt.contains("\"insight\": false"));
    }
}
