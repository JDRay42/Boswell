//! Provenance tracking (per ADR-009)

/// A single provenance entry tracking the source of a claim
#[derive(Debug, Clone, PartialEq)]
pub struct ProvenanceEntry {
    /// Source identifier (e.g., "user:alice", "agent:gpt4", "synthesis:v1")
    pub source: String,
    
    /// Timestamp when this provenance was recorded
    pub timestamp: u64,
    
    /// Optional rationale or reasoning
    pub rationale: Option<String>,
    
    /// Source type (e.g., "user", "agent", "extraction", "synthesis")
    pub source_type: String,
}

impl ProvenanceEntry {
    /// Create a new provenance entry
    pub fn new(source: String, timestamp: u64, source_type: String) -> Self {
        Self {
            source,
            timestamp,
            rationale: None,
            source_type,
        }
    }

    /// Create a provenance entry with rationale
    pub fn with_rationale(mut self, rationale: String) -> Self {
        self.rationale = Some(rationale);
        self
    }
}
