//! Age-based confidence decay (per ADR-007).
//!
//! Confidence fades organically as a claim ages without reinforcement. Decay is
//! an exponential half-life model applied per tier: ephemeral claims fade
//! quickly, permanent claims do not decay at all.
//!
//! ```text
//! effective = base * 0.5 ^ (age / half_life(tier))
//! ```
//!
//! This is the deterministic staleness component of the confidence fast path.
//! Relationship (support/contradiction) adjustments live in
//! [`crate::confidence_computation`]; decay here depends only on the claim's own
//! tier and age, so it can be computed without loading related claims.

use crate::Tier;

/// Per-tier half-lives (in seconds) controlling how fast confidence decays.
#[derive(Debug, Clone)]
pub struct DecayConfig {
    /// Half-life for ephemeral claims.
    pub ephemeral_half_life_secs: u64,
    /// Half-life for task claims.
    pub task_half_life_secs: u64,
    /// Half-life for project claims.
    pub project_half_life_secs: u64,
    /// Half-life for permanent claims. `None` means permanent claims never decay.
    pub permanent_half_life_secs: Option<u64>,
}

impl Default for DecayConfig {
    fn default() -> Self {
        Self {
            ephemeral_half_life_secs: 6 * 3600,     // 6 hours
            task_half_life_secs: 12 * 3600,         // 12 hours
            project_half_life_secs: 45 * 24 * 3600, // 45 days
            permanent_half_life_secs: None,         // never decays
        }
    }
}

impl DecayConfig {
    /// Half-life for a tier string, or `None` when that tier does not decay
    /// (permanent by default) or the tier string is unrecognized.
    pub fn half_life_for(&self, tier: &str) -> Option<u64> {
        match Tier::parse(tier) {
            Some(Tier::Ephemeral) => Some(self.ephemeral_half_life_secs),
            Some(Tier::Task) => Some(self.task_half_life_secs),
            Some(Tier::Project) => Some(self.project_half_life_secs),
            Some(Tier::Permanent) => self.permanent_half_life_secs,
            None => None, // unknown tier: do not decay
        }
    }
}

/// Exponential decay factor in `(0.0, 1.0]` for a given age and half-life.
///
/// Returns `1.0` when `half_life_secs == 0` (guards against divide-by-zero) or
/// when `age_secs == 0`.
pub fn decay_factor(age_secs: u64, half_life_secs: u64) -> f64 {
    if half_life_secs == 0 || age_secs == 0 {
        return 1.0;
    }
    0.5_f64.powf(age_secs as f64 / half_life_secs as f64)
}

/// Apply age-based decay to a base confidence interval.
///
/// `created_at` and `now` are Unix timestamps in **seconds**. A claim whose tier
/// does not decay (e.g. permanent) is returned unchanged, as is a claim with
/// `now <= created_at` (clock skew / not yet aged). Both bounds are scaled by the
/// same factor, so a decayed interval shrinks toward zero as the claim ages.
pub fn decayed_confidence(
    base: (f64, f64),
    tier: &str,
    created_at: u64,
    now: u64,
    config: &DecayConfig,
) -> (f64, f64) {
    let Some(half_life) = config.half_life_for(tier) else {
        return base;
    };
    let age = now.saturating_sub(created_at);
    let factor = decay_factor(age, half_life);
    (base.0 * factor, base.1 * factor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decay_factor_edges() {
        assert_eq!(decay_factor(0, 100), 1.0);
        assert_eq!(decay_factor(100, 0), 1.0);
        assert!((decay_factor(100, 100) - 0.5).abs() < 1e-9);
        assert!((decay_factor(200, 100) - 0.25).abs() < 1e-9);
    }

    #[test]
    fn test_decay_factor_monotonic_decreasing() {
        let mut prev = 1.0;
        for age in [0u64, 3600, 7200, 21600, 86400] {
            let f = decay_factor(age, 6 * 3600);
            assert!(f <= prev, "factor should not increase with age");
            assert!(f > 0.0 && f <= 1.0);
            prev = f;
        }
    }

    #[test]
    fn test_permanent_does_not_decay() {
        let cfg = DecayConfig::default();
        let base = (0.7, 0.9);
        // A year later, permanent is unchanged.
        let out = decayed_confidence(base, "permanent", 0, 365 * 24 * 3600, &cfg);
        assert_eq!(out, base);
    }

    #[test]
    fn test_ephemeral_halves_at_half_life() {
        let cfg = DecayConfig::default();
        let base = (0.6, 0.8);
        // At exactly one half-life (6h), both bounds are halved.
        let out = decayed_confidence(base, "ephemeral", 0, 6 * 3600, &cfg);
        assert!((out.0 - 0.3).abs() < 1e-6);
        assert!((out.1 - 0.4).abs() < 1e-6);
    }

    #[test]
    fn test_unknown_tier_does_not_decay() {
        let cfg = DecayConfig::default();
        let base = (0.5, 0.6);
        assert_eq!(decayed_confidence(base, "bogus", 0, 1_000_000, &cfg), base);
    }

    #[test]
    fn test_clock_skew_returns_base() {
        let cfg = DecayConfig::default();
        let base = (0.5, 0.6);
        // now < created_at → no decay (saturating age = 0).
        assert_eq!(decayed_confidence(base, "task", 1000, 500, &cfg), base);
    }

    #[test]
    fn test_tiers_decay_at_different_rates() {
        let cfg = DecayConfig::default();
        let base = (0.8, 0.8);
        let day = 24 * 3600;
        let eph = decayed_confidence(base, "ephemeral", 0, day, &cfg).1;
        let task = decayed_confidence(base, "task", 0, day, &cfg).1;
        let proj = decayed_confidence(base, "project", 0, day, &cfg).1;
        // Faster half-life ⇒ more decay ⇒ lower remaining confidence.
        assert!(eph < task, "ephemeral should decay faster than task");
        assert!(task < proj, "task should decay faster than project");
    }
}
