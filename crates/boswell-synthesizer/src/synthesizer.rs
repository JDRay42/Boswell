//! Core Synthesizer implementation.

use crate::clustering::build_clusters;
use crate::config::SynthesizerConfig;
use crate::confidence::propagate_confidence;
use crate::error::SynthesizerError;
use crate::parser::parse_insight_response;
use crate::prompt::PromptBuilder;
use crate::types::{ClaimCluster, InsightCandidate, SynthesisReport, SynthesisScope, SynthesizedInsight};
use boswell_domain::traits::{ClaimQuery, ClaimStore, LlmProvider};
use boswell_domain::{Claim, ClaimId, Relationship, RelationshipType, Tier};
use boswell_gatekeeper::{Gatekeeper, ValidationStatus};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::timeout;
use tracing::{debug, info, warn};

/// The Synthesizer discovers emergent, higher-order insights across the claim
/// graph and persists them as new `derived_from` claims.
///
/// It is intended to run as a background process (see
/// [`SynthesizerWorker`](crate::SynthesizerWorker)), but a single pass can be
/// driven directly via [`Synthesizer::run_pass`].
pub struct Synthesizer<L>
where
    L: LlmProvider,
{
    llm: Arc<L>,
    gatekeeper: Gatekeeper,
    config: SynthesizerConfig,
    model_name: String,
}

impl<L> Synthesizer<L>
where
    L: LlmProvider + Send + Sync + 'static,
    L::Error: std::fmt::Display,
{
    /// Create a new Synthesizer with the default Gatekeeper configuration.
    pub fn new(llm: L, config: SynthesizerConfig) -> Self {
        Self {
            llm: Arc::new(llm),
            gatekeeper: Gatekeeper::default_config(),
            config,
            model_name: "llm".to_string(),
        }
    }

    /// Override the Gatekeeper used to validate synthesized claims.
    pub fn with_gatekeeper(mut self, gatekeeper: Gatekeeper) -> Self {
        self.gatekeeper = gatekeeper;
        self
    }

    /// Set a human-readable model name (used only for logging).
    pub fn with_model_name(mut self, model_name: impl Into<String>) -> Self {
        self.model_name = model_name.into();
        self
    }

    /// Access the current configuration.
    pub fn config(&self) -> &SynthesizerConfig {
        &self.config
    }

    /// Run a single synthesis pass over the claims matching `scope`.
    ///
    /// Returns a [`SynthesisReport`] describing what was examined and created.
    /// When `config.enabled` is `false`, the pass is a no-op.
    pub async fn run_pass<S>(
        &self,
        store: &mut S,
        scope: SynthesisScope,
    ) -> Result<SynthesisReport, SynthesizerError>
    where
        S: ClaimStore,
        S::Error: std::fmt::Display,
    {
        let start = SystemTime::now();
        let mut report = SynthesisReport::default();

        if !self.config.enabled {
            debug!("Synthesizer disabled; skipping pass");
            return Ok(report);
        }

        // 1. Gather candidate claims across all qualifying tiers/namespaces.
        let candidates = self.gather_candidates(store, &scope)?;
        report.claims_examined = candidates.len();
        info!("Synthesis candidates: {}", candidates.len());

        if candidates.len() < self.config.min_cluster_size {
            report.duration_ms = elapsed_ms(start);
            return Ok(report);
        }

        // 2. Fetch relationships among candidates for clustering + depth.
        let relationships = self.gather_relationships(store, &candidates)?;

        // 3. Build clusters.
        let raw_clusters = build_clusters(
            candidates,
            &relationships,
            self.config.min_cluster_size,
            self.config.max_cluster_size,
        );
        debug!("Built {} candidate clusters", raw_clusters.len());

        // 4. Evaluate clusters (respecting max_clusters and depth limits).
        for claims in raw_clusters.into_iter().take(scope.max_clusters) {
            let max_depth = self.cluster_max_depth(store, &claims);
            let cluster = ClaimCluster { claims, max_depth };

            // Enforce the derivation-depth limit to prevent runaway synthesis.
            if max_depth >= self.config.max_derivation_depth {
                report.clusters_depth_skipped += 1;
                debug!(
                    "Skipping cluster at derivation depth {} (limit {})",
                    max_depth, self.config.max_derivation_depth
                );
                continue;
            }

            report.clusters_evaluated += 1;

            match self.analyze_cluster(&cluster).await {
                Ok(Some(candidate)) => {
                    match self.materialize_insight(store, &cluster, candidate) {
                        Ok(Some(insight)) => {
                            report.insights_created += 1;
                            report.insights.push(insight);
                        }
                        Ok(None) => report.insights_rejected += 1,
                        Err(e) => {
                            warn!("Failed to persist insight: {}", e);
                            report.insights_rejected += 1;
                        }
                    }
                }
                Ok(None) => {
                    debug!("No insight for cluster");
                }
                Err(e) => {
                    warn!("Cluster analysis failed: {}", e);
                }
            }
        }

        report.duration_ms = elapsed_ms(start);
        info!("{}", report.summary());
        Ok(report)
    }

    /// Query the store for all candidate claims matching the scope.
    fn gather_candidates<S>(
        &self,
        store: &S,
        scope: &SynthesisScope,
    ) -> Result<Vec<Claim>, SynthesizerError>
    where
        S: ClaimStore,
        S::Error: std::fmt::Display,
    {
        let tiers = tiers_at_or_above(&scope.min_tier);
        let namespaces: Vec<Option<String>> = match &scope.namespaces {
            Some(list) if !list.is_empty() => list.iter().cloned().map(Some).collect(),
            _ => vec![None],
        };

        let mut seen: HashSet<u128> = HashSet::new();
        let mut candidates = Vec::new();

        for ns in &namespaces {
            for tier in &tiers {
                let query = ClaimQuery {
                    namespace: ns.clone(),
                    tier: Some(tier.clone()),
                    min_confidence: None,
                    semantic_text: None,
                    limit: None,
                };
                let claims = store
                    .query_claims(&query)
                    .map_err(|e| SynthesizerError::Store(e.to_string()))?;

                for claim in claims {
                    if let Some(since) = scope.since {
                        if claim.created_at < since {
                            continue;
                        }
                    }
                    if seen.insert(claim.id.value()) {
                        candidates.push(claim);
                    }
                }
            }
        }

        Ok(candidates)
    }

    /// Collect the deduplicated set of relationships among the candidate claims.
    fn gather_relationships<S>(
        &self,
        store: &S,
        candidates: &[Claim],
    ) -> Result<Vec<Relationship>, SynthesizerError>
    where
        S: ClaimStore,
        S::Error: std::fmt::Display,
    {
        let mut seen: HashSet<(u128, u128, u8)> = HashSet::new();
        let mut rels = Vec::new();

        for claim in candidates {
            let claim_rels = store
                .get_relationships(claim.id)
                .map_err(|e| SynthesizerError::Store(e.to_string()))?;
            for rel in claim_rels {
                let key = (
                    rel.from_claim.value(),
                    rel.to_claim.value(),
                    rel_type_ordinal(rel.relationship_type),
                );
                if seen.insert(key) {
                    rels.push(rel);
                }
            }
        }

        Ok(rels)
    }

    /// Compute the maximum `derived_from` depth among a cluster's claims.
    fn cluster_max_depth<S>(&self, store: &S, claims: &[Claim]) -> usize
    where
        S: ClaimStore,
        S::Error: std::fmt::Display,
    {
        let cap = self.config.max_derivation_depth;
        let mut memo: HashMap<u128, usize> = HashMap::new();
        claims
            .iter()
            .map(|c| {
                let mut visiting = HashSet::new();
                derivation_depth(store, c.id, cap, &mut memo, &mut visiting)
            })
            .max()
            .unwrap_or(0)
    }

    /// Ask the LLM whether the cluster implies an insight.
    async fn analyze_cluster(
        &self,
        cluster: &ClaimCluster,
    ) -> Result<Option<InsightCandidate>, SynthesizerError> {
        let namespace = dominant_namespace(&cluster.claims);
        let prompt = PromptBuilder::new(&namespace, &cluster.claims).build();

        let response = timeout(self.config.cluster_timeout(), self.call_llm(prompt))
            .await
            .map_err(|_| SynthesizerError::Timeout)??;

        parse_insight_response(&response)
    }

    /// Validate, persist, and link an insight candidate. Returns `None` when the
    /// candidate is rejected (low confidence or gatekeeper).
    fn materialize_insight<S>(
        &self,
        store: &mut S,
        cluster: &ClaimCluster,
        candidate: InsightCandidate,
    ) -> Result<Option<SynthesizedInsight>, SynthesizerError>
    where
        S: ClaimStore,
        S::Error: std::fmt::Display,
    {
        let confidence = propagate_confidence(&cluster.claims, candidate.llm_confidence);

        // Quality bar: drop weak insights.
        if confidence.1 < self.config.min_insight_confidence {
            debug!(
                "Rejecting insight (upper confidence {:.2} < {:.2})",
                confidence.1, self.config.min_insight_confidence
            );
            return Ok(None);
        }

        let namespace = dominant_namespace(&cluster.claims);
        let tier = weakest_tier(&cluster.claims);
        let now = unix_now();

        let claim = Claim {
            id: ClaimId::new(),
            namespace,
            subject: candidate.subject.clone(),
            predicate: candidate.predicate.clone(),
            object: candidate.object.clone(),
            source_type: Claim::SOURCE_INFERENCE.to_string(),
            confidence,
            tier: tier.as_str().to_string(),
            created_at: now,
            stale_at: None,
        };

        // Gatekeeper validation (with duplicate detection against the store).
        let validation = self
            .gatekeeper
            .validate(&claim, Some(&*store))
            .map_err(|e| SynthesizerError::Validation(e.to_string()))?;

        if validation.status != ValidationStatus::Accepted {
            debug!("Gatekeeper rejected insight: {:?}", validation.reasons);
            return Ok(None);
        }

        let derived_from = cluster.claim_ids();

        if self.config.dry_run {
            info!(
                "[dry-run] would synthesize: {} {} {} from {} claims",
                claim.subject,
                claim.predicate,
                claim.object,
                derived_from.len()
            );
            return Ok(Some(SynthesizedInsight {
                claim_id: claim.id,
                subject: claim.subject,
                predicate: claim.predicate,
                object: claim.object,
                confidence,
                derived_from,
                rationale: candidate.rationale,
            }));
        }

        // Persist the derived claim.
        let claim_id = store
            .assert_claim(claim.clone())
            .map_err(|e| SynthesizerError::Store(e.to_string()))?;

        // Create derived_from relationships to each constituent. The strength
        // reflects the LLM's confidence in the inference.
        let strength = candidate.llm_confidence.1.clamp(0.0, 1.0);
        for &constituent in &derived_from {
            let rel = Relationship::new(
                claim_id,
                constituent,
                RelationshipType::DerivedFrom,
                strength,
                now,
            );
            if let Err(e) = store.add_relationship(rel) {
                warn!("Failed to add derived_from relationship: {}", e);
            }
        }

        info!(
            "Synthesized insight {}: {} {} {} (confidence {:.2}-{:.2}) from {} claims",
            claim_id,
            claim.subject,
            claim.predicate,
            claim.object,
            confidence.0,
            confidence.1,
            derived_from.len(),
        );

        Ok(Some(SynthesizedInsight {
            claim_id,
            subject: claim.subject,
            predicate: claim.predicate,
            object: claim.object,
            confidence,
            derived_from,
            rationale: candidate.rationale,
        }))
    }

    /// Call the (synchronous) LLM provider off the async runtime.
    async fn call_llm(&self, prompt: String) -> Result<String, SynthesizerError> {
        let llm = Arc::clone(&self.llm);
        tokio::task::spawn_blocking(move || {
            llm.generate(&prompt)
                .map_err(|e| SynthesizerError::Llm(e.to_string()))
        })
        .await
        .map_err(|e| SynthesizerError::Llm(format!("Task join error: {}", e)))?
    }
}

/// Compute the derivation depth of a claim by following `derived_from` edges.
/// Bounded by `cap` and cycle-safe.
fn derivation_depth<S>(
    store: &S,
    id: ClaimId,
    cap: usize,
    memo: &mut HashMap<u128, usize>,
    visiting: &mut HashSet<u128>,
) -> usize
where
    S: ClaimStore,
    S::Error: std::fmt::Display,
{
    if let Some(&d) = memo.get(&id.value()) {
        return d;
    }
    if cap == 0 || !visiting.insert(id.value()) {
        // Depth budget exhausted or cycle detected.
        return 0;
    }

    let targets: Vec<ClaimId> = match store.get_relationships(id) {
        Ok(rels) => rels
            .into_iter()
            .filter(|r| r.relationship_type == RelationshipType::DerivedFrom && r.from_claim == id)
            .map(|r| r.to_claim)
            .collect(),
        Err(_) => Vec::new(),
    };

    let depth = if targets.is_empty() {
        0
    } else {
        1 + targets
            .into_iter()
            .map(|t| derivation_depth(store, t, cap - 1, memo, visiting))
            .max()
            .unwrap_or(0)
    };

    visiting.remove(&id.value());
    memo.insert(id.value(), depth);
    depth
}

/// Numeric ordinal for a tier (higher = more permanent).
fn tier_ordinal(tier: Tier) -> u8 {
    match tier {
        Tier::Ephemeral => 0,
        Tier::Task => 1,
        Tier::Project => 2,
        Tier::Permanent => 3,
    }
}

/// The list of tier names at or above `min_tier` (defaults to `task`).
fn tiers_at_or_above(min_tier: &str) -> Vec<String> {
    let start = Tier::parse(min_tier).unwrap_or(Tier::Task);
    let start_ord = tier_ordinal(start);
    [Tier::Ephemeral, Tier::Task, Tier::Project, Tier::Permanent]
        .into_iter()
        .filter(|t| tier_ordinal(*t) >= start_ord)
        .map(|t| t.as_str().to_string())
        .collect()
}

/// The weakest (lowest) tier among a set of claims. A synthesized claim is no
/// more durable than its least-durable constituent.
fn weakest_tier(claims: &[Claim]) -> Tier {
    claims
        .iter()
        .filter_map(|c| Tier::parse(&c.tier))
        .min_by_key(|t| tier_ordinal(*t))
        .unwrap_or(Tier::Task)
}

/// The most common namespace among a set of claims (ties broken by first seen).
fn dominant_namespace(claims: &[Claim]) -> String {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    let mut best: Option<(&str, usize)> = None;
    for claim in claims {
        let entry = counts.entry(claim.namespace.as_str()).or_insert(0);
        *entry += 1;
        let count = *entry;
        match best {
            Some((_, best_count)) if best_count >= count => {}
            _ => best = Some((claim.namespace.as_str(), count)),
        }
    }
    best.map(|(ns, _)| ns.to_string())
        .unwrap_or_else(|| "synthesis".to_string())
}

/// Stable ordinal for a relationship type (used for dedup keys).
fn rel_type_ordinal(t: RelationshipType) -> u8 {
    match t {
        RelationshipType::Supports => 0,
        RelationshipType::Contradicts => 1,
        RelationshipType::DerivedFrom => 2,
        RelationshipType::References => 3,
        RelationshipType::Supersedes => 4,
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0))
        .as_secs()
}

fn elapsed_ms(start: SystemTime) -> u64 {
    start
        .elapsed()
        .unwrap_or(Duration::from_secs(0))
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claim(subject: &str, tier: Tier) -> Claim {
        Claim {
            id: ClaimId::new(),
            namespace: "ns".to_string(),
            subject: subject.to_string(),
            predicate: "p".to_string(),
            object: "o".to_string(),
            source_type: "assertion".to_string(),
            confidence: (0.6, 0.8),
            tier: tier.as_str().to_string(),
            created_at: 0,
            stale_at: None,
        }
    }

    #[test]
    fn test_tiers_at_or_above() {
        assert_eq!(
            tiers_at_or_above("task"),
            vec!["task", "project", "permanent"]
        );
        assert_eq!(tiers_at_or_above("permanent"), vec!["permanent"]);
        assert_eq!(
            tiers_at_or_above("ephemeral"),
            vec!["ephemeral", "task", "project", "permanent"]
        );
        // Unknown tier falls back to task.
        assert_eq!(tiers_at_or_above("bogus"), vec!["task", "project", "permanent"]);
    }

    #[test]
    fn test_weakest_tier() {
        let claims = vec![
            claim("a", Tier::Project),
            claim("b", Tier::Task),
            claim("c", Tier::Permanent),
        ];
        assert_eq!(weakest_tier(&claims), Tier::Task);
    }

    #[test]
    fn test_dominant_namespace() {
        let mut a = claim("a", Tier::Task);
        let mut b = claim("b", Tier::Task);
        let c = claim("c", Tier::Task);
        a.namespace = "eng".to_string();
        b.namespace = "eng".to_string();
        // c stays "ns"
        assert_eq!(dominant_namespace(&[a, b, c]), "eng");
    }
}
