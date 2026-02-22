//! LLM prompt engineering for claim extraction

use crate::types::ClaimSummary;

/// Builds prompts for the LLM to extract claims
pub struct PromptBuilder {
    text: String,
    namespace: String,
    existing_claims: Vec<ClaimSummary>,
}

impl PromptBuilder {
    /// Create a new prompt builder
    pub fn new(text: String, namespace: String) -> Self {
        Self {
            text,
            namespace,
            existing_claims: Vec::new(),
        }
    }
    
    /// Add existing claims as context for deduplication
    pub fn with_existing_claims(mut self, claims: Vec<ClaimSummary>) -> Self {
        self.existing_claims = claims;
        self
    }
    
    /// Build the complete extraction prompt
    pub fn build(&self) -> String {
        let mut prompt = String::new();
        
        // 1. Instruction and format specification
        prompt.push_str(EXTRACTION_INSTRUCTIONS);
        prompt.push_str("\n\n");
        
        // 2. Namespace context
        prompt.push_str(&format!("Target namespace: {}\n", self.namespace));
        prompt.push_str(&format!("Domain: {}\n\n", self.infer_domain()));
        
        // 3. Deduplication hints (if any)
        if !self.existing_claims.is_empty() {
            prompt.push_str("Existing claims in this namespace (avoid duplicating):\n");
            for claim in self.existing_claims.iter().take(20) {
                prompt.push_str(&format!(
                    "- {} {} {} ({:.2}, {:.2})\n",
                    claim.subject,
                    claim.predicate,
                    claim.object,
                    claim.confidence.0,
                    claim.confidence.1
                ));
            }
            prompt.push_str("\n");
        }
        
        // 4. The text to analyze
        prompt.push_str("Text to analyze:\n");
        prompt.push_str("---\n");
        prompt.push_str(&self.text);
        prompt.push_str("\n---\n\n");
        
        // 5. Output format reminder
        prompt.push_str(OUTPUT_FORMAT_REMINDER);
        
        prompt
    }
    
    /// Infer the domain from the namespace
    fn infer_domain(&self) -> String {
        let parts: Vec<&str> = self.namespace.split(':').collect();
        if parts.is_empty() {
            return "General knowledge".to_string();
        }
        
        match parts[0] {
            "person" => "Personal information",
            "project" => "Project documentation",
            "company" => "Company information",
            "engineering" => "Software engineering",
            "research" => "Research and academic content",
            "medical" => "Medical information",
            "legal" => "Legal documentation",
            _ => "General knowledge",
        }.to_string()
    }
}

const EXTRACTION_INSTRUCTIONS: &str = r#"Extract discrete, atomic claims from the following text.
Each claim should follow this format:

{
  "subject": "entity:identifier",
  "predicate": "relationship_type",
  "object": "entity:value or literal:value",
  "confidence_lower": 0.0-1.0,
  "confidence_upper": 0.0-1.0,
  "raw_expression": "exact text from source"
}

Rules:
- One idea per claim
- Subject/object must be namespaced (e.g., "person:john_doe", "company:acme", "date:2025-01-01")
- Preserve nuance in raw_expression (the exact words from the source)
- Include temporal context when present ("as of Q3 2025", "since 2019")
- Flag uncertainty in confidence - if the source hedges ("approximately", "reportedly"), reflect that in the confidence interval
- Use lower confidence for uncertain statements and higher confidence for definitive statements
- Typical confidence ranges:
  - Speculative/rumored: (0.3, 0.5)
  - Uncertain/approximate: (0.5, 0.7)
  - Generally accepted: (0.7, 0.85)
  - Well-documented: (0.85, 0.95)
  - Definitive/factual: (0.95, 0.98)
- Extract relationships between entities, not just entity properties
- For dates, use format "date:YYYY-MM-DD"
- For numbers, use format "number:value" or include units like "measurement:10kg""#;

const OUTPUT_FORMAT_REMINDER: &str = r#"Output format (JSON array only, no additional text):
[
  {
    "subject": "entity:identifier",
    "predicate": "relationship",
    "object": "entity:value",
    "confidence_lower": 0.0-1.0,
    "confidence_upper": 0.0-1.0,
    "raw_expression": "exact text"
  }
]

Remember: Return ONLY valid JSON, no markdown code blocks, no explanations."#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prompt_includes_namespace() {
        let builder = PromptBuilder::new(
            "Test text".to_string(),
            "test:namespace".to_string(),
        );
        
        let prompt = builder.build();
        assert!(prompt.contains("test:namespace"));
        assert!(prompt.contains("Target namespace:"));
    }

    #[test]
    fn test_prompt_includes_text() {
        let builder = PromptBuilder::new(
            "Alice works at Acme Corp".to_string(),
            "test:ns".to_string(),
        );
        
        let prompt = builder.build();
        assert!(prompt.contains("Alice works at Acme Corp"));
    }

    #[test]
    fn test_prompt_includes_existing_claims() {
        let existing = vec![
            ClaimSummary {
                subject: "person:bob".to_string(),
                predicate: "works_at".to_string(),
                object: "company:acme".to_string(),
                confidence: (0.9, 0.95),
            },
        ];
        
        let builder = PromptBuilder::new(
            "Test text".to_string(),
            "test:ns".to_string(),
        ).with_existing_claims(existing);
        
        let prompt = builder.build();
        assert!(prompt.contains("Existing claims"));
        assert!(prompt.contains("person:bob"));
        assert!(prompt.contains("works_at"));
        assert!(prompt.contains("company:acme"));
    }

    #[test]
    fn test_prompt_includes_instructions() {
        let builder = PromptBuilder::new(
            "Test text".to_string(),
            "test:ns".to_string(),
        );
        
        let prompt = builder.build();
        assert!(prompt.contains("Extract discrete, atomic claims"));
        assert!(prompt.contains("confidence_lower"));
        assert!(prompt.contains("raw_expression"));
    }

    #[test]
    fn test_domain_inference() {
        let builder = PromptBuilder::new(
            "Test".to_string(),
            "engineering:project".to_string(),
        );
        assert_eq!(builder.infer_domain(), "Software engineering");
        
        let builder = PromptBuilder::new(
            "Test".to_string(),
            "medical:records".to_string(),
        );
        assert_eq!(builder.infer_domain(), "Medical information");
        
        let builder = PromptBuilder::new(
            "Test".to_string(),
            "unknown:type".to_string(),
        );
        assert_eq!(builder.infer_domain(), "General knowledge");
    }

    #[test]
    fn test_limits_existing_claims_to_20() {
        let existing: Vec<_> = (0..50)
            .map(|i| ClaimSummary {
                subject: format!("entity:{}", i),
                predicate: "relation".to_string(),
                object: "value".to_string(),
                confidence: (0.9, 0.95),
            })
            .collect();
        
        let builder = PromptBuilder::new(
            "Test".to_string(),
            "test:ns".to_string(),
        ).with_existing_claims(existing);
        
        let prompt = builder.build();
        // Should only include first 20
        assert!(prompt.contains("entity:0"));
        assert!(prompt.contains("entity:19"));
        assert!(!prompt.contains("entity:20"));
        assert!(!prompt.contains("entity:49"));
    }
}
