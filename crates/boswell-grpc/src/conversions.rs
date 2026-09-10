//! Type conversions between proto and domain types
//!
//! Handles bidirectional conversion between gRPC protobuf types and internal domain types.

use crate::proto;
use boswell_domain::{
    BodyFormat as DomainBodyFormat, ChildKind as DomainChildKind, ChildRef as DomainChildRef,
    Claim, ClaimId, ClaimMatch as DomainClaimMatch, ConfidenceInterval as DomainConfidence,
    EdgeRole as DomainEdgeRole, ExecutionReceipt as DomainReceipt,
    ExpandedCandidate as DomainExpandedCandidate, Expect as DomainExpect,
    FactorReading as DomainFactorReading, FailureMode as DomainFailureMode, Goal as DomainGoal,
    GoalId, Outcome as DomainOutcome, OutcomeReport, Parameter as DomainParameter,
    Precondition as DomainPrecondition, PreconditionCheck as DomainPreconditionCheck,
    Procedure as DomainProcedure, ProcedureId, ProcedureSource as DomainProcedureSource,
    Relationship as DomainRelationship, RelationshipType as DomainRelationshipType,
    Tier as DomainTier, TraversalContext as DomainTraversalContext,
};

/// Error type for conversion failures
#[derive(Debug, thiserror::Error)]
pub enum ConversionError {
    /// Invalid ULID string
    #[error("Invalid claim ID: {0}")]
    InvalidClaimId(String),

    /// Invalid confidence interval
    #[error("Invalid confidence interval: {0}")]
    InvalidConfidence(String),

    /// Invalid tier value
    #[error("Invalid tier value: {0}")]
    InvalidTier(i32),

    /// Invalid relationship-type value
    #[error("Invalid relationship type value: {0}")]
    InvalidRelationshipType(i32),

    /// Missing required field
    #[error("Missing required field: {0}")]
    MissingField(&'static str),

    /// Invalid procedure/receipt id
    #[error("Invalid procedure id: {0}")]
    InvalidProcedureId(String),

    /// Unrecognised execution outcome
    #[error("Invalid outcome (expected success|failure|abandoned): {0}")]
    InvalidOutcome(String),

    /// Unrecognised or misplaced failure attribution
    #[error("Invalid failure mode: {0}")]
    InvalidFailureMode(String),

    /// A procedure field carried a value outside its stable string set
    #[error("Invalid procedure {0}: {1}")]
    InvalidProcedureField(&'static str, String),

    /// Invalid goal id
    #[error("Invalid goal id: {0}")]
    InvalidGoalId(String),

    /// A goal/edge field carried a value outside its stable string set
    #[error("Invalid goal {0}: {1}")]
    InvalidGoalField(&'static str, String),
}

/// Convert proto Tier to tier string
pub fn tier_from_proto(tier: proto::Tier) -> Result<String, ConversionError> {
    let tier_enum = match tier {
        proto::Tier::Unspecified => return Err(ConversionError::InvalidTier(0)),
        proto::Tier::Ephemeral => DomainTier::Ephemeral,
        proto::Tier::Task => DomainTier::Task,
        proto::Tier::Project => DomainTier::Project,
        proto::Tier::Permanent => DomainTier::Permanent,
    };
    Ok(tier_enum.as_str().to_string())
}

/// Convert tier string to proto Tier
pub fn tier_to_proto(tier: &str) -> proto::Tier {
    match DomainTier::parse(tier) {
        Some(DomainTier::Ephemeral) => proto::Tier::Ephemeral,
        Some(DomainTier::Task) => proto::Tier::Task,
        Some(DomainTier::Project) => proto::Tier::Project,
        Some(DomainTier::Permanent) => proto::Tier::Permanent,
        None => proto::Tier::Unspecified,
    }
}

/// Convert per-tier counters into their wire form.
///
/// A tier the Janitor never touched is simply absent; the renderer downstream
/// zero-fills so a series does not appear and disappear between scrapes.
pub fn tier_counts_to_proto(counts: &[(DomainTier, u64)]) -> Vec<proto::TierCount> {
    counts
        .iter()
        .map(|(tier, count)| proto::TierCount {
            tier: tier_to_proto(tier.as_str()) as i32,
            count: *count,
        })
        .collect()
}

/// Convert proto ConfidenceInterval to domain ConfidenceInterval
pub fn confidence_from_proto(
    conf: Option<proto::ConfidenceInterval>,
) -> Result<DomainConfidence, ConversionError> {
    let conf = conf.ok_or(ConversionError::MissingField("confidence"))?;

    if !(0.0..=1.0).contains(&conf.lower) || !(0.0..=1.0).contains(&conf.upper) {
        return Err(ConversionError::InvalidConfidence(
            "bounds must be in [0, 1]".to_string(),
        ));
    }

    if conf.lower > conf.upper {
        return Err(ConversionError::InvalidConfidence(
            "lower must be <= upper".to_string(),
        ));
    }

    Ok(DomainConfidence::new(conf.lower, conf.upper))
}

/// Convert domain ConfidenceInterval to proto ConfidenceInterval
pub fn confidence_to_proto(conf: DomainConfidence) -> proto::ConfidenceInterval {
    proto::ConfidenceInterval {
        lower: conf.lower,
        upper: conf.upper,
    }
}

/// Convert proto Claim to domain Claim
pub fn claim_from_proto(claim: proto::Claim) -> Result<Claim, ConversionError> {
    let id = ClaimId::from_string(&claim.id).map_err(ConversionError::InvalidClaimId)?;

    let confidence = confidence_from_proto(claim.confidence)?;
    let tier = tier_from_proto(
        proto::Tier::try_from(claim.tier).map_err(|_| ConversionError::InvalidTier(claim.tier))?,
    )?;

    let created_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    // Default to a direct assertion when the client omits the origin.
    let source_type = if claim.source_type.is_empty() {
        Claim::SOURCE_ASSERTION.to_string()
    } else {
        claim.source_type
    };

    Ok(Claim {
        id,
        namespace: claim.namespace,
        subject: claim.subject,
        predicate: claim.predicate,
        object: claim.object,
        source_type,
        confidence: (confidence.lower, confidence.upper),
        tier,
        created_at,
        stale_at: None,
    })
}

/// Convert domain Claim to proto Claim
pub fn claim_to_proto(claim: Claim) -> proto::Claim {
    proto::Claim {
        id: claim.id.to_string(),
        namespace: claim.namespace,
        subject: claim.subject,
        predicate: claim.predicate,
        object: claim.object,
        confidence: Some(proto::ConfidenceInterval {
            lower: claim.confidence.0,
            upper: claim.confidence.1,
        }),
        tier: tier_to_proto(&claim.tier) as i32,
        source_type: claim.source_type,
    }
}

/// Convert a domain [`RelationshipType`](DomainRelationshipType) to its proto enum discriminant.
pub fn relationship_type_to_proto(rt: DomainRelationshipType) -> proto::RelationshipType {
    match rt {
        DomainRelationshipType::Supports => proto::RelationshipType::Supports,
        DomainRelationshipType::Contradicts => proto::RelationshipType::Contradicts,
        DomainRelationshipType::DerivedFrom => proto::RelationshipType::DerivedFrom,
        DomainRelationshipType::References => proto::RelationshipType::References,
        DomainRelationshipType::Supersedes => proto::RelationshipType::Supersedes,
    }
}

/// Convert a proto relationship-type discriminant back to the domain enum.
///
/// Returns [`ConversionError::InvalidRelationshipType`] for the unspecified/zero
/// value or any value outside the known set.
pub fn relationship_type_from_proto(value: i32) -> Result<DomainRelationshipType, ConversionError> {
    match proto::RelationshipType::try_from(value) {
        Ok(proto::RelationshipType::Supports) => Ok(DomainRelationshipType::Supports),
        Ok(proto::RelationshipType::Contradicts) => Ok(DomainRelationshipType::Contradicts),
        Ok(proto::RelationshipType::DerivedFrom) => Ok(DomainRelationshipType::DerivedFrom),
        Ok(proto::RelationshipType::References) => Ok(DomainRelationshipType::References),
        Ok(proto::RelationshipType::Supersedes) => Ok(DomainRelationshipType::Supersedes),
        Ok(proto::RelationshipType::Unspecified) | Err(_) => {
            Err(ConversionError::InvalidRelationshipType(value))
        }
    }
}

/// Convert a domain [`Relationship`](DomainRelationship) to its proto message.
pub fn relationship_to_proto(rel: DomainRelationship) -> proto::Relationship {
    proto::Relationship {
        from_claim: rel.from_claim.to_string(),
        to_claim: rel.to_claim.to_string(),
        relationship_type: relationship_type_to_proto(rel.relationship_type) as i32,
        strength: rel.strength,
        created_at: rel.created_at as i64,
    }
}

/// Convert a proto [`Relationship`](proto::Relationship) to the domain type.
pub fn relationship_from_proto(
    rel: proto::Relationship,
) -> Result<DomainRelationship, ConversionError> {
    Ok(DomainRelationship {
        from_claim: ClaimId::from_string(&rel.from_claim)
            .map_err(ConversionError::InvalidClaimId)?,
        to_claim: ClaimId::from_string(&rel.to_claim).map_err(ConversionError::InvalidClaimId)?,
        relationship_type: relationship_type_from_proto(rel.relationship_type)?,
        strength: rel.strength,
        created_at: rel.created_at.max(0) as u64,
    })
}

// ========== Procedural memory (design 15) ==========

/// Convert a domain [`DomainProcedure`] to its proto representation.
pub fn procedure_to_proto(p: &DomainProcedure) -> proto::Procedure {
    proto::Procedure {
        id: p.id.to_string(),
        namespace: p.namespace.clone(),
        name: p.name.clone(),
        version: p.version,
        supersedes: p.supersedes.map(|s| s.to_string()),
        is_current: p.is_current,
        source: p.source.as_str().to_string(),
        goal: p.goal.clone(),
        intent: p.intent.clone(),
        tags: p.tags.clone(),
        parameters: p.parameters.iter().map(parameter_to_proto).collect(),
        preconditions: p.preconditions.iter().map(precondition_to_proto).collect(),
        required_tools: p.required_tools.clone(),
        postconditions: p.postconditions.clone(),
        usage_notes: p.usage_notes.clone(),
        context_tags: p.context_tags.clone(),
        body_format: p.body_format.as_str().to_string(),
        content_type: p.content_type.clone(),
        body: p.body.clone(),
        tier: p.tier.as_str().to_string(),
        use_count: p.use_count,
        success_count: p.success_count,
        failure_count: p.failure_count,
        unknown_count: p.unknown_count,
        est_duration_sec: p.est_duration_sec,
        last_used_at: p.last_used_at,
        created_at: p.created_at,
        updated_at: p.updated_at,
        stale_at: p.stale_at,
    }
}

/// Convert a proto procedure back to its domain form.
pub fn procedure_from_proto(p: &proto::Procedure) -> Result<DomainProcedure, ConversionError> {
    Ok(DomainProcedure {
        id: ProcedureId::from_string(&p.id).map_err(ConversionError::InvalidProcedureId)?,
        namespace: p.namespace.clone(),
        name: p.name.clone(),
        version: p.version,
        supersedes: p
            .supersedes
            .as_deref()
            .map(|s| ProcedureId::from_string(s).map_err(ConversionError::InvalidProcedureId))
            .transpose()?,
        is_current: p.is_current,
        source: DomainProcedureSource::parse(&p.source)
            .ok_or_else(|| ConversionError::InvalidProcedureField("source", p.source.clone()))?,
        goal: p.goal.clone(),
        intent: p.intent.clone(),
        tags: p.tags.clone(),
        parameters: p.parameters.iter().map(parameter_from_proto).collect(),
        preconditions: p
            .preconditions
            .iter()
            .map(precondition_from_proto)
            .collect::<Result<Vec<_>, _>>()?,
        required_tools: p.required_tools.clone(),
        postconditions: p.postconditions.clone(),
        est_duration_sec: p.est_duration_sec,
        usage_notes: p.usage_notes.clone(),
        context_tags: p.context_tags.clone(),
        body_format: DomainBodyFormat::parse(&p.body_format).ok_or_else(|| {
            ConversionError::InvalidProcedureField("body_format", p.body_format.clone())
        })?,
        content_type: p.content_type.clone(),
        body: p.body.clone(),
        tier: DomainTier::parse(&p.tier)
            .ok_or_else(|| ConversionError::InvalidProcedureField("tier", p.tier.clone()))?,
        use_count: p.use_count,
        success_count: p.success_count,
        failure_count: p.failure_count,
        unknown_count: p.unknown_count,
        last_used_at: p.last_used_at,
        created_at: p.created_at,
        updated_at: p.updated_at,
        stale_at: p.stale_at,
    })
}

fn parameter_to_proto(p: &DomainParameter) -> proto::Parameter {
    proto::Parameter {
        name: p.name.clone(),
        type_name: p.type_name.clone(),
        default: p.default.clone(),
        desc: p.desc.clone(),
    }
}

fn parameter_from_proto(p: &proto::Parameter) -> DomainParameter {
    DomainParameter {
        name: p.name.clone(),
        type_name: p.type_name.clone(),
        default: p.default.clone(),
        desc: p.desc.clone(),
    }
}

fn precondition_to_proto(p: &DomainPrecondition) -> proto::Precondition {
    proto::Precondition {
        kind: p.kind.clone(),
        description: p.description.clone(),
        check: Some(proto::PreconditionCheck {
            match_pattern: Some(proto::ClaimMatch {
                subject: p.check.match_pattern.subject.clone(),
                predicate: p.check.match_pattern.predicate.clone(),
                object: p.check.match_pattern.object.clone(),
            }),
            min_confidence: p.check.min_confidence,
            expect: p.check.expect.as_str().to_string(),
        }),
    }
}

fn precondition_from_proto(p: &proto::Precondition) -> Result<DomainPrecondition, ConversionError> {
    let check = p
        .check
        .as_ref()
        .ok_or(ConversionError::MissingField("precondition.check"))?;
    let pattern = check
        .match_pattern
        .as_ref()
        .ok_or(ConversionError::MissingField(
            "precondition.check.match_pattern",
        ))?;

    Ok(DomainPrecondition {
        kind: p.kind.clone(),
        description: p.description.clone(),
        check: DomainPreconditionCheck {
            match_pattern: DomainClaimMatch {
                subject: pattern.subject.clone(),
                predicate: pattern.predicate.clone(),
                object: pattern.object.clone(),
            },
            min_confidence: check.min_confidence,
            expect: DomainExpect::parse(&check.expect).ok_or_else(|| {
                ConversionError::InvalidProcedureField("expect", check.expect.clone())
            })?,
        },
    })
}

/// Convert an issued [`DomainReceipt`] to its proto form.
pub fn receipt_to_proto(r: &DomainReceipt) -> proto::ExecutionReceipt {
    proto::ExecutionReceipt {
        receipt_id: r.receipt_id.to_string(),
        procedure_id: r.procedure_id.to_string(),
        version: r.version,
        issued_to: r.issued_to.clone(),
        task_id: r.task_id.clone(),
        session_id: r.session_id.clone(),
        issued_at: r.issued_at,
        expires_at: r.expires_at,
        report_to: r.report_to.clone(),
    }
}

/// Convert a proto execution receipt back to its domain form.
pub fn receipt_from_proto(c: &proto::ExecutionReceipt) -> Result<DomainReceipt, ConversionError> {
    Ok(DomainReceipt {
        receipt_id: ProcedureId::from_string(&c.receipt_id)
            .map_err(ConversionError::InvalidProcedureId)?,
        procedure_id: ProcedureId::from_string(&c.procedure_id)
            .map_err(ConversionError::InvalidProcedureId)?,
        version: c.version,
        issued_to: c.issued_to.clone(),
        task_id: c.task_id.clone(),
        session_id: c.session_id.clone(),
        issued_at: c.issued_at,
        expires_at: c.expires_at,
        report_to: c.report_to.clone(),
    })
}

/// Parse a [`ProcedureId`] from its UUIDv7 string form.
pub fn procedure_id_from_proto(s: &str) -> Result<ProcedureId, ConversionError> {
    ProcedureId::from_string(s).map_err(ConversionError::InvalidProcedureId)
}

/// Build a domain [`OutcomeReport`] from the wire fields of a report request.
///
/// `failure_mode` is only meaningful for a `failure` outcome; `failed_step`
/// names the step when the mode is `step_failed`.
pub fn outcome_report_from_proto(
    receipt_id: ProcedureId,
    outcome: &str,
    failure_mode: Option<&str>,
    failed_step: Option<&str>,
    executor_confidence: Option<f64>,
    cost: Option<f64>,
    notes: Option<String>,
) -> Result<OutcomeReport, ConversionError> {
    let outcome = match outcome {
        "success" => DomainOutcome::Success,
        "failure" => DomainOutcome::Failure,
        "abandoned" => DomainOutcome::Abandoned,
        other => return Err(ConversionError::InvalidOutcome(other.to_string())),
    };

    let failure_mode = match failure_mode {
        None => None,
        Some("preconditions_stale") => Some(DomainFailureMode::PreconditionsStale),
        Some("bad_result") => Some(DomainFailureMode::BadResult),
        Some("executor_error") => Some(DomainFailureMode::ExecutorError),
        Some("step_failed") => Some(DomainFailureMode::StepFailed(
            failed_step.unwrap_or_default().to_string(),
        )),
        Some(other) => return Err(ConversionError::InvalidFailureMode(other.to_string())),
    };

    // A failure attribution only makes sense for a failure outcome; carrying one
    // on a success would silently mis-attribute the report.
    if failure_mode.is_some() && outcome != DomainOutcome::Failure {
        return Err(ConversionError::InvalidFailureMode(
            "failure_mode is only valid when outcome = failure".to_string(),
        ));
    }

    Ok(OutcomeReport {
        receipt_id,
        outcome,
        failure_mode,
        executor_confidence,
        cost,
        notes,
    })
}

// ---- Goal traversal (design §3.2, §4.1) ----

/// Parse a [`GoalId`] from its UUIDv7 string form.
pub fn goal_id_from_proto(s: &str) -> Result<GoalId, ConversionError> {
    GoalId::from_string(s).map_err(ConversionError::InvalidGoalId)
}

/// Convert a domain goal onto the wire.
pub fn goal_to_proto(g: &DomainGoal) -> proto::Goal {
    proto::Goal {
        id: g.id.to_string(),
        namespace: g.namespace.clone(),
        name: g.name.clone(),
        intent: g.intent.clone(),
        definition_of_done: g.definition_of_done.clone(),
        tier: g.tier.as_str().to_string(),
        created_at: g.created_at,
        updated_at: g.updated_at,
        stale_at: g.stale_at,
    }
}

/// Convert a proto goal back to its domain form.
pub fn goal_from_proto(g: &proto::Goal) -> Result<DomainGoal, ConversionError> {
    Ok(DomainGoal {
        id: goal_id_from_proto(&g.id)?,
        namespace: g.namespace.clone(),
        name: g.name.clone(),
        intent: g.intent.clone(),
        definition_of_done: g.definition_of_done.clone(),
        tier: DomainTier::parse(&g.tier)
            .ok_or_else(|| ConversionError::InvalidGoalField("tier", g.tier.clone()))?,
        created_at: g.created_at,
        updated_at: g.updated_at,
        stale_at: g.stale_at,
    })
}

/// Convert a ranked expand candidate onto the wire.
///
/// The child is carried as a `(kind, id)` pair rather than a oneof so the wire
/// shape stays flat and a client can route on `child_kind` without unwrapping.
pub fn expanded_candidate_to_proto(c: &DomainExpandedCandidate) -> proto::ExpandedCandidate {
    proto::ExpandedCandidate {
        child_kind: c.child.kind().as_str().to_string(),
        child_id: match c.child {
            DomainChildRef::Goal(id) => id.to_string(),
            DomainChildRef::Procedure(id) => id.to_string(),
        },
        role: c.role.as_str().to_string(),
        context_tags: c.context_tags.clone(),
        usage_notes: c.usage_notes.clone(),
        effectiveness: c.effectiveness,
        context_match: c.context_match as u32,
    }
}

/// Convert a proto expand candidate back to its domain form.
pub fn expanded_candidate_from_proto(
    c: &proto::ExpandedCandidate,
) -> Result<DomainExpandedCandidate, ConversionError> {
    let kind = DomainChildKind::parse(&c.child_kind)
        .ok_or_else(|| ConversionError::InvalidGoalField("child_kind", c.child_kind.clone()))?;
    let child = match kind {
        DomainChildKind::Goal => DomainChildRef::Goal(goal_id_from_proto(&c.child_id)?),
        DomainChildKind::Procedure => {
            DomainChildRef::Procedure(procedure_id_from_proto(&c.child_id)?)
        }
    };
    Ok(DomainExpandedCandidate {
        child,
        role: DomainEdgeRole::parse(&c.role)
            .ok_or_else(|| ConversionError::InvalidGoalField("role", c.role.clone()))?,
        context_tags: c.context_tags.clone(),
        usage_notes: c.usage_notes.clone(),
        effectiveness: c.effectiveness,
        context_match: c.context_match as usize,
    })
}

/// Convert a factor reading onto the wire.
pub fn factor_reading_to_proto(f: &DomainFactorReading) -> proto::FactorReading {
    proto::FactorReading {
        subject: f.subject.clone(),
        predicate: f.predicate.clone(),
        object: f.object.clone(),
        confidence: Some(proto::ConfidenceInterval {
            lower: f.confidence.0,
            upper: f.confidence.1,
        }),
    }
}

/// Convert a proto factor reading back to its domain form.
///
/// A missing confidence interval is an error rather than a default: a reading
/// is evidence the agent weighs, and silently substituting `[0, 0]` would
/// misrepresent how strongly a factor held.
pub fn factor_reading_from_proto(
    f: &proto::FactorReading,
) -> Result<DomainFactorReading, ConversionError> {
    let conf = f
        .confidence
        .as_ref()
        .ok_or(ConversionError::MissingField("factor_reading.confidence"))?;
    Ok(DomainFactorReading {
        subject: f.subject.clone(),
        predicate: f.predicate.clone(),
        object: f.object.clone(),
        confidence: (conf.lower, conf.upper),
    })
}

/// Convert a proto traversal context to its domain form. An absent context is
/// an empty one — a hop with no situational tags still ranks by effectiveness.
pub fn traversal_context_from_proto(c: Option<&proto::TraversalContext>) -> DomainTraversalContext {
    DomainTraversalContext {
        context_tags: c.map(|c| c.context_tags.clone()).unwrap_or_default(),
    }
}

/// Convert a domain traversal context onto the wire.
pub fn traversal_context_to_proto(c: &DomainTraversalContext) -> proto::TraversalContext {
    proto::TraversalContext {
        context_tags: c.context_tags.clone(),
    }
}

#[cfg(test)]
mod tests {
    use boswell_domain::{
        BodyFormat, ClaimMatch as DClaimMatch, Expect, Parameter, Precondition, PreconditionCheck,
        Procedure, ProcedureSource, Tier,
    };

    /// The wire shape claims to be lossless, so a full-fat procedure must
    /// survive the round trip unchanged — otherwise an executor silently loses
    /// the signature it needs to decide whether the procedure applies.
    #[test]
    fn procedure_survives_a_proto_round_trip() {
        let original = Procedure {
            id: ProcedureId::new(),
            namespace: "person:jd".into(),
            name: "omelette-classic".into(),
            version: 3,
            supersedes: Some(ProcedureId::new()),
            is_current: true,
            source: ProcedureSource::Learned,
            goal: "goal:person:jd/cook-eggs".into(),
            intent: "cook eggs into a classic omelette".into(),
            tags: vec!["breakfast".into(), "eggs".into()],
            parameters: vec![Parameter {
                name: "count".into(),
                type_name: "int".into(),
                default: Some("2".into()),
                desc: Some("how many eggs".into()),
            }],
            preconditions: vec![Precondition {
                kind: "resource".into(),
                description: "eggs on hand".into(),
                check: PreconditionCheck {
                    match_pattern: DClaimMatch {
                        subject: "jd".into(),
                        predicate: "has".into(),
                        object: "eggs".into(),
                    },
                    min_confidence: 0.6,
                    expect: Expect::Absent,
                },
            }],
            required_tools: vec!["pan".into(), "whisk".into()],
            postconditions: vec!["eggs are cooked".into()],
            est_duration_sec: Some(300),
            usage_notes: "keep the heat low".into(),
            context_tags: vec!["kitchen".into()],
            body_format: BodyFormat::Dsl,
            content_type: "application/x-boswell-steps".into(),
            body: "1. beat eggs\n2. pour".into(),
            tier: Tier::Project,
            use_count: 12,
            success_count: 9,
            failure_count: 2,
            unknown_count: 1,
            last_used_at: Some(1_700_000_000_000),
            created_at: 1_600_000_000_000,
            updated_at: 1_700_000_000_000,
            stale_at: Some(1_800_000_000_000),
        };

        let round_tripped = procedure_from_proto(&procedure_to_proto(&original)).unwrap();
        assert_eq!(round_tripped, original);
    }

    /// A receipt is the executor's only handle on its obligation, so its
    /// correlation fields have to survive the wire too.
    #[test]
    fn execution_receipt_survives_a_proto_round_trip() {
        let original = DomainReceipt {
            receipt_id: ProcedureId::new(),
            procedure_id: ProcedureId::new(),
            version: 2,
            issued_to: "agent:cook-1".into(),
            task_id: Some("task-7".into()),
            session_id: Some("session-3".into()),
            issued_at: 1_700_000_000_000,
            expires_at: 1_700_000_600_000,
            report_to: Some("https://example.invalid/report".into()),
        };

        let round_tripped = receipt_from_proto(&receipt_to_proto(&original)).unwrap();
        assert_eq!(round_tripped, original);
    }

    /// `step_failed` carries the step name; dropping it would erase the
    /// diagnosis the attribution exists to provide.
    #[test]
    fn step_failed_carries_the_step_name() {
        let report = outcome_report_from_proto(
            ProcedureId::new(),
            "failure",
            Some("step_failed"),
            Some("whisk"),
            None,
            None,
            None,
        )
        .unwrap();

        assert_eq!(
            report.failure_mode,
            Some(DomainFailureMode::StepFailed("whisk".into()))
        );
        assert!(report.is_negative());
    }

    /// An unrecognised outcome must not be silently coerced into a success.
    #[test]
    fn an_unknown_outcome_is_rejected() {
        let err = outcome_report_from_proto(
            ProcedureId::new(),
            "probably-fine",
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap_err();

        assert!(matches!(err, ConversionError::InvalidOutcome(_)));
    }

    use super::*;

    #[test]
    fn test_tier_roundtrip() {
        let tiers = vec![
            ("ephemeral", proto::Tier::Ephemeral),
            ("task", proto::Tier::Task),
            ("project", proto::Tier::Project),
            ("permanent", proto::Tier::Permanent),
        ];

        for (tier_str, proto_tier) in tiers {
            let proto = tier_to_proto(tier_str);
            assert_eq!(proto, proto_tier);
            let back = tier_from_proto(proto).unwrap();
            assert_eq!(tier_str, back);
        }
    }

    #[test]
    fn test_confidence_roundtrip() {
        let conf = DomainConfidence::new(0.7, 0.9);
        let proto = confidence_to_proto(conf);
        let back = confidence_from_proto(Some(proto)).unwrap();
        assert_eq!(conf.lower, back.lower);
        assert_eq!(conf.upper, back.upper);
    }

    #[test]
    fn test_invalid_confidence() {
        let invalid = proto::ConfidenceInterval {
            lower: 0.9,
            upper: 0.1, // Invalid: lower > upper
        };
        assert!(confidence_from_proto(Some(invalid)).is_err());
    }

    #[test]
    fn test_claim_roundtrip() {
        let claim = Claim {
            id: ClaimId::new(),
            namespace: "test".to_string(),
            subject: "Alice".to_string(),
            predicate: "knows".to_string(),
            object: "Bob".to_string(),
            source_type: "inference".to_string(),
            confidence: (0.8, 0.95),
            tier: "task".to_string(),
            created_at: 1000000,
            stale_at: None,
        };

        let proto = claim_to_proto(claim.clone());
        let back = claim_from_proto(proto).unwrap();

        assert_eq!(claim.id, back.id);
        assert_eq!(claim.namespace, back.namespace);
        assert_eq!(claim.subject, back.subject);
        assert_eq!(claim.confidence, back.confidence);
        assert_eq!(claim.tier, back.tier);
        assert_eq!(claim.source_type, back.source_type);
    }

    #[test]
    fn test_relationship_roundtrip() {
        let rel = DomainRelationship {
            from_claim: ClaimId::new(),
            to_claim: ClaimId::new(),
            relationship_type: DomainRelationshipType::Supports,
            strength: 0.75,
            created_at: 1_700_000_000,
        };

        let proto = relationship_to_proto(rel.clone());
        let back = relationship_from_proto(proto).unwrap();

        assert_eq!(rel.from_claim, back.from_claim);
        assert_eq!(rel.to_claim, back.to_claim);
        assert_eq!(rel.relationship_type, back.relationship_type);
        assert_eq!(rel.strength, back.strength);
        assert_eq!(rel.created_at, back.created_at);
    }

    #[test]
    fn test_relationship_type_all_variants_roundtrip() {
        for rt in [
            DomainRelationshipType::Supports,
            DomainRelationshipType::Contradicts,
            DomainRelationshipType::DerivedFrom,
            DomainRelationshipType::References,
            DomainRelationshipType::Supersedes,
        ] {
            let proto = relationship_type_to_proto(rt);
            let back = relationship_type_from_proto(proto as i32).unwrap();
            assert_eq!(rt, back);
        }
    }

    #[test]
    fn test_relationship_type_unspecified_is_error() {
        assert!(relationship_type_from_proto(0).is_err());
    }
}
