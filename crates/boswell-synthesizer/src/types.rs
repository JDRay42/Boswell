//! Public types for the Synthesizer

use boswell_domain::{Claim, ClaimId};

/// Scope limiting which claims a synthesis pass considers.
#[derive(Debug, Clone)]
pub struct SynthesisScope {
    /// Limit to specific namespaces (prefix match). `None` = all namespaces.
    pub namespaces: Option<Vec<String>>,

    /// Minimum tier to consider (e.g. `"task"`). Claims below this tier are skipped.
    pub min_tier: String,

    /// Only consider claims created at or after this Unix timestamp (seconds).
    /// `None` = no lower bound.
    pub since: Option<u64>,

    /// Maximum number of clusters to evaluate in this pass.
    pub max_clusters: usize,
}

impl SynthesisScope {
    /// Create a scope covering all namespaces at or above the given tier.
    pub fn all(min_tier: impl Into<String>, max_clusters: usize) -> Self {
        Self {
            namespaces: None,
            min_tier: min_tier.into(),
            since: None,
            max_clusters,
        }
    }
}

/// A cluster of related claims that may together imply a higher-order insight.
#[derive(Debug, Clone)]
pub struct ClaimCluster {
    /// The claims making up the cluster.
    pub claims: Vec<Claim>,

    /// The maximum `derived_from` depth among the cluster's claims. A cluster
    /// built entirely from first-order claims has depth 0.
    pub max_depth: usize,
}

impl ClaimCluster {
    /// The number of claims in the cluster.
    pub fn size(&self) -> usize {
        self.claims.len()
    }

    /// The constituent claim IDs.
    pub fn claim_ids(&self) -> Vec<ClaimId> {
        self.claims.iter().map(|c| c.id).collect()
    }
}

/// A candidate insight parsed from an LLM response, before it becomes a claim.
#[derive(Debug, Clone, PartialEq)]
pub struct InsightCandidate {
    /// Subject of the derived claim.
    pub subject: String,

    /// Predicate of the derived claim.
    pub predicate: String,

    /// Object of the derived claim.
    pub object: String,

    /// The LLM's assessed confidence interval in the *inference itself*.
    pub llm_confidence: (f64, f64),

    /// The LLM's rationale for why the cluster implies this insight.
    pub rationale: String,
}

/// A synthesized insight that was created and persisted during a pass.
#[derive(Debug, Clone)]
pub struct SynthesizedInsight {
    /// The ID of the newly created derived claim.
    pub claim_id: ClaimId,

    /// Subject of the derived claim.
    pub subject: String,

    /// Predicate of the derived claim.
    pub predicate: String,

    /// Object of the derived claim.
    pub object: String,

    /// The propagated confidence interval assigned to the derived claim.
    pub confidence: (f64, f64),

    /// The claims this insight was derived from.
    pub derived_from: Vec<ClaimId>,

    /// The LLM's rationale.
    pub rationale: String,
}

/// The outcome of a single synthesis pass.
#[derive(Debug, Clone, Default)]
pub struct SynthesisReport {
    /// Total claims examined as synthesis candidates.
    pub claims_examined: usize,

    /// Number of clusters actually evaluated by the LLM.
    pub clusters_evaluated: usize,

    /// Number of clusters skipped for exceeding the derivation-depth limit.
    pub clusters_depth_skipped: usize,

    /// Number of new insight claims created.
    pub insights_created: usize,

    /// Number of candidate insights rejected (low confidence or validation).
    pub insights_rejected: usize,

    /// Wall-clock duration of the pass, in milliseconds.
    pub duration_ms: u64,

    /// The insights created during this pass. In `dry_run` mode these describe
    /// what *would* be created; nothing is written to the store.
    pub insights: Vec<SynthesizedInsight>,
}

impl SynthesisReport {
    /// A human-readable one-line summary of the pass.
    pub fn summary(&self) -> String {
        format!(
            "Synthesis pass: {} claims examined, {} clusters evaluated \
             ({} depth-skipped), {} insights created, {} rejected ({} ms)",
            self.claims_examined,
            self.clusters_evaluated,
            self.clusters_depth_skipped,
            self.insights_created,
            self.insights_rejected,
            self.duration_ms,
        )
    }
}
