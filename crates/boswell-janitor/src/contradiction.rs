//! LLM-backed contradiction detection — the "Contradiction Janitor".
//!
//! Unlike the tier [`Janitor`](crate::Janitor), which is synchronous and
//! deterministic, this janitor uses an LLM to decide whether two claims about
//! the same subject contradict each other. When they do, it records a
//! `Contradicts` relationship between them — which the confidence computation
//! (ADR-007) then folds in as a penalty, lowering the effective confidence of
//! both claims.
//!
//! To bound LLM cost it only compares claims that share a subject but differ in
//! object, skips pairs that already have a relationship, and caps the number of
//! pairs evaluated per pass.

use crate::JanitorError;
use boswell_domain::traits::{ClaimQuery, ClaimStore, LlmProvider};
use boswell_domain::{Claim, ClaimId, Relationship, RelationshipType, Tier};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::timeout;
use tracing::{debug, info, warn};

/// Configuration for contradiction detection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ContradictionConfig {
    /// Whether a pass does any work. When `false`, a pass is a no-op.
    pub enabled: bool,
    /// How often the background worker runs a scan, in hours.
    pub scan_interval_hours: u64,
    /// Minimum tier to consider (claims below this are skipped as noise).
    pub min_tier: String,
    /// Maximum number of claim pairs evaluated per pass (rate limiting).
    pub max_pairs_per_pass: usize,
    /// Minimum LLM confidence required to record a contradiction.
    pub min_confidence: f64,
    /// When `true`, detect and log contradictions without recording them.
    pub dry_run: bool,
    /// Per-pair LLM call timeout, in seconds.
    pub llm_timeout_secs: u64,
}

impl Default for ContradictionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            scan_interval_hours: 12,
            min_tier: "task".to_string(),
            max_pairs_per_pass: 50,
            min_confidence: 0.6,
            dry_run: false,
            llm_timeout_secs: 60,
        }
    }
}

impl ContradictionConfig {
    /// The scan interval as a [`Duration`].
    pub fn scan_interval(&self) -> Duration {
        Duration::from_secs(self.scan_interval_hours.max(1) * 3600)
    }

    fn llm_timeout(&self) -> Duration {
        Duration::from_secs(self.llm_timeout_secs)
    }
}

/// A contradiction the janitor detected between two claims.
#[derive(Debug, Clone)]
pub struct DetectedContradiction {
    /// One claim in the contradicting pair.
    pub from: ClaimId,
    /// The other claim.
    pub to: ClaimId,
    /// LLM confidence in the contradiction, in `[0, 1]`.
    pub confidence: f64,
    /// The LLM's rationale.
    pub rationale: String,
}

/// Summary of a single contradiction-detection pass.
#[derive(Debug, Default, Clone)]
pub struct ContradictionReport {
    /// Number of candidate claims examined.
    pub claims_examined: usize,
    /// Number of claim pairs sent to the LLM.
    pub pairs_evaluated: usize,
    /// Number of contradictions recorded (or, in dry-run, that would be).
    pub contradictions_found: usize,
    /// Wall-clock duration of the pass, in milliseconds.
    pub duration_ms: u64,
    /// The contradictions found this pass.
    pub contradictions: Vec<DetectedContradiction>,
}

impl ContradictionReport {
    /// A one-line human-readable summary.
    pub fn summary(&self) -> String {
        format!(
            "Contradiction scan: {} claims examined, {} pairs evaluated, {} contradictions found ({} ms)",
            self.claims_examined, self.pairs_evaluated, self.contradictions_found, self.duration_ms
        )
    }
}

/// The Contradiction Janitor detects and records contradictory claim pairs.
pub struct ContradictionJanitor<L>
where
    L: LlmProvider,
{
    llm: Arc<L>,
    config: ContradictionConfig,
}

impl<L> ContradictionJanitor<L>
where
    L: LlmProvider + Send + Sync + 'static,
    L::Error: std::fmt::Display,
{
    /// Create a new Contradiction Janitor.
    pub fn new(llm: L, config: ContradictionConfig) -> Self {
        Self {
            llm: Arc::new(llm),
            config,
        }
    }

    /// Access the current configuration.
    pub fn config(&self) -> &ContradictionConfig {
        &self.config
    }

    /// Run a single contradiction scan over an owned/exclusive store.
    pub async fn scan_pass<S>(&self, store: &mut S) -> Result<ContradictionReport, JanitorError>
    where
        S: ClaimStore,
        S::Error: std::fmt::Display,
    {
        let start = SystemTime::now();
        let mut report = ContradictionReport::default();
        if !self.config.enabled {
            return Ok(report);
        }

        let (examined, pairs) = self.plan(store)?;
        report.claims_examined = examined;

        for (a, b) in pairs {
            report.pairs_evaluated += 1;
            if let Some(detected) = self.evaluate_pair(&a, &b).await? {
                let result = self.record(store, detected);
                Self::record_detection(&mut report, result);
            }
        }

        report.duration_ms = elapsed_ms(start);
        info!("{}", report.summary());
        Ok(report)
    }

    /// Run a single scan against a store shared with other services
    /// (e.g. the gRPC server). The store lock is held only for the synchronous
    /// planning and recording phases — never across the LLM call.
    pub async fn scan_pass_shared<S>(
        &self,
        store: Arc<Mutex<S>>,
    ) -> Result<ContradictionReport, JanitorError>
    where
        S: ClaimStore + Send + 'static,
        S::Error: std::fmt::Display,
    {
        let start = SystemTime::now();
        let mut report = ContradictionReport::default();
        if !self.config.enabled {
            return Ok(report);
        }

        let (examined, pairs) = {
            let guard = store.lock().unwrap();
            self.plan(&*guard)?
        };
        report.claims_examined = examined;

        for (a, b) in pairs {
            report.pairs_evaluated += 1;
            if let Some(detected) = self.evaluate_pair(&a, &b).await? {
                let result = {
                    let mut guard = store.lock().unwrap();
                    self.record(&mut *guard, detected)
                };
                Self::record_detection(&mut report, result);
            }
        }

        report.duration_ms = elapsed_ms(start);
        info!("{}", report.summary());
        Ok(report)
    }

    /// Plan the candidate pairs for a pass: gather claims at or above the minimum
    /// tier, and form same-subject / different-object pairs that do not already
    /// have a relationship, capped at `max_pairs_per_pass`. Read-only.
    fn plan<S>(&self, store: &S) -> Result<(usize, Vec<(Claim, Claim)>), JanitorError>
    where
        S: ClaimStore,
        S::Error: std::fmt::Display,
    {
        let min_ord = tier_ordinal(Tier::parse(&self.config.min_tier).unwrap_or(Tier::Task));

        let claims: Vec<Claim> = store
            .query_claims(&ClaimQuery::default())
            .map_err(|e| JanitorError::Store(e.to_string()))?
            .into_iter()
            .filter(|c| {
                Tier::parse(&c.tier)
                    .map(|t| tier_ordinal(t) >= min_ord)
                    .unwrap_or(false)
            })
            .collect();

        let examined = claims.len();

        // Pairs that already have a relationship (either direction) are skipped.
        let mut related: HashSet<(u128, u128)> = HashSet::new();
        for claim in &claims {
            let rels = store
                .get_relationships(claim.id)
                .map_err(|e| JanitorError::Store(e.to_string()))?;
            for rel in rels {
                related.insert(unordered(rel.from_claim.value(), rel.to_claim.value()));
            }
        }

        // Group by subject, then form candidate pairs.
        let mut by_subject: std::collections::HashMap<&str, Vec<&Claim>> =
            std::collections::HashMap::new();
        for claim in &claims {
            by_subject.entry(claim.subject.as_str()).or_default().push(claim);
        }

        let mut pairs = Vec::new();
        'outer: for group in by_subject.values() {
            for i in 0..group.len() {
                for j in (i + 1)..group.len() {
                    let (a, b) = (group[i], group[j]);
                    // Same object → not a contradiction candidate (likely a
                    // duplicate or corroboration).
                    if a.object == b.object {
                        continue;
                    }
                    if related.contains(&unordered(a.id.value(), b.id.value())) {
                        continue;
                    }
                    pairs.push((a.clone(), b.clone()));
                    if pairs.len() >= self.config.max_pairs_per_pass {
                        break 'outer;
                    }
                }
            }
        }

        debug!("Contradiction candidates: {} claims, {} pairs", examined, pairs.len());
        Ok((examined, pairs))
    }

    /// Ask the LLM whether a pair contradicts; returns `Some` only when it does
    /// and the LLM's confidence meets the configured minimum.
    async fn evaluate_pair(
        &self,
        a: &Claim,
        b: &Claim,
    ) -> Result<Option<DetectedContradiction>, JanitorError> {
        let prompt = build_prompt(a, b);
        let response = timeout(self.config.llm_timeout(), self.call_llm(prompt))
            .await
            .map_err(|_| JanitorError::Timeout)??;

        debug!("Contradiction LLM response: {}", response);

        let Some((contradicts, confidence, rationale)) = parse_response(&response) else {
            return Ok(None);
        };

        if !contradicts || confidence < self.config.min_confidence {
            return Ok(None);
        }

        Ok(Some(DetectedContradiction {
            from: a.id,
            to: b.id,
            confidence,
            rationale,
        }))
    }

    /// Record a detected contradiction as a `Contradicts` relationship.
    fn record<S>(
        &self,
        store: &mut S,
        detected: DetectedContradiction,
    ) -> Result<DetectedContradiction, JanitorError>
    where
        S: ClaimStore,
        S::Error: std::fmt::Display,
    {
        if self.config.dry_run {
            info!(
                "[dry-run] would record contradiction {} <-> {} (confidence {:.2})",
                detected.from, detected.to, detected.confidence
            );
            return Ok(detected);
        }

        let rel = Relationship::new(
            detected.from,
            detected.to,
            RelationshipType::Contradicts,
            detected.confidence.clamp(0.0, 1.0),
            unix_now(),
        );
        store
            .add_relationship(rel)
            .map_err(|e| JanitorError::Store(e.to_string()))?;

        info!(
            "Recorded contradiction {} <-> {} (confidence {:.2}): {}",
            detected.from, detected.to, detected.confidence, detected.rationale
        );
        Ok(detected)
    }

    /// Fold a record result into the report.
    fn record_detection(
        report: &mut ContradictionReport,
        result: Result<DetectedContradiction, JanitorError>,
    ) {
        match result {
            Ok(detected) => {
                report.contradictions_found += 1;
                report.contradictions.push(detected);
            }
            Err(e) => warn!("Failed to record contradiction: {}", e),
        }
    }

    /// Call the (synchronous) LLM provider off the async runtime.
    async fn call_llm(&self, prompt: String) -> Result<String, JanitorError> {
        let llm = Arc::clone(&self.llm);
        tokio::task::spawn_blocking(move || {
            llm.generate(&prompt).map_err(|e| JanitorError::Llm(e.to_string()))
        })
        .await
        .map_err(|e| JanitorError::Llm(format!("Task join error: {}", e)))?
    }
}

/// Build the contradiction-detection prompt for a pair of claims.
fn build_prompt(a: &Claim, b: &Claim) -> String {
    format!(
        "You are checking whether two knowledge claims CONTRADICT each other — \
that is, whether they cannot both be true at the same time.\n\n\
Claim A: {} — {} — {}\n\
Claim B: {} — {} — {}\n\n\
Two claims contradict if they assert incompatible things about the same subject \
(e.g. different exclusive values for the same property). They do NOT contradict \
if they are merely different, unrelated, or complementary.\n\n\
Respond with a single JSON object and nothing else:\n\
{{\"contradicts\": true or false, \"confidence\": 0.0 to 1.0, \"rationale\": \"...\"}}",
        a.subject, a.predicate, a.object, b.subject, b.predicate, b.object
    )
}

/// Parse the LLM response into `(contradicts, confidence, rationale)`.
fn parse_response(response: &str) -> Option<(bool, f64, String)> {
    let json_str = extract_json(response);
    let value: Value = serde_json::from_str(&json_str).ok()?;
    let obj = value.as_object()?;

    let contradicts = obj.get("contradicts").and_then(|v| v.as_bool())?;
    let confidence = obj
        .get("confidence")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.5)
        .clamp(0.0, 1.0);
    let rationale = obj
        .get("rationale")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    Some((contradicts, confidence, rationale))
}

/// Extract a JSON object from a response that may be wrapped in markdown fences.
fn extract_json(response: &str) -> String {
    let trimmed = response.trim();
    if let Some(start) = trimmed.find('{') {
        if let Some(end) = trimmed.rfind('}') {
            if end > start {
                return trimmed[start..=end].to_string();
            }
        }
    }
    trimmed.to_string()
}

/// Normalize a pair of claim id values into an order-independent key.
fn unordered(a: u128, b: u128) -> (u128, u128) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

fn tier_ordinal(tier: Tier) -> u8 {
    match tier {
        Tier::Ephemeral => 0,
        Tier::Task => 1,
        Tier::Project => 2,
        Tier::Permanent => 3,
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0))
        .as_secs()
}

fn elapsed_ms(start: SystemTime) -> u64 {
    start.elapsed().unwrap_or(Duration::from_secs(0)).as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use boswell_llm::MockProvider;
    use boswell_store::SqliteStore;

    fn insert(store: &mut SqliteStore, subject: &str, object: &str) -> ClaimId {
        let claim = Claim {
            id: ClaimId::new(),
            namespace: "test".to_string(),
            subject: subject.to_string(),
            predicate: "status:is".to_string(),
            object: object.to_string(),
            source_type: "assertion".to_string(),
            confidence: (0.8, 0.9),
            tier: "task".to_string(),
            created_at: 1_000,
            stale_at: None,
        };
        store.assert_claim(claim).unwrap()
    }

    const YES: &str = r#"{"contradicts": true, "confidence": 0.9, "rationale": "incompatible values"}"#;
    const NO: &str = r#"{"contradicts": false, "confidence": 0.8, "rationale": "unrelated"}"#;

    #[test]
    fn test_parse_response() {
        assert_eq!(parse_response(YES), Some((true, 0.9, "incompatible values".to_string())));
        let (c, _, _) = parse_response(NO).unwrap();
        assert!(!c);
        // Markdown-fenced JSON is tolerated.
        let fenced = "```json\n{\"contradicts\": true, \"confidence\": 0.7}\n```";
        assert_eq!(parse_response(fenced).map(|(c, _, _)| c), Some(true));
        assert!(parse_response("not json").is_none());
    }

    #[test]
    fn test_extract_json() {
        assert_eq!(extract_json("prefix {\"a\":1} suffix"), "{\"a\":1}");
    }

    #[tokio::test]
    async fn test_records_contradiction_for_same_subject_diff_object() {
        let mut store = SqliteStore::new(":memory:", false, 0).unwrap();
        let a = insert(&mut store, "sky:today", "color:blue");
        let b = insert(&mut store, "sky:today", "color:green");

        let janitor = ContradictionJanitor::new(MockProvider::new(YES), ContradictionConfig::default());
        let report = janitor.scan_pass(&mut store).await.unwrap();

        assert_eq!(report.pairs_evaluated, 1);
        assert_eq!(report.contradictions_found, 1);

        // A Contradicts relationship now exists between the two claims.
        let rels = store.get_relationships(a).unwrap();
        assert!(rels
            .iter()
            .any(|r| r.relationship_type == RelationshipType::Contradicts
                && (r.to_claim == b || r.from_claim == b)));
    }

    #[tokio::test]
    async fn test_no_contradiction_when_llm_says_no() {
        let mut store = SqliteStore::new(":memory:", false, 0).unwrap();
        insert(&mut store, "sky:today", "color:blue");
        insert(&mut store, "sky:today", "color:green");

        let janitor = ContradictionJanitor::new(MockProvider::new(NO), ContradictionConfig::default());
        let report = janitor.scan_pass(&mut store).await.unwrap();

        assert_eq!(report.pairs_evaluated, 1);
        assert_eq!(report.contradictions_found, 0);
    }

    #[tokio::test]
    async fn test_same_object_pairs_are_not_evaluated() {
        let mut store = SqliteStore::new(":memory:", false, 0).unwrap();
        // Same subject AND same object → corroboration, not a contradiction pair.
        insert(&mut store, "sky:today", "color:blue");
        insert(&mut store, "sky:today", "color:blue");

        let janitor = ContradictionJanitor::new(MockProvider::new(YES), ContradictionConfig::default());
        let report = janitor.scan_pass(&mut store).await.unwrap();

        assert_eq!(report.pairs_evaluated, 0);
        assert_eq!(report.contradictions_found, 0);
    }

    #[tokio::test]
    async fn test_dry_run_does_not_record() {
        let mut store = SqliteStore::new(":memory:", false, 0).unwrap();
        let a = insert(&mut store, "sky:today", "color:blue");
        insert(&mut store, "sky:today", "color:green");

        let config = ContradictionConfig {
            dry_run: true,
            ..Default::default()
        };
        let janitor = ContradictionJanitor::new(MockProvider::new(YES), config);
        let report = janitor.scan_pass(&mut store).await.unwrap();

        assert_eq!(report.contradictions_found, 1); // detected...
        assert!(store.get_relationships(a).unwrap().is_empty()); // ...but not recorded
    }

    #[tokio::test]
    async fn test_existing_relationship_pair_is_skipped() {
        let mut store = SqliteStore::new(":memory:", false, 0).unwrap();
        let a = insert(&mut store, "sky:today", "color:blue");
        let b = insert(&mut store, "sky:today", "color:green");
        // Pre-existing relationship → the pair should not be re-evaluated.
        store
            .add_relationship(Relationship::new(a, b, RelationshipType::Contradicts, 0.9, 1))
            .unwrap();

        let janitor = ContradictionJanitor::new(MockProvider::new(YES), ContradictionConfig::default());
        let report = janitor.scan_pass(&mut store).await.unwrap();

        assert_eq!(report.pairs_evaluated, 0);
    }
}

#[cfg(test)]
mod real_llm_tests {
    use super::*;
    use boswell_llm::OllamaProvider;
    use boswell_store::SqliteStore;

    fn insert(store: &mut SqliteStore, subject: &str, predicate: &str, object: &str) -> ClaimId {
        let claim = Claim {
            id: ClaimId::new(),
            namespace: "weather".to_string(),
            subject: subject.to_string(),
            predicate: predicate.to_string(),
            object: object.to_string(),
            source_type: "assertion".to_string(),
            confidence: (0.8, 0.9),
            tier: "task".to_string(),
            created_at: 1_000,
            stale_at: None,
        };
        store.assert_claim(claim).unwrap()
    }

    /// Real contradiction detection against a live Ollama chat model.
    ///
    ///   cargo test -p boswell-janitor real_llm_contradiction -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "requires a local Ollama with qwen2.5:7b"]
    async fn test_real_llm_contradiction() {
        let mut store = SqliteStore::new(":memory:", false, 0).unwrap();
        // Two mutually exclusive claims about the same subject/property.
        let a = insert(&mut store, "location:paris", "capital_of:is", "country:france");
        let b = insert(&mut store, "location:paris", "capital_of:is", "country:germany");
        // A clearly non-contradictory, unrelated pair sharing the subject.
        insert(&mut store, "location:paris", "population:approx", "count:2100000");

        let llm = OllamaProvider::new("http://localhost:11434", "qwen2.5:7b");
        let janitor = ContradictionJanitor::new(llm, ContradictionConfig::default());
        let report = janitor.scan_pass(&mut store).await.unwrap();

        println!("\n{}", report.summary());
        for c in &report.contradictions {
            println!("  contradiction: {} <-> {} ({:.2})\n    {}", c.from, c.to, c.confidence, c.rationale);
        }

        // The france/germany pair must be detected as a contradiction and recorded.
        let rels = store.get_relationships(a).unwrap();
        assert!(
            rels.iter().any(|r| r.relationship_type == RelationshipType::Contradicts
                && (r.to_claim == b || r.from_claim == b)),
            "expected a recorded contradiction between the france and germany claims"
        );
    }
}
