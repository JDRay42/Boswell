//! Confidence computation module (per ADR-007)
//!
//! Implements the deterministic confidence formula for computing effective
//! confidence intervals from provenance, staleness, and relationship data.

use crate::{ConfidenceInterval, ProvenanceEntry, Relationship};
use crate::relationship::RelationshipType;
use std::collections::HashSet;

/// Tunable constant for support relationship boost (default: 0.1)
pub const BOOST_FACTOR: f64 = 0.1;

/// Tunable constant for contradiction relationship penalty (default: 0.2)
pub const PENALTY_FACTOR: f64 = 0.2;

/// Configuration for confidence computation
#[derive(Debug, Clone)]
pub struct ConfidenceConfig {
    /// Boost factor for supporting relationships
    pub boost_factor: f64,
    /// Penalty factor for contradicting relationships
    pub penalty_factor: f64,
    /// Instance trust scaling factor [0.0, 1.0]
    pub instance_trust: f64,
}

impl Default for ConfidenceConfig {
    fn default() -> Self {
        Self {
            boost_factor: BOOST_FACTOR,
            penalty_factor: PENALTY_FACTOR,
            instance_trust: 1.0, // Full trust for local instance
        }
    }
}

/// Data about a related claim needed for confidence computation
#[derive(Debug, Clone)]
pub struct RelatedClaimData {
    /// The claim's stale-adjusted confidence interval
    pub stale_confidence: ConfidenceInterval,
}

/// Compute effective confidence interval using the deterministic formula
///
/// This implements the four-step formula from ADR-007:
/// 1. Provenance aggregation
/// 2. Staleness decay
/// 3. Relationship adjustment
/// 4. Instance trust scaling
///
/// # Arguments
/// * `provenance` - List of provenance entries for the claim
/// * `current_time` - Current timestamp for staleness calculation
/// * `stale_at` - When the claim should be considered stale (None = no staleness)
/// * `half_life_ms` - Half-life for staleness decay in milliseconds
/// * `relationships` - Relationships to other claims
/// * `related_claims` - Confidence data for related claims (to avoid circular deps)
/// * `config` - Configuration for computation
pub fn compute_effective_confidence(
    provenance: &[ProvenanceEntry],
    current_time: u64,
    stale_at: Option<u64>,
    half_life_ms: u64,
    _relationships: &[Relationship],
    related_claims: &[(Relationship, RelatedClaimData)],
    config: &ConfidenceConfig,
) -> ConfidenceInterval {
    // Step 1: Provenance aggregation
    let (aggregate_lower, aggregate_upper) = aggregate_provenance(provenance);
    
    // Step 2: Staleness decay
    let staleness_factor = compute_staleness_factor(current_time, stale_at, half_life_ms);
    let stale_lower = aggregate_lower * staleness_factor;
    let stale_upper = aggregate_upper * staleness_factor;
    
    // Step 3: Relationship adjustment
    let (support_boost, contradiction_penalty) = 
        compute_relationship_adjustments(related_claims, config);
    
    let adjusted_lower = stale_lower * contradiction_penalty;
    let adjusted_upper = (stale_upper * support_boost * contradiction_penalty).min(1.0);
    
    // Step 4: Instance trust scaling
    let final_lower = (adjusted_lower * config.instance_trust).clamp(0.0, 1.0);
    let final_upper = (adjusted_upper * config.instance_trust).clamp(0.0, 1.0);
    
    // Ensure lower <= upper
    let final_lower = final_lower.min(final_upper);
    
    ConfidenceInterval::new(final_lower, final_upper)
}

/// Step 1: Aggregate confidence from multiple provenance entries
///
/// Uses the "probability of at least one source being right" model for upper bound
/// and conservative anchoring with diversity factor for lower bound.
fn aggregate_provenance(provenance: &[ProvenanceEntry]) -> (f64, f64) {
    if provenance.is_empty() {
        return (0.0, 0.0);
    }
    
    // For now, assume each provenance entry has an implied confidence of 0.8
    // In a full implementation, this would come from the provenance entry itself
    let confidence_values: Vec<f64> = provenance.iter().map(|_| 0.8).collect();
    
    // Upper bound: 1 - ∏(1 - cᵢ)
    let aggregate_upper = 1.0 - confidence_values.iter()
        .map(|&c| 1.0 - c)
        .product::<f64>();
    
    // Source diversity factor
    let unique_source_types: HashSet<&str> = 
        provenance.iter().map(|p| p.source_type.as_str()).collect();
    let source_diversity_factor = 0.5 + (0.5 * (unique_source_types.len() as f64 / 3.0).min(1.0));
    
    // Lower bound: max confidence × diversity factor
    let max_confidence = confidence_values.iter().cloned().fold(0.0, f64::max);
    let aggregate_lower = max_confidence * source_diversity_factor;
    
    (aggregate_lower, aggregate_upper)
}

/// Step 2: Compute staleness decay factor using half-life model
fn compute_staleness_factor(current_time: u64, stale_at: Option<u64>, half_life_ms: u64) -> f64 {
    let Some(stale_at) = stale_at else {
        return 1.0; // No staleness
    };
    
    if current_time <= stale_at {
        return 1.0; // Not yet stale
    }
    
    let time_since_staleness = current_time - stale_at;
    let half_lives = time_since_staleness as f64 / half_life_ms as f64;
    
    // staleness_factor = 0.5^(half_lives)
    0.5_f64.powf(half_lives)
}

/// Step 3: Compute relationship adjustments (support boost and contradiction penalty)
///
/// Note: This uses related claims' stale-adjusted confidence (not their full effective
/// confidence) to avoid circular dependencies.
fn compute_relationship_adjustments(
    related_claims: &[(Relationship, RelatedClaimData)],
    config: &ConfidenceConfig,
) -> (f64, f64) {
    let mut support_sum = 0.0;
    let mut contradiction_sum = 0.0;
    
    for (rel, data) in related_claims {
        let weighted_confidence = data.stale_confidence.upper * rel.strength;
        
        match rel.relationship_type {
            RelationshipType::Supports => {
                support_sum += weighted_confidence;
            }
            RelationshipType::Contradicts => {
                contradiction_sum += weighted_confidence;
            }
            _ => {} // Other relationship types don't affect confidence
        }
    }
    
    let support_boost = 1.0 + (support_sum * config.boost_factor);
    let contradiction_penalty = 1.0 - (contradiction_sum * config.penalty_factor);
    
    // Ensure penalty doesn't go below 0
    let contradiction_penalty = contradiction_penalty.max(0.0);
    
    (support_boost, contradiction_penalty)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_aggregate_provenance_single_source() {
        let provenance = vec![
            ProvenanceEntry::new("user:alice".to_string(), 1000, "user".to_string()),
        ];
        
        let (lower, upper) = aggregate_provenance(&provenance);
        
        // Single source gets diversity factor = 0.5 + 0.5 * (1/3) = 0.667
        // Lower = 0.8 * 0.667 = 0.533
        assert!((lower - 0.533).abs() < 0.01);
        // Upper is 0.8
        assert!((upper - 0.8).abs() < 0.01);
    }

    #[test]
    fn test_aggregate_provenance_multiple_sources() {
        let provenance = vec![
            ProvenanceEntry::new("user:alice".to_string(), 1000, "user".to_string()),
            ProvenanceEntry::new("agent:gpt4".to_string(), 1001, "agent".to_string()),
            ProvenanceEntry::new("extraction:doc1".to_string(), 1002, "extraction".to_string()),
        ];
        
        let (lower, upper) = aggregate_provenance(&provenance);
        
        // Three different source types = full diversity (factor = 1.0)
        // Lower = 0.8 * 1.0 = 0.8
        assert!((lower - 0.8).abs() < 0.01);
        
        // Upper = 1 - (1 - 0.8)^3 = 1 - 0.008 = 0.992
        assert!((upper - 0.992).abs() < 0.01);
    }

    #[test]
    fn test_staleness_factor_not_stale() {
        let factor = compute_staleness_factor(1000, Some(2000), 1000);
        assert_eq!(factor, 1.0);
    }

    #[test]
    fn test_staleness_factor_one_half_life() {
        let factor = compute_staleness_factor(2000, Some(1000), 1000);
        assert!((factor - 0.5).abs() < 0.01);
    }

    #[test]
    fn test_staleness_factor_two_half_lives() {
        let factor = compute_staleness_factor(3000, Some(1000), 1000);
        assert!((factor - 0.25).abs() < 0.01);
    }

    #[test]
    fn test_relationship_adjustments_support() {
        let config = ConfidenceConfig::default();
        let related = vec![
            (
                Relationship::new(
                    crate::ClaimId::from_value(1),
                    crate::ClaimId::from_value(2),
                    RelationshipType::Supports,
                    1.0,
                    1000,
                ),
                RelatedClaimData {
                    stale_confidence: ConfidenceInterval::new(0.7, 0.9),
                },
            ),
        ];
        
        let (support_boost, contradiction_penalty) = 
            compute_relationship_adjustments(&related, &config);
        
        // Support boost = 1.0 + (0.9 * 1.0 * 0.1) = 1.09
        assert!((support_boost - 1.09).abs() < 0.01);
        assert_eq!(contradiction_penalty, 1.0);
    }

    #[test]
    fn test_relationship_adjustments_contradiction() {
        let config = ConfidenceConfig::default();
        let related = vec![
            (
                Relationship::new(
                    crate::ClaimId::from_value(1),
                    crate::ClaimId::from_value(2),
                    RelationshipType::Contradicts,
                    1.0,
                    1000,
                ),
                RelatedClaimData {
                    stale_confidence: ConfidenceInterval::new(0.7, 0.9),
                },
            ),
        ];
        
        let (support_boost, contradiction_penalty) = 
            compute_relationship_adjustments(&related, &config);
        
        assert_eq!(support_boost, 1.0);
        // Contradiction penalty = 1.0 - (0.9 * 1.0 * 0.2) = 0.82
        assert!((contradiction_penalty - 0.82).abs() < 0.01);
    }

    #[test]
    fn test_full_confidence_computation() {
        let provenance = vec![
            ProvenanceEntry::new("user:alice".to_string(), 1000, "user".to_string()),
        ];
        
        let config = ConfidenceConfig::default();
        let confidence = compute_effective_confidence(
            &provenance,
            1000,
            None,  // No staleness
            1000,
            &[],
            &[],
            &config,
        );
        
        // Should match aggregate provenance results with single source diversity factor
        assert!((confidence.lower - 0.533).abs() < 0.01);
        assert!((confidence.upper - 0.8).abs() < 0.01);
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Property: Computed confidence always has lower <= upper
        #[test]
        fn test_confidence_bounds_invariant(
            num_provenance in 1..=5usize,
            current_time in 1000u64..10000u64,
            stale_at in 0u64..10000u64,
        ) {
            let provenance: Vec<_> = (0..num_provenance)
                .map(|i| ProvenanceEntry::new(
                    format!("source:{}", i),
                    current_time - 100,
                    format!("type{}", i % 3)
                ))
                .collect();

            let config = ConfidenceConfig::default();
            let confidence = compute_effective_confidence(
                &provenance,
                current_time,
                Some(stale_at),
                1000,
                &[],
                &[],
                &config,
            );

            prop_assert!(confidence.lower <= confidence.upper,
                "Lower {} must be <= upper {}", confidence.lower, confidence.upper);
        }

        /// Property: Confidence values are always in [0, 1]
        #[test]
        fn test_confidence_range(
            num_provenance in 1..=5usize,
            current_time in 1000u64..10000u64,
        ) {
            let provenance: Vec<_> = (0..num_provenance)
                .map(|i| ProvenanceEntry::new(
                    format!("source:{}", i),
                    current_time - 100,
                    format!("type{}", i % 3)
                ))
                .collect();

            let config = ConfidenceConfig::default();
            let confidence = compute_effective_confidence(
                &provenance,
                current_time,
                None,
                1000,
                &[],
                &[],
                &config,
            );

            prop_assert!(confidence.lower >= 0.0 && confidence.lower <= 1.0);
            prop_assert!(confidence.upper >= 0.0 && confidence.upper <= 1.0);
        }

        /// Property: More provenance sources increase upper bound
        #[test]
        fn test_provenance_increases_confidence(
            base_sources in 1..=3usize,
            additional_sources in 0..=2usize,
        ) {
            let provenance1: Vec<_> = (0..base_sources)
                .map(|i| ProvenanceEntry::new(
                    format!("source:{}", i),
                    1000,
                    format!("type{}", i)
                ))
                .collect();

            let provenance2: Vec<_> = (0..(base_sources + additional_sources))
                .map(|i| ProvenanceEntry::new(
                    format!("source:{}", i),
                    1000,
                    format!("type{}", i)
                ))
                .collect();

            let config = ConfidenceConfig::default();
            
            let conf1 = compute_effective_confidence(
                &provenance1, 1000, None, 1000, &[], &[], &config
            );
            
            let conf2 = compute_effective_confidence(
                &provenance2, 1000, None, 1000, &[], &[], &config
            );

            // More provenance should not decrease upper bound
            prop_assert!(conf2.upper >= conf1.upper);
        }

        /// Property: Staleness always reduces or maintains confidence
        #[test]
        fn test_staleness_decreases_confidence(
            time_delta in 1u64..10000u64,
        ) {
            let provenance = vec![
                ProvenanceEntry::new("source:1".to_string(), 1000, "user".to_string()),
            ];

            let config = ConfidenceConfig::default();
            
            // Fresh claim
            let conf_fresh = compute_effective_confidence(
                &provenance, 1000, Some(2000), 1000, &[], &[], &config
            );
            
            // Stale claim
            let conf_stale = compute_effective_confidence(
                &provenance, 2000 + time_delta, Some(2000), 1000, &[], &[], &config
            );

            // Staleness should not increase confidence
            prop_assert!(conf_stale.lower <= conf_fresh.lower);
            prop_assert!(conf_stale.upper <= conf_fresh.upper);
        }

        /// Property: Contradictions reduce confidence
        #[test]
        fn test_contradictions_reduce_confidence(
            num_contradictions in 0..=3usize,
        ) {
            let provenance = vec![
                ProvenanceEntry::new("source:1".to_string(), 1000, "user".to_string()),
            ];

            let contradictions: Vec<_> = (0..num_contradictions)
                .map(|i| {
                    (
                        Relationship::new(
                            crate::ClaimId::from_value(i as u128),
                            crate::ClaimId::from_value(100),
                            RelationshipType::Contradicts,
                            0.5,
                            1000,
                        ),
                        RelatedClaimData {
                            stale_confidence: ConfidenceInterval::new(0.7, 0.9),
                        },
                    )
                })
                .collect();

            let config = ConfidenceConfig::default();
            
            let conf_no_contra = compute_effective_confidence(
                &provenance, 1000, None, 1000, &[], &[], &config
            );
            
            let conf_with_contra = compute_effective_confidence(
                &provenance, 1000, None, 1000, &[], &contradictions, &config
            );

            if num_contradictions > 0 {
                // Contradictions should reduce confidence
                prop_assert!(conf_with_contra.lower <= conf_no_contra.lower);
                prop_assert!(conf_with_contra.upper <= conf_no_contra.upper);
            }
        }
    }
}
