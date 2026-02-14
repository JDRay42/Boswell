//! Tier module - lifecycle stages for claims

/// Tier in the claim lifecycle
/// 
/// Claims progress through tiers with different retention and evaluation criteria:
/// - Ephemeral: Short-lived, task-specific
/// - Task: Medium-term, specific task context
/// - Project: Long-term, project-level knowledge
/// - Permanent: Indefinite, core knowledge
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tier {
    /// Short-lived claims (hours to days)
    Ephemeral,
    
    /// Task-specific claims (days to weeks)
    Task,
    
    /// Project-level claims (weeks to months)
    Project,
    
    /// Core knowledge (indefinite)
    Permanent,
}

impl Tier {
    /// Get the tier name as a string
    pub fn as_str(&self) -> &'static str {
        match self {
            Tier::Ephemeral => "ephemeral",
            Tier::Task => "task",
            Tier::Project => "project",
            Tier::Permanent => "permanent",
        }
    }

    /// Parse a tier from a string (internal use)
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "ephemeral" => Some(Tier::Ephemeral),
            "task" => Some(Tier::Task),
            "project" => Some(Tier::Project),
            "permanent" => Some(Tier::Permanent),
            _ => None,
        }
    }

    /// Get the next tier in the hierarchy (for promotion)
    pub fn next(&self) -> Option<Self> {
        match self {
            Tier::Ephemeral => Some(Tier::Task),
            Tier::Task => Some(Tier::Project),
            Tier::Project => Some(Tier::Permanent),
            Tier::Permanent => None, // Already at top
        }
    }

    /// Get the previous tier in the hierarchy (for demotion)
    pub fn previous(&self) -> Option<Self> {
        match self {
            Tier::Ephemeral => None, // Already at bottom
            Tier::Task => Some(Tier::Ephemeral),
            Tier::Project => Some(Tier::Task),
            Tier::Permanent => Some(Tier::Project),
        }
    }
}

impl std::str::FromStr for Tier {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s).ok_or_else(|| format!("Invalid tier: {}", s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tier_progression() {
        assert_eq!(Tier::Ephemeral.next(), Some(Tier::Task));
        assert_eq!(Tier::Task.next(), Some(Tier::Project));
        assert_eq!(Tier::Project.next(), Some(Tier::Permanent));
        assert_eq!(Tier::Permanent.next(), None);
    }

    #[test]
    fn test_tier_demotion() {
        assert_eq!(Tier::Permanent.previous(), Some(Tier::Project));
        assert_eq!(Tier::Project.previous(), Some(Tier::Task));
        assert_eq!(Tier::Task.previous(), Some(Tier::Ephemeral));
        assert_eq!(Tier::Ephemeral.previous(), None);
    }
}
