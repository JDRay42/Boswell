//! Instance registry for tracking gRPC service instances.
//!
//! Phase 2 implementation: single-instance mode with basic health tracking.
//! Multi-instance federation deferred to Phase 3.

use crate::config::InstanceConfig;
use crate::session::InstanceInfo;
use std::sync::{Arc, RwLock};
use thiserror::Error;

/// Registry error
#[derive(Debug, Error)]
pub enum RegistryError {
    /// Instance not found
    #[error("Instance not found: {0}")]
    InstanceNotFound(String),

    /// No healthy instances available
    #[error("No healthy instances available")]
    NoHealthyInstances,
}

/// Health status of an instance
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthStatus {
    /// Instance is healthy and accepting requests
    Healthy,
    /// Instance is degraded but functional
    Degraded,
    /// Instance is unhealthy
    Unhealthy,
}

impl HealthStatus {
    /// Convert to string representation
    pub fn as_str(&self) -> &'static str {
        match self {
            HealthStatus::Healthy => "healthy",
            HealthStatus::Degraded => "degraded",
            HealthStatus::Unhealthy => "unhealthy",
        }
    }
}

/// Registered instance information
#[derive(Debug, Clone)]
struct RegisteredInstance {
    id: String,
    endpoint: String,
    expertise: Vec<String>,
    health: HealthStatus,
}

/// Instance registry for managing available service instances
pub struct InstanceRegistry {
    instances: Arc<RwLock<Vec<RegisteredInstance>>>,
}

impl InstanceRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            instances: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Create a registry from configuration
    pub fn from_config(configs: Vec<InstanceConfig>) -> Self {
        let instances = configs
            .into_iter()
            .map(|config| RegisteredInstance {
                id: config.id,
                endpoint: config.endpoint,
                expertise: config.expertise,
                health: HealthStatus::Healthy, // Assume healthy on startup
            })
            .collect();

        Self {
            instances: Arc::new(RwLock::new(instances)),
        }
    }

    /// Register a new instance
    pub fn register(&self, id: String, endpoint: String, expertise: Vec<String>) {
        let mut instances = self.instances.write().unwrap();

        // Remove existing instance with same ID
        instances.retain(|inst| inst.id != id);

        // Add new instance
        instances.push(RegisteredInstance {
            id,
            endpoint,
            expertise,
            health: HealthStatus::Healthy,
        });
    }

    /// Get all instances as InstanceInfo (for session responses)
    pub fn get_all_instances(&self) -> Vec<InstanceInfo> {
        let instances = self.instances.read().unwrap();
        instances
            .iter()
            .map(|inst| InstanceInfo {
                id: inst.id.clone(),
                endpoint: inst.endpoint.clone(),
                expertise: inst.expertise.clone(),
                health: inst.health.as_str().to_string(),
            })
            .collect()
    }

    /// Get healthy instances only
    pub fn get_healthy_instances(&self) -> Vec<InstanceInfo> {
        let instances = self.instances.read().unwrap();
        instances
            .iter()
            .filter(|inst| inst.health == HealthStatus::Healthy)
            .map(|inst| InstanceInfo {
                id: inst.id.clone(),
                endpoint: inst.endpoint.clone(),
                expertise: inst.expertise.clone(),
                health: inst.health.as_str().to_string(),
            })
            .collect()
    }

    /// Update health status of an instance
    pub fn update_health(&self, id: &str, health: HealthStatus) -> Result<(), RegistryError> {
        let mut instances = self.instances.write().unwrap();
        let instance = instances
            .iter_mut()
            .find(|inst| inst.id == id)
            .ok_or_else(|| RegistryError::InstanceNotFound(id.to_string()))?;

        instance.health = health;
        Ok(())
    }

    /// Check if registry has any healthy instances
    pub fn has_healthy_instances(&self) -> bool {
        let instances = self.instances.read().unwrap();
        instances.iter().any(|inst| inst.health == HealthStatus::Healthy)
    }

    /// Get instance count
    pub fn instance_count(&self) -> usize {
        self.instances.read().unwrap().len()
    }
}

impl Default for InstanceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_registry() {
        let registry = InstanceRegistry::new();
        assert_eq!(registry.instance_count(), 0);
        assert!(!registry.has_healthy_instances());
    }

    #[test]
    fn test_register_instance() {
        let registry = InstanceRegistry::new();
        registry.register(
            "test-instance".to_string(),
            "http://localhost:50051".to_string(),
            vec!["*".to_string()],
        );

        assert_eq!(registry.instance_count(), 1);
        assert!(registry.has_healthy_instances());

        let instances = registry.get_all_instances();
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].id, "test-instance");
        assert_eq!(instances[0].endpoint, "http://localhost:50051");
    }

    #[test]
    fn test_register_duplicate_id() {
        let registry = InstanceRegistry::new();
        registry.register(
            "test".to_string(),
            "http://localhost:50051".to_string(),
            vec![],
        );
        registry.register(
            "test".to_string(),
            "http://localhost:50052".to_string(),
            vec![],
        );

        // Should only have one instance (duplicate replaced)
        assert_eq!(registry.instance_count(), 1);
        let instances = registry.get_all_instances();
        assert_eq!(instances[0].endpoint, "http://localhost:50052");
    }

    #[test]
    fn test_from_config() {
        let configs = vec![
            InstanceConfig {
                id: "instance1".to_string(),
                endpoint: "http://localhost:50051".to_string(),
                expertise: vec!["domain1".to_string()],
            },
            InstanceConfig {
                id: "instance2".to_string(),
                endpoint: "http://localhost:50052".to_string(),
                expertise: vec!["domain2".to_string()],
            },
        ];

        let registry = InstanceRegistry::from_config(configs);
        assert_eq!(registry.instance_count(), 2);
        assert!(registry.has_healthy_instances());
    }

    #[test]
    fn test_update_health() {
        let registry = InstanceRegistry::new();
        registry.register(
            "test".to_string(),
            "http://localhost:50051".to_string(),
            vec![],
        );

        registry.update_health("test", HealthStatus::Degraded).unwrap();
        let instances = registry.get_all_instances();
        assert_eq!(instances[0].health, "degraded");

        registry.update_health("test", HealthStatus::Unhealthy).unwrap();
        let instances = registry.get_all_instances();
        assert_eq!(instances[0].health, "unhealthy");
    }

    #[test]
    fn test_update_health_not_found() {
        let registry = InstanceRegistry::new();
        let result = registry.update_health("nonexistent", HealthStatus::Healthy);
        assert!(matches!(result, Err(RegistryError::InstanceNotFound(_))));
    }

    #[test]
    fn test_get_healthy_instances() {
        let registry = InstanceRegistry::new();
        registry.register(
            "healthy".to_string(),
            "http://localhost:50051".to_string(),
            vec![],
        );
        registry.register(
            "unhealthy".to_string(),
            "http://localhost:50052".to_string(),
            vec![],
        );

        registry.update_health("unhealthy", HealthStatus::Unhealthy).unwrap();

        let healthy = registry.get_healthy_instances();
        assert_eq!(healthy.len(), 1);
        assert_eq!(healthy[0].id, "healthy");
    }
}
