//! Claim module - the fundamental unit of Boswell's memory system

use std::fmt;

/// Unique identifier for a claim based on UUIDv7 (per ADR-011)
///
/// UUIDv7 provides:
/// - Chronological sortability for temporal queries
/// - 128-bit uniqueness
/// - RFC 9562-standard format with broad ecosystem support
/// - No coordination required for distributed generation
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClaimId(u128);

impl ClaimId {
    /// Generate a new UUIDv7-based ClaimId
    ///
    /// # Examples
    ///
    /// ```
    /// use boswell_domain::ClaimId;
    ///
    /// let id = ClaimId::new();
    /// assert!(id.value() > 0);
    /// ```
    pub fn new() -> Self {
        Self(uuid::Uuid::now_v7().as_u128())
    }

    /// Create a new ClaimId from a raw u128 value
    ///
    /// This is primarily for storage layer deserialization.
    pub fn from_value(value: u128) -> Self {
        Self(value)
    }

    /// Parse a ClaimId from a UUIDv7 string
    ///
    /// # Examples
    ///
    /// ```
    /// use boswell_domain::ClaimId;
    ///
    /// let id = ClaimId::new();
    /// let id_str = id.to_string();
    /// let parsed = ClaimId::from_string(&id_str).unwrap();
    /// assert_eq!(id, parsed);
    /// ```
    pub fn from_string(s: &str) -> Result<Self, String> {
        uuid::Uuid::parse_str(s)
            .map(|u| Self(u.as_u128()))
            .map_err(|e| format!("Invalid UUIDv7 string: {}", e))
    }

    /// Get the raw u128 value
    pub fn value(&self) -> u128 {
        self.0
    }

    /// Get the timestamp component of the UUIDv7 (milliseconds since Unix epoch)
    pub fn timestamp(&self) -> u64 {
        // UUIDv7: top 48 bits are Unix millisecond timestamp
        (self.0 >> 80) as u64
    }
}

impl Default for ClaimId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ClaimId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", uuid::Uuid::from_u128(self.0))
    }
}

/// A claim - the fundamental unit of knowledge in Boswell
///
/// Per ADR-001, everything is a claim with confidence, not a fact.
/// Claims are immutable once created; updates create new claims.
#[derive(Debug, Clone, PartialEq)]
pub struct Claim {
    /// Unique identifier
    pub id: ClaimId,
    
    /// Namespace for organization (per ADR-006)
    pub namespace: String,
    
    /// Subject of the claim
    pub subject: String,
    
    /// Predicate/relationship
    pub predicate: String,
    
    /// Object of the claim
    pub object: String,
    
    /// Confidence interval [lower, upper] (per ADR-003)
    pub confidence: (f64, f64),
    
    /// Current tier (ephemeral, task, project, permanent)
    pub tier: String,
    
    /// When this claim was created (timestamp)
    pub created_at: u64,
    
    /// When this claim should be considered stale
    pub stale_at: Option<u64>,
}

impl Claim {
    /// Create a new claim
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: ClaimId,
        namespace: String,
        subject: String,
        predicate: String,
        object: String,
        confidence: (f64, f64),
        tier: String,
        created_at: u64,
    ) -> Self {
        Self {
            id,
            namespace,
            subject,
            predicate,
            object,
            confidence,
            tier,
            created_at,
            stale_at: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_claim_id_ordering() {
        let id1 = ClaimId::from_value(1000);
        let id2 = ClaimId::from_value(2000);
        
        assert!(id1 < id2);
        assert!(id2 > id1);
    }

    #[test]
    fn test_claim_id_chronological() {
        // UUIDv7s generated in sequence should be chronologically ordered
        let id1 = ClaimId::new();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let id2 = ClaimId::new();

        assert!(id1 < id2, "Earlier UUIDv7 should be less than later UUIDv7");
        assert!(id1.timestamp() <= id2.timestamp(), "Timestamps should be ordered");
    }

    #[test]
    fn test_claim_id_display_and_parse() {
        let id = ClaimId::new();
        let id_str = id.to_string();

        // UUIDv7 strings are 36 characters (8-4-4-4-12 with hyphens)
        assert_eq!(id_str.len(), 36);

        // Round-trip through string should preserve ID
        let parsed = ClaimId::from_string(&id_str).unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn test_claim_id_invalid_string() {
        assert!(ClaimId::from_string("not-a-valid-uuid").is_err());
        assert!(ClaimId::from_string("").is_err());
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Property: UUIDv7 ordering matches u128 ordering
        #[test]
        fn test_uuid_ordering_property(a: u128, b: u128) {
            let id_a = ClaimId::from_value(a);
            let id_b = ClaimId::from_value(b);

            // Ordering should be consistent with underlying values
            prop_assert_eq!(id_a < id_b, a < b);
            prop_assert_eq!(id_a == id_b, a == b);
            prop_assert_eq!(id_a > id_b, a > b);
        }

        /// Property: Round-trip through string representation preserves ID
        #[test]
        fn test_uuid_string_roundtrip(value: u128) {
            let id = ClaimId::from_value(value);
            let id_str = id.to_string();

            match ClaimId::from_string(&id_str) {
                Ok(parsed) => prop_assert_eq!(id, parsed),
                Err(e) => return Err(TestCaseError::fail(e)),
            }
        }

        /// Property: Generated UUIDv7s have valid timestamps
        #[test]
        fn test_uuid_timestamp_validity(_n in 0..10) {
            let id = ClaimId::new();
            let timestamp = id.timestamp();

            // Timestamp should be reasonable (after 2020, before 2100)
            let min_timestamp = 1577836800000u64; // 2020-01-01
            let max_timestamp = 4102444800000u64; // 2100-01-01

            prop_assert!(timestamp >= min_timestamp && timestamp <= max_timestamp,
                "Timestamp {} out of reasonable range", timestamp);
        }
    }
}
