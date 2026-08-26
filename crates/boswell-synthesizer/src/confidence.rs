//! Confidence propagation for synthesized claims.
//!
//! Per `docs/architecture/06-synthesizer.md`:
//!
//! - The lower bound of a derived claim cannot exceed the minimum lower bound
//!   of its constituents.
//! - The upper bound is bounded by the LLM's assessed confidence in the inference.
//! - Uncertainty propagates *outward* — a derived claim is never more certain
//!   than its foundations.

use boswell_domain::Claim;

/// Compute the confidence interval for a claim derived from `constituents`,
/// given the LLM's assessed confidence in the inference itself.
///
/// # Rules
///
/// - `lower = min(constituent lowers) * llm_lower`, which is guaranteed to be
///   `<= min(constituent lowers)` so uncertainty widens downward.
/// - `upper = llm_upper`, clamped so the interval stays ordered and within
///   `[0.0, 1.0]`.
///
/// If `constituents` is empty, the LLM confidence is returned unchanged
/// (clamped to a valid interval).
pub fn propagate_confidence(constituents: &[Claim], llm_confidence: (f64, f64)) -> (f64, f64) {
    let (llm_lower, llm_upper) = clamp_interval(llm_confidence);

    let min_lower = constituents
        .iter()
        .map(|c| c.confidence.0)
        .fold(f64::INFINITY, f64::min);

    if !min_lower.is_finite() {
        // No constituents: fall back to the LLM's own interval.
        return (llm_lower, llm_upper);
    }

    // Widen downward: the derived lower bound can never exceed the weakest
    // constituent's lower bound, and is further discounted by the LLM's own
    // uncertainty about the inference.
    let lower = (min_lower * llm_lower).clamp(0.0, min_lower);

    // The upper bound is capped by the LLM's confidence in the inference and
    // must remain >= the lower bound.
    let upper = llm_upper.clamp(lower, 1.0);

    (lower, upper)
}

/// Clamp an interval into `[0.0, 1.0]` with `lower <= upper`.
fn clamp_interval((lower, upper): (f64, f64)) -> (f64, f64) {
    let lower = lower.clamp(0.0, 1.0);
    let upper = upper.clamp(0.0, 1.0);
    if lower <= upper {
        (lower, upper)
    } else {
        (upper, lower)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use boswell_domain::{Claim, ClaimId};

    fn claim_with_conf(lower: f64, upper: f64) -> Claim {
        Claim {
            id: ClaimId::new(),
            namespace: "test".to_string(),
            subject: "s".to_string(),
            predicate: "p".to_string(),
            object: "o".to_string(),
            source_type: "assertion".to_string(),
            confidence: (lower, upper),
            tier: "task".to_string(),
            created_at: 0,
            stale_at: None,
        }
    }

    #[test]
    fn test_lower_never_exceeds_min_constituent_lower() {
        let constituents = vec![claim_with_conf(0.6, 0.9), claim_with_conf(0.4, 0.8)];
        let (lower, _upper) = propagate_confidence(&constituents, (0.9, 0.95));
        // min constituent lower is 0.4
        assert!(lower <= 0.4, "lower {} must not exceed 0.4", lower);
    }

    #[test]
    fn test_upper_bounded_by_llm() {
        let constituents = vec![claim_with_conf(0.9, 0.99)];
        let (_lower, upper) = propagate_confidence(&constituents, (0.5, 0.6));
        assert!(upper <= 0.6, "upper {} must be bounded by LLM 0.6", upper);
    }

    #[test]
    fn test_uncertainty_widens_outward() {
        // The derived interval should be at least as wide as it is discounted:
        // lower drops below the weakest constituent.
        let constituents = vec![claim_with_conf(0.8, 0.9)];
        let (lower, upper) = propagate_confidence(&constituents, (0.7, 0.85));
        assert!(lower < 0.8);
        assert!(upper <= 0.85);
        assert!(lower <= upper);
    }

    #[test]
    fn test_interval_always_ordered_and_valid() {
        let constituents = vec![claim_with_conf(0.1, 0.2), claim_with_conf(0.3, 0.4)];
        let (lower, upper) = propagate_confidence(&constituents, (0.9, 0.1));
        assert!((0.0..=1.0).contains(&lower));
        assert!((0.0..=1.0).contains(&upper));
        assert!(lower <= upper);
    }

    #[test]
    fn test_empty_constituents_uses_llm() {
        let (lower, upper) = propagate_confidence(&[], (0.3, 0.7));
        assert_eq!((lower, upper), (0.3, 0.7));
    }
}
