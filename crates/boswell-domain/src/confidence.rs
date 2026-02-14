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
        assert!(lower >= 0.0 && lower <= 1.0, "Lower bound must be in [0, 1]");
        assert!(upper >= 0.0 && upper <= 1.0, "Upper bound must be in [0, 1]");
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
