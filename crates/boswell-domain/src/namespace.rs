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
