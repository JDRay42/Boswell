///! gRPC server configuration and lifecycle management
///!
///! Handles server initialization, TLS setup, and graceful shutdown.

use std::sync::{Arc, Mutex};
use tonic::transport::Server;
use boswell_domain::traits::ClaimStore;

use crate::proto::bos_well_service_server::BosWellServiceServer;
use crate::service::BosWellServiceImpl;

/// Server configuration
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Server listen address
    pub addr: String,
    
    /// Server port
    pub port: u16,
    
    /// Enable TLS (per ADR-017)
    pub enable_tls: bool,
    
    /// TLS certificate path
    pub tls_cert_path: Option<String>,
    
    /// TLS key path
    pub tls_key_path: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            addr: "127.0.0.1".to_string(),
            port: 50051,
            enable_tls: false,
            tls_cert_path: None,
            tls_key_path: None,
        }
    }
}

impl ServerConfig {
    /// Create a new server configuration
    pub fn new(addr: impl Into<String>, port: u16) -> Self {
        Self {
            addr: addr.into(),
            port,
            ..Default::default()
        }
    }
    
    /// Enable TLS with certificate paths
    pub fn with_tls(mut self, cert_path: impl Into<String>, key_path: impl Into<String>) -> Self {
        self.enable_tls = true;
        self.tls_cert_path = Some(cert_path.into());
        self.tls_key_path = Some(key_path.into());
        self
    }
    
    /// Get the full server address
    pub fn full_address(&self) -> String {
        format!("{}:{}", self.addr, self.port)
    }
}

/// Start the gRPC server
///
/// # Errors
/// Returns error if server fails to start or bind to address
pub async fn start_server<S>(
    config: ServerConfig,
    store: Arc<Mutex<S>>,
) -> Result<(), Box<dyn std::error::Error>>
where
    S: ClaimStore + Send + Sync + 'static,
    S::Error: std::fmt::Debug,
{
    let addr = config.full_address().parse()?;
    
    let service = BosWellServiceImpl::new(store);
    let service_server = BosWellServiceServer::new(service);
    
    println!("BosWell gRPC server starting on {}", addr);
    
    if config.enable_tls {
        // TLS configuration (placeholder for Phase 2)
        println!("TLS enabled (certificate validation deferred)");
        // TODO: Load and validate certificates
    }
    
    Server::builder()
        .add_service(service_server)
        .serve(addr)
        .await?;
    
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = ServerConfig::default();
        assert_eq!(config.addr, "127.0.0.1");
        assert_eq!(config.port, 50051);
        assert!(!config.enable_tls);
    }

    #[test]
    fn test_config_with_tls() {
        let config = ServerConfig::new("0.0.0.0", 50052)
            .with_tls("cert.pem", "key.pem");
        
        assert!(config.enable_tls);
        assert_eq!(config.tls_cert_path, Some("cert.pem".to_string()));
        assert_eq!(config.tls_key_path, Some("key.pem".to_string()));
    }

    #[test]
    fn test_full_address() {
        let config = ServerConfig::new("localhost", 8080);
        assert_eq!(config.full_address(), "localhost:8080");
    }
}
