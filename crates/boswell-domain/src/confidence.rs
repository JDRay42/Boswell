//! Confidence interval module (per ADR-003)

/// Confidence interval representing [lower, upper] bounds
/// 
/// Per ADR-003, we use intervals to capture both:
/// - The confidence level itself
/// - Certainty about that confidence level
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ConfidenceInterval {
    /// Lower bound [0.0, 1.0]
    pub lower: f64,
    /// Upper bound [0.0, 1.0]
    pub upper: f64,
}

impl ConfidenceInterval {
    /// Create a new confidence interval
    /// 
    /// # Panics
    /// Panics if bounds are invalid (lower > upper or out of [0, 1])
    pub fn new(lower: f64, upper: f64) -> Self {
        assert!((0.0..=1.0).contains(&lower), "Lower bound must be in [0, 1]");
        assert!((0.0..=1.0).contains(&upper), "Upper bound must be in [0, 1]");
        assert!(lower <= upper, "Lower bound must be <= upper bound");
        
        Self { lower, upper }
    }

    /// Get the midpoint of the interval
    pub fn midpoint(&self) -> f64 {
        (self.lower + self.upper) / 2.0
    }

    /// Get the width of the interval (uncertainty measure)
    pub fn width(&self) -> f64 {
        self.upper - self.lower
    }

    /// Check if the interval contains a value
    pub fn contains(&self, value: f64) -> bool {
        value >= self.lower && value <= self.upper
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_confidence_interval_creation() {
        let ci = ConfidenceInterval::new(0.7, 0.9);
        assert_eq!(ci.lower, 0.7);
        assert_eq!(ci.upper, 0.9);
    }

    #[test]
    fn test_midpoint() {
        let ci = ConfidenceInterval::new(0.6, 0.8);
        assert_eq!(ci.midpoint(), 0.7);
    }

    #[test]
    fn test_width() {
        let ci = ConfidenceInterval::new(0.5, 0.9);
        assert_eq!(ci.width(), 0.4);
    }

    #[test]
    #[should_panic]
    fn test_invalid_bounds() {
        ConfidenceInterval::new(0.9, 0.5); // Lower > upper
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Property: Confidence interval always satisfies lower <= upper
        #[test]
        fn test_confidence_interval_invariant(lower in 0.0..=1.0, upper in 0.0..=1.0) {
            if lower <= upper {
                let ci = ConfidenceInterval::new(lower, upper);
                prop_assert!(ci.lower <= ci.upper);
            }
        }

        /// Property: Midpoint is always between bounds
        #[test]
        fn test_midpoint_bounds(lower in 0.0..=1.0, upper in 0.0..=1.0) {
            if lower <= upper {
                let ci = ConfidenceInterval::new(lower, upper);
                let mid = ci.midpoint();
                
                prop_assert!(mid >= ci.lower);
                prop_assert!(mid <= ci.upper);
            }
        }

        /// Property: Width is always non-negative and at most 1.0
        #[test]
        fn test_width_bounds(lower in 0.0..=1.0, upper in 0.0..=1.0) {
            if lower <= upper {
                let ci = ConfidenceInterval::new(lower, upper);
                let width = ci.width();
                
                prop_assert!(width >= 0.0);
                prop_assert!(width <= 1.0);
                prop_assert_eq!(width, upper - lower);
            }
        }

        /// Property: Contains should work correctly for boundary values
        #[test]
        fn test_contains_boundaries(lower in 0.0..=1.0, upper in 0.0..=1.0) {
            if lower <= upper {
                let ci = ConfidenceInterval::new(lower, upper);
                
                prop_assert!(ci.contains(lower));
                prop_assert!(ci.contains(upper));
                prop_assert!(ci.contains(ci.midpoint()));
            }
        }
    }
}
