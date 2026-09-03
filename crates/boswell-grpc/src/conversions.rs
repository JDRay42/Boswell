//! Type conversions between proto and domain types
//!
//! Handles bidirectional conversion between gRPC protobuf types and internal domain types.

use crate::proto;
use boswell_domain::{
    Claim, ClaimId, ConfidenceInterval as DomainConfidence, Relationship as DomainRelationship,
    RelationshipType as DomainRelationshipType, Tier as DomainTier,
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

#[cfg(test)]
mod tests {
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
