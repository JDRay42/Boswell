//! Namespace module (per ADR-006 - convention-based namespaces)

/// Namespace for organizing claims
/// 
/// Uses slash-delimited hierarchy: `project/context/subcontext`
/// Validated by slash count, not tree structures.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Namespace(String);

impl Namespace {
    /// Create a new namespace
    /// 
    /// # Errors
    /// Returns error if namespace format is invalid
    pub fn new(value: String) -> Result<Self, String> {
        // Basic validation - extend based on requirements
        if value.is_empty() {
            return Err("Namespace cannot be empty".to_string());
        }
        
        // TODO: Add depth validation when max depth is configured
        
        Ok(Self(value))
    }

    /// Get namespace as string
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Get depth (number of slash-separated components)
    pub fn depth(&self) -> usize {
        self.0.split('/').count()
    }

    /// Check if this namespace is a parent of another
    pub fn is_parent_of(&self, other: &Namespace) -> bool {
        other.0.starts_with(&self.0) && other.0.len() > self.0.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_namespace_creation() {
        let ns = Namespace::new("project/task".to_string()).unwrap();
        assert_eq!(ns.as_str(), "project/task");
    }

    #[test]
    fn test_namespace_depth() {
        let ns = Namespace::new("a/b/c".to_string()).unwrap();
        assert_eq!(ns.depth(), 3);
    }

    #[test]
    fn test_parent_relationship() {
        let parent = Namespace::new("project".to_string()).unwrap();
        let child = Namespace::new("project/task".to_string()).unwrap();
        
        assert!(parent.is_parent_of(&child));
        assert!(!child.is_parent_of(&parent));
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Property: Namespace depth matches slash count + 1
        #[test]
        fn test_namespace_depth_property(s in "[a-z]{1,10}(/[a-z]{1,10}){0,5}") {
            let ns = Namespace::new(s.clone()).unwrap();
            let expected_depth = s.split('/').count();
            
            prop_assert_eq!(ns.depth(), expected_depth);
        }

        /// Property: Parent relationship is transitive
        #[test]
        fn test_parent_relationship_transitive(
            a in "[a-z]{1,5}",
            b in "[a-z]{1,5}",
            c in "[a-z]{1,5}"
        ) {
            let ns_a = Namespace::new(a.clone()).unwrap();
            let ns_ab = Namespace::new(format!("{}/{}", a, b)).unwrap();
            let ns_abc = Namespace::new(format!("{}/{}/{}", a, b, c)).unwrap();
            
            // If A is parent of AB and AB is parent of ABC, then A is parent of ABC
            prop_assert!(ns_a.is_parent_of(&ns_ab));
            prop_assert!(ns_ab.is_parent_of(&ns_abc));
            prop_assert!(ns_a.is_parent_of(&ns_abc));
        }

        /// Property: Parent relationship is not reflexive
        #[test]
        fn test_parent_relationship_not_reflexive(s in "[a-z]{1,10}(/[a-z]{1,10}){0,3}") {
            let ns = Namespace::new(s).unwrap();
            
            // A namespace is not its own parent
            prop_assert!(!ns.is_parent_of(&ns));
        }

        /// Property: Non-empty namespaces always have depth >= 1
        #[test]
        fn test_namespace_min_depth(s in "[a-z]{1,10}(/[a-z]{1,10}){0,5}") {
            let ns = Namespace::new(s).unwrap();
            
            prop_assert!(ns.depth() >= 1);
        }
    }
}
