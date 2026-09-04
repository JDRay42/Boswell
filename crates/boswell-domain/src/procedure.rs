//! Procedure module - a stored, reusable "how-to" (procedural memory).
//!
//! A [`Procedure`] is a first-class entity, a **sibling** to [`crate::Claim`] rather
//! than something built out of claims. Where a claim answers *what is true*, a
//! procedure captures *how to do a thing*: a rich, uniform, typed signature (for
//! retrieval, gating, and lifecycle) plus an opaque, format-tagged body.
//!
//! This is Phase 1 of the design in `docs/architecture/15-procedural-memory.md`:
//! the `Procedure` entity, prose bodies only, grouped by a `goal` string handle
//! (the `Goal` entity itself is Phase 2). The truth model for a procedure is
//! **effectiveness** (working / not working), not confidence.

use crate::Tier;
use std::fmt;

/// Unique identifier for a procedure, based on UUIDv7 (per ADR-011).
///
/// Mirrors [`crate::ClaimId`]: chronologically sortable, 128-bit, RFC 9562, no
/// coordination required for distributed generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProcedureId(u128);

impl ProcedureId {
    /// Generate a new UUIDv7-based `ProcedureId`.
    pub fn new() -> Self {
        Self(uuid::Uuid::now_v7().as_u128())
    }

    /// Create a `ProcedureId` from a raw `u128` value (for storage deserialization).
    pub fn from_value(value: u128) -> Self {
        Self(value)
    }

    /// Parse a `ProcedureId` from a UUIDv7 string.
    pub fn from_string(s: &str) -> Result<Self, String> {
        uuid::Uuid::parse_str(s)
            .map(|u| Self(u.as_u128()))
            .map_err(|e| format!("Invalid UUIDv7 string: {}", e))
    }

    /// Get the raw `u128` value.
    pub fn value(&self) -> u128 {
        self.0
    }

    /// Get the timestamp component of the UUIDv7 (milliseconds since Unix epoch).
    pub fn timestamp(&self) -> u64 {
        (self.0 >> 80) as u64
    }
}

impl Default for ProcedureId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ProcedureId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", uuid::Uuid::from_u128(self.0))
    }
}

/// How a procedure came to exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcedureSource {
    /// Hand-authored by a human or agent.
    Authored,
    /// Learned/induced from observed execution episodes.
    Learned,
    /// Bulk-imported from an external source.
    Imported,
}

impl ProcedureSource {
    /// Get the source name as a stable string (for storage).
    pub fn as_str(&self) -> &'static str {
        match self {
            ProcedureSource::Authored => "authored",
            ProcedureSource::Learned => "learned",
            ProcedureSource::Imported => "imported",
        }
    }

    /// Parse a source from its string form.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "authored" => Some(ProcedureSource::Authored),
            "learned" => Some(ProcedureSource::Learned),
            "imported" => Some(ProcedureSource::Imported),
            _ => None,
        }
    }
}

impl std::str::FromStr for ProcedureSource {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s).ok_or_else(|| format!("Invalid procedure source: {}", s))
    }
}

/// The format of a procedure's opaque body.
///
/// The signature (intent, preconditions, etc.) is format-invariant; adding
/// `Dsl`/`Code` later touches only the body and its executor, never the
/// retrieval/gating surface. Phase 1 ships `Prose` only (the LLM is the
/// interpreter).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyFormat {
    /// Free-text prose, interpreted by an LLM.
    Prose,
    /// A domain-specific control-flow language (not yet implemented).
    Dsl,
    /// Executable code (not yet implemented).
    Code,
}

impl BodyFormat {
    /// Get the format name as a stable string (for storage).
    pub fn as_str(&self) -> &'static str {
        match self {
            BodyFormat::Prose => "prose",
            BodyFormat::Dsl => "dsl",
            BodyFormat::Code => "code",
        }
    }

    /// Parse a body format from its string form.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "prose" => Some(BodyFormat::Prose),
            "dsl" => Some(BodyFormat::Dsl),
            "code" => Some(BodyFormat::Code),
            _ => None,
        }
    }
}

impl std::str::FromStr for BodyFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s).ok_or_else(|| format!("Invalid body format: {}", s))
    }
}

/// A typed input parameter of a procedure's signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parameter {
    /// Parameter name.
    pub name: String,
    /// Declared type (free-form for now, e.g. `"string"`, `"int"`, `"ingredient"`).
    pub type_name: String,
    /// Optional default value (as text).
    pub default: Option<String>,
    /// Optional human-readable description.
    pub desc: Option<String>,
}

/// Whether a precondition's claim-query pattern expects a matching claim to
/// exist or to be absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expect {
    /// At least one matching claim (at or above `min_confidence`) must exist.
    Exists,
    /// No matching claim (at or above `min_confidence`) may exist.
    Absent,
}

impl Expect {
    /// Get the expectation name as a stable string (for storage).
    pub fn as_str(&self) -> &'static str {
        match self {
            Expect::Exists => "exists",
            Expect::Absent => "absent",
        }
    }

    /// Parse an expectation from its string form.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "exists" => Some(Expect::Exists),
            "absent" => Some(Expect::Absent),
            _ => None,
        }
    }
}

/// A claim-query pattern: the `(subject, predicate, object)` triple a
/// precondition resolves against Boswell's claim store.
///
/// This is the "dogfood seam" onto claims — a precondition's satisfaction is
/// decided by querying the same claim substrate everything else uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimMatch {
    /// Subject to match exactly.
    pub subject: String,
    /// Predicate to match exactly.
    pub predicate: String,
    /// Object to match exactly.
    pub object: String,
}

/// The structured `check` of a precondition: a claim-query pattern plus the
/// confidence floor and the existence expectation.
#[derive(Debug, Clone, PartialEq)]
pub struct PreconditionCheck {
    /// The `(subject, predicate, object)` pattern to resolve against claims.
    pub match_pattern: ClaimMatch,
    /// Minimum (lower-bound) confidence a matching claim must carry to count.
    pub min_confidence: f64,
    /// Whether a matching claim must exist or must be absent.
    pub expect: Expect,
}

/// A hard precondition that gates whether a procedure may be run at all.
///
/// Preconditions are *node-intrinsic* (they belong to the procedure), as opposed
/// to the edge-contextual selection signals that will live on `Goal` edges in
/// Phase 2.
#[derive(Debug, Clone, PartialEq)]
pub struct Precondition {
    /// Category of the precondition, e.g. `"resource"`, `"state"` (free-form).
    pub kind: String,
    /// Human-readable description, e.g. `"eggs on hand"`.
    pub description: String,
    /// The structured, resolvable check.
    pub check: PreconditionCheck,
}

/// A stored how-to: a reusable, refined procedure with an intact, typed
/// signature and an opaque, format-tagged body.
///
/// See the module docs and `docs/architecture/15-procedural-memory.md` §3.1.
/// Effectiveness is **derived** ([`Procedure::effectiveness`]), never stored.
#[derive(Debug, Clone, PartialEq)]
pub struct Procedure {
    // --- Identity / versioning ---
    /// Unique identifier.
    pub id: ProcedureId,
    /// Namespace for organization (per ADR-006).
    pub namespace: String,
    /// Human-readable name, e.g. `"omelette-classic"`.
    pub name: String,
    /// Monotonic version within a same-technique lineage.
    pub version: u32,
    /// Prior-version link: the procedure this one refines/replaces, if any.
    pub supersedes: Option<ProcedureId>,
    /// Head-of-lineage flag: `true` marks the live version of its lineage.
    pub is_current: bool,
    /// How this procedure came to exist.
    pub source: ProcedureSource,

    // --- Grouping ---
    /// Handle grouping variants and versions that pursue the same outcome.
    ///
    /// In Phase 1 this is an opaque string key (e.g.
    /// `"goal:person:jd/cook-eggs"`); the `Goal` entity arrives in Phase 2.
    pub goal: String,

    // --- Signature (uniform; drives retrieval + gating) ---
    /// Embedded intent text — the semantic match key for retrieval.
    pub intent: String,
    /// Free-form tags.
    pub tags: Vec<String>,
    /// Typed input parameters.
    pub parameters: Vec<Parameter>,
    /// Hard preconditions that gate execution.
    pub preconditions: Vec<Precondition>,
    /// Tools the procedure requires to run.
    pub required_tools: Vec<String>,
    /// Postconditions describing the intended end state (prose in Phase 1).
    pub postconditions: Vec<String>,
    /// Estimated duration in seconds, if known.
    pub est_duration_sec: Option<u64>,

    // --- Selection hints (soft; for ranking among siblings) ---
    /// Prose usage notes.
    pub usage_notes: String,
    /// Soft selection tags, e.g. `"time:quick"`, `"mood:fancy"`.
    pub context_tags: Vec<String>,

    // --- Body (opaque, format-tagged) ---
    /// Format of the body content.
    pub body_format: BodyFormat,
    /// MIME-ish content type, e.g. `"text/plain"`.
    pub content_type: String,
    /// The opaque body itself.
    pub body: String,

    // --- Effectiveness & lifecycle ---
    /// Current tier (reuses the claim lifecycle tiers).
    pub tier: Tier,
    /// Number of times this procedure has been used (a genuine trial of its body).
    pub use_count: u64,
    /// Number of trials reported as successful.
    pub success_count: u64,
    /// Number of trials reported as a failure attributable to the procedure.
    pub failure_count: u64,
    /// Number of handed-out executions whose receipt expired unreported
    /// ("silence is not success", design §3.3). Each counts, mildly, against
    /// effectiveness.
    pub unknown_count: u64,
    /// When this procedure was last used (Unix ms), if ever.
    pub last_used_at: Option<u64>,
    /// When this procedure was created (Unix ms).
    pub created_at: u64,
    /// When this procedure was last updated (Unix ms).
    pub updated_at: u64,
    /// When this procedure should be considered stale (Unix ms), if set.
    pub stale_at: Option<u64>,
}

/// Half-life (in milliseconds) used by the recency term of
/// [`Procedure::effectiveness`]. Thirty days: a procedure unused for a month
/// counts, for ranking, at roughly half the effectiveness of an identical one
/// used today.
pub const EFFECTIVENESS_RECENCY_HALF_LIFE_MS: u64 = 30 * 24 * 60 * 60 * 1000;

impl Procedure {
    /// Derive effectiveness (`success-rate × recency`) as of `now` (Unix ms).
    ///
    /// - **success-rate** is `success / (success + failure + ½·unknown)`, and
    ///   `0.0` when there have been no trials at all (an unproven procedure ranks
    ///   below any proven one). Unknowns (unreported, expired receipts) count as
    ///   half a failure — mildly negative, per "silence is not success".
    /// - **recency** decays exponentially from the last use (or, if never used,
    ///   from creation) with a half-life of
    ///   [`EFFECTIVENESS_RECENCY_HALF_LIFE_MS`], clamped to `[0.0, 1.0]`.
    ///
    /// This is a deterministic ranking signal; the store surfaces and ranks, it
    /// never decides.
    pub fn effectiveness(&self, now: u64) -> f64 {
        // Unknowns weigh half a failure: mildly negative for reliability (§3.3).
        let denom =
            self.success_count as f64 + self.failure_count as f64 + 0.5 * self.unknown_count as f64;
        let success_rate = if denom == 0.0 {
            0.0
        } else {
            self.success_count as f64 / denom
        };

        let reference = self.last_used_at.unwrap_or(self.created_at);
        // Guard against clock skew (a reference in the future -> no decay).
        let age_ms = now.saturating_sub(reference);
        let half_lives = age_ms as f64 / EFFECTIVENESS_RECENCY_HALF_LIFE_MS as f64;
        let recency = 0.5_f64.powf(half_lives);

        (success_rate * recency).clamp(0.0, 1.0)
    }

    /// Apply an [`OutcomeReport`] to this procedure's counters, returning a
    /// [`ReportEffect`] describing exactly what changed.
    ///
    /// Attribution follows `docs/architecture/15-procedural-memory.md` §3.3:
    /// - `Success` counts as a success (and a use).
    /// - `Failure(BadResult | StepFailed)` counts as a failure (and a use).
    /// - `Failure(ExecutorError)` does **not** touch procedure stats — the
    ///   executor, not the procedure, is at fault.
    /// - `Failure(PreconditionsStale)` demotes the *precondition check*, not the
    ///   body: no counter change, but the effect flags it for follow-up.
    /// - `Failure` with no `failure_mode` is treated as a `BadResult`.
    /// - `Abandoned` records a use but is neither success nor failure.
    ///
    /// `now` (Unix ms) updates `last_used_at`/`updated_at` where a use is recorded.
    pub fn apply_report(&mut self, report: &OutcomeReport, now: u64) -> ReportEffect {
        let mut effect = ReportEffect::default();

        match &report.outcome {
            Outcome::Success => {
                self.use_count += 1;
                self.success_count += 1;
                self.last_used_at = Some(now);
                self.updated_at = now;
                effect.counted_as_success = true;
                effect.use_recorded = true;
            }
            Outcome::Abandoned => {
                // Reported abandonment: a use occurred but neither succeeded nor
                // failed. (Unreported/expired receipts are "unknown" and handled
                // by a later sweep, not here.)
                self.use_count += 1;
                self.last_used_at = Some(now);
                self.updated_at = now;
                effect.use_recorded = true;
            }
            Outcome::Failure => match report.failure_mode {
                Some(FailureMode::ExecutorError) => {
                    // Not the procedure's fault: leave its stats untouched.
                    effect.attributed_to_executor = true;
                }
                Some(FailureMode::PreconditionsStale) => {
                    // Demote the precondition check, not the body.
                    effect.flagged_precondition_stale = true;
                }
                Some(FailureMode::BadResult) | Some(FailureMode::StepFailed(_)) | None => {
                    self.use_count += 1;
                    self.failure_count += 1;
                    self.last_used_at = Some(now);
                    self.updated_at = now;
                    effect.counted_as_failure = true;
                    effect.use_recorded = true;
                }
            },
        }

        effect
    }

    /// Record that a handed-out execution's receipt expired unreported (design
    /// §3.3): bumps `unknown_count` and `updated_at`. This is *not* a use — it does
    /// not touch `last_used_at` — so it cannot flatter the recency term.
    pub fn record_unknown(&mut self, now: u64) {
        self.unknown_count += 1;
        self.updated_at = now;
    }
}

/// The lifecycle status of an [`ExecutionReceipt`] (design §3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiptStatus {
    /// Issued and awaiting a report.
    Pending,
    /// A report was received and applied.
    Reported,
    /// The receipt expired unreported (counts as `unknown`).
    Expired,
}

impl ReceiptStatus {
    /// Stable storage string.
    pub fn as_str(&self) -> &'static str {
        match self {
            ReceiptStatus::Pending => "pending",
            ReceiptStatus::Reported => "reported",
            ReceiptStatus::Expired => "expired",
        }
    }

    /// Parse from the storage string.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(ReceiptStatus::Pending),
            "reported" => Some(ReceiptStatus::Reported),
            "expired" => Some(ReceiptStatus::Expired),
            _ => None,
        }
    }
}

/// The outcome of an execution, as reported by the executor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The procedure achieved its intent.
    Success,
    /// The procedure did not achieve its intent (see [`FailureMode`]).
    Failure,
    /// Execution was abandoned before an outcome could be determined.
    Abandoned,
}

/// Why a `Failure` outcome occurred — the attribution the gatekeeper weights by
/// reporter trust (design §3.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailureMode {
    /// The procedure's preconditions no longer held; the check is stale, not the
    /// body.
    PreconditionsStale,
    /// A specific step failed (the step is named for diagnosis).
    StepFailed(String),
    /// The procedure ran but produced a bad result.
    BadResult,
    /// The executor itself erred; the procedure is not to blame.
    ExecutorError,
}

/// An execution receipt: the store's obligation-to-report contract, issued when
/// a procedure is handed out for execution (design §3.3).
///
/// In Phase 1 receipts are modeled and constructable but not yet persisted or
/// enforced; wiring to capture hooks/gateway is a later phase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionReceipt {
    /// Unique receipt identifier.
    pub receipt_id: ProcedureId,
    /// The procedure handed out.
    pub procedure_id: ProcedureId,
    /// The procedure version handed out.
    pub version: u32,
    /// The principal the receipt was issued to.
    pub issued_to: String,
    /// Optional task correlation id.
    pub task_id: Option<String>,
    /// Optional session correlation id.
    pub session_id: Option<String>,
    /// When the receipt was issued (Unix ms).
    pub issued_at: u64,
    /// When the receipt expires; an unreported, expired receipt counts as unknown.
    pub expires_at: u64,
    /// Where the report should be sent (opaque handle), if specified.
    pub report_to: Option<String>,
}

impl ExecutionReceipt {
    /// Issue a receipt for `procedure` to `issued_to`, valid for `ttl_ms` from
    /// `issued_at` (both Unix ms). Generates a fresh `receipt_id`.
    pub fn issue(
        procedure: &Procedure,
        issued_to: impl Into<String>,
        issued_at: u64,
        ttl_ms: u64,
    ) -> Self {
        Self {
            receipt_id: ProcedureId::new(),
            procedure_id: procedure.id,
            version: procedure.version,
            issued_to: issued_to.into(),
            task_id: None,
            session_id: None,
            issued_at,
            expires_at: issued_at.saturating_add(ttl_ms),
            report_to: None,
        }
    }
}

/// An outcome report against a receipt (design §3.3).
#[derive(Debug, Clone, PartialEq)]
pub struct OutcomeReport {
    /// The receipt this report answers.
    pub receipt_id: ProcedureId,
    /// The reported outcome.
    pub outcome: Outcome,
    /// The failure attribution, when `outcome` is `Failure`.
    pub failure_mode: Option<FailureMode>,
    /// The executor's self-assessed confidence, if provided.
    pub executor_confidence: Option<f64>,
    /// The reported cost (units are executor-defined), if provided.
    pub cost: Option<f64>,
    /// Free-form notes.
    pub notes: Option<String>,
}

impl OutcomeReport {
    /// A minimal `Success` report against `receipt_id`.
    pub fn success(receipt_id: ProcedureId) -> Self {
        Self {
            receipt_id,
            outcome: Outcome::Success,
            failure_mode: None,
            executor_confidence: None,
            cost: None,
            notes: None,
        }
    }

    /// Whether this report is a failure attributable to the procedure *body* —
    /// `bad_result`, `step_failed`, or an unspecified failure. This is the
    /// negative signal that moves failure counters and that the gatekept write
    /// path guards at team tier; `executor_error` and `preconditions_stale` are
    /// deliberately not procedure-negative (design §3.3).
    pub fn is_negative(&self) -> bool {
        matches!(self.outcome, Outcome::Failure)
            && matches!(
                self.failure_mode,
                Some(FailureMode::BadResult) | Some(FailureMode::StepFailed(_)) | None
            )
    }

    /// A `Failure` report with the given attribution.
    pub fn failure(receipt_id: ProcedureId, failure_mode: FailureMode) -> Self {
        Self {
            receipt_id,
            outcome: Outcome::Failure,
            failure_mode: Some(failure_mode),
            executor_confidence: None,
            cost: None,
            notes: None,
        }
    }
}

/// Query criteria for retrieving procedures (design §4.1, procedure-sibling slice).
///
/// The store filters by these coarse fields in SQL, then applies precondition
/// filtering and effectiveness ranking among the surviving siblings. By default
/// only current (head-of-lineage) procedures are returned.
#[derive(Debug, Clone, Default)]
pub struct ProcedureQuery {
    /// Filter by namespace prefix.
    pub namespace: Option<String>,
    /// Filter by exact `goal` grouping key.
    pub goal: Option<String>,
    /// Filter by a case-insensitive substring of `intent` (Phase 1 stand-in for
    /// semantic intent match).
    pub intent_contains: Option<String>,
    /// Include superseded (non-current) versions. Defaults to `false`, i.e. only
    /// `is_current` procedures are returned.
    pub include_superseded: bool,
    /// Maximum number of ranked results to return.
    pub limit: Option<usize>,
}

/// What [`Procedure::apply_report`] changed — a precise, testable summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ReportEffect {
    /// A success was counted.
    pub counted_as_success: bool,
    /// A failure was counted against the procedure.
    pub counted_as_failure: bool,
    /// The failure was attributed to the executor; procedure stats untouched.
    pub attributed_to_executor: bool,
    /// The precondition check was flagged stale; the body was not blamed.
    pub flagged_precondition_stale: bool,
    /// A use was recorded (`use_count` incremented, `last_used_at` updated).
    pub use_recorded: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(now: u64) -> Procedure {
        Procedure {
            id: ProcedureId::new(),
            namespace: "person:jd".into(),
            name: "omelette-classic".into(),
            version: 1,
            supersedes: None,
            is_current: true,
            source: ProcedureSource::Authored,
            goal: "goal:person:jd/cook-eggs".into(),
            intent: "cook eggs into a classic omelette".into(),
            tags: vec!["breakfast".into()],
            parameters: vec![],
            preconditions: vec![],
            required_tools: vec![],
            postconditions: vec!["eggs are cooked".into()],
            est_duration_sec: Some(600),
            usage_notes: "French-style, buttery.".into(),
            context_tags: vec!["mood:fancy".into()],
            body_format: BodyFormat::Prose,
            content_type: "text/plain".into(),
            body: "Heat pan...".into(),
            tier: Tier::Project,
            use_count: 0,
            success_count: 0,
            failure_count: 0,
            unknown_count: 0,
            last_used_at: None,
            created_at: now,
            updated_at: now,
            stale_at: None,
        }
    }

    #[test]
    fn effectiveness_zero_without_trials() {
        let now = 1_000_000;
        let p = sample(now);
        assert_eq!(p.effectiveness(now), 0.0);
    }

    #[test]
    fn effectiveness_success_rate_times_recency() {
        let now = 1_000_000;
        let mut p = sample(now);
        p.success_count = 3;
        p.failure_count = 1;
        p.last_used_at = Some(now);
        // recency == 1.0 at zero age, so effectiveness == success rate (0.75).
        assert!((p.effectiveness(now) - 0.75).abs() < 1e-9);
    }

    #[test]
    fn unknown_counts_as_half_a_failure() {
        let now = 1_000_000;
        let mut p = sample(now);
        p.success_count = 2;
        p.last_used_at = Some(now);
        // 2 successes, no failures -> 1.0.
        assert!((p.effectiveness(now) - 1.0).abs() < 1e-9);

        // Two unknowns weigh as one failure: 2 / (2 + 0 + 1) = 0.666...
        p.record_unknown(now);
        p.record_unknown(now);
        assert_eq!(p.unknown_count, 2);
        assert!((p.effectiveness(now) - (2.0 / 3.0)).abs() < 1e-9);
        // record_unknown is not a use: recency reference is untouched.
        assert_eq!(p.last_used_at, Some(now));
    }

    #[test]
    fn effectiveness_decays_with_age() {
        let base = 1_000_000_000;
        let mut p = sample(base);
        p.success_count = 4;
        p.failure_count = 0;
        p.last_used_at = Some(base);
        // One half-life later, recency halves: 1.0 * 0.5 == 0.5.
        let later = base + EFFECTIVENESS_RECENCY_HALF_LIFE_MS;
        assert!((p.effectiveness(later) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn report_success_increments_success_and_use() {
        let now = 5;
        let mut p = sample(0);
        let effect = p.apply_report(&OutcomeReport::success(ProcedureId::new()), now);
        assert_eq!(p.success_count, 1);
        assert_eq!(p.failure_count, 0);
        assert_eq!(p.use_count, 1);
        assert_eq!(p.last_used_at, Some(now));
        assert!(effect.counted_as_success && effect.use_recorded);
    }

    #[test]
    fn report_bad_result_counts_as_failure() {
        let mut p = sample(0);
        let effect = p.apply_report(
            &OutcomeReport::failure(ProcedureId::new(), FailureMode::BadResult),
            9,
        );
        assert_eq!(p.failure_count, 1);
        assert_eq!(p.use_count, 1);
        assert!(effect.counted_as_failure);
    }

    #[test]
    fn report_step_failed_counts_as_failure() {
        let mut p = sample(0);
        p.apply_report(
            &OutcomeReport::failure(ProcedureId::new(), FailureMode::StepFailed("flip".into())),
            9,
        );
        assert_eq!(p.failure_count, 1);
        assert_eq!(p.use_count, 1);
    }

    #[test]
    fn report_executor_error_does_not_touch_stats() {
        let mut p = sample(0);
        let effect = p.apply_report(
            &OutcomeReport::failure(ProcedureId::new(), FailureMode::ExecutorError),
            9,
        );
        assert_eq!(p.success_count, 0);
        assert_eq!(p.failure_count, 0);
        assert_eq!(p.use_count, 0);
        assert_eq!(p.last_used_at, None);
        assert!(effect.attributed_to_executor && !effect.use_recorded);
    }

    #[test]
    fn report_preconditions_stale_flags_precondition_only() {
        let mut p = sample(0);
        let effect = p.apply_report(
            &OutcomeReport::failure(ProcedureId::new(), FailureMode::PreconditionsStale),
            9,
        );
        assert_eq!(p.success_count, 0);
        assert_eq!(p.failure_count, 0);
        assert_eq!(p.use_count, 0);
        assert!(effect.flagged_precondition_stale && !effect.counted_as_failure);
    }

    #[test]
    fn report_abandoned_records_use_only() {
        let mut p = sample(0);
        let report = OutcomeReport {
            receipt_id: ProcedureId::new(),
            outcome: Outcome::Abandoned,
            failure_mode: None,
            executor_confidence: None,
            cost: None,
            notes: None,
        };
        let effect = p.apply_report(&report, 7);
        assert_eq!(p.use_count, 1);
        assert_eq!(p.success_count, 0);
        assert_eq!(p.failure_count, 0);
        assert!(effect.use_recorded && !effect.counted_as_success && !effect.counted_as_failure);
    }

    #[test]
    fn source_and_format_roundtrip() {
        for s in [
            ProcedureSource::Authored,
            ProcedureSource::Learned,
            ProcedureSource::Imported,
        ] {
            assert_eq!(ProcedureSource::parse(s.as_str()), Some(s));
        }
        for f in [BodyFormat::Prose, BodyFormat::Dsl, BodyFormat::Code] {
            assert_eq!(BodyFormat::parse(f.as_str()), Some(f));
        }
        for e in [Expect::Exists, Expect::Absent] {
            assert_eq!(Expect::parse(e.as_str()), Some(e));
        }
        assert!(ProcedureSource::parse("bogus").is_none());
    }

    #[test]
    fn receipt_issue_sets_fields() {
        let p = sample(0);
        let r = ExecutionReceipt::issue(&p, "agent:worker", 100, 50);
        assert_eq!(r.procedure_id, p.id);
        assert_eq!(r.version, p.version);
        assert_eq!(r.issued_to, "agent:worker");
        assert_eq!(r.issued_at, 100);
        assert_eq!(r.expires_at, 150);
    }
}
