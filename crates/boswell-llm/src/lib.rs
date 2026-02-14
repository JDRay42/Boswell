//! Boswell LLM Provider Layer
//!
//! Pluggable LLM provider implementations per ADR-015.
//!
//! # Architecture
//!
//! This crate provides implementations of the `LlmProvider` trait from `boswell-domain`.
//! It supports multiple LLM backends with a common interface.
//!
//! # Providers
//!
//! - `MockProvider`: Deterministic mock for testing
//! - `OllamaProvider`: Local Ollama API integration
//!
//! # Examples
//!
//! ```
//! use boswell_llm::MockProvider;
//! use boswell_domain::traits::LlmProvider;
//!
//! let provider = MockProvider::new("Hello from LLM!");
//! let result = provider.generate("test prompt").unwrap();
//! assert_eq!(result, "Hello from LLM!");
//! ```

#![warn(missing_docs)]

pub mod ollama;

use boswell_domain::traits::LlmProvider as LlmProviderTrait;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use thiserror::Error;

pub use ollama::OllamaProvider;

/// Errors that can occur during LLM operations
#[derive(Error, Debug)]
pub enum LlmError {
    /// Network or API communication error
    #[error("Communication error: {0}")]
    Communication(String),
    
    /// Invalid response from LLM
    #[error("Invalid response: {0}")]
    InvalidResponse(String),
    
    /// Rate limit exceeded
    #[error("Rate limit exceeded")]
    RateLimitExceeded,
    
    /// Model not available
    #[error("Model not available: {0}")]
    ModelNotAvailable(String),
    
    /// Generic error
    #[error("LLM error: {0}")]
    Other(String),
}

/// Mock LLM provider for deterministic testing
///
/// This provider returns pre-configured responses without making any network calls.
/// It's useful for testing and development.
///
/// # Examples
///
/// ```
/// use boswell_llm::MockProvider;
/// use boswell_domain::traits::LlmProvider;
///
/// // Simple fixed response
/// let provider = MockProvider::new("Fixed response");
/// assert_eq!(provider.generate("any prompt").unwrap(), "Fixed response");
///
/// // Multiple responses
/// let mut provider = MockProvider::default();
/// provider.add_response("prompt1", "response1");
/// provider.add_response("prompt2", "response2");
/// assert_eq!(provider.generate("prompt1").unwrap(), "response1");
/// assert_eq!(provider.generate("prompt2").unwrap(), "response2");
/// ```
#[derive(Debug, Clone)]
pub struct MockProvider {
    default_response: String,
    responses: Arc<Mutex<HashMap<String, String>>>,
    call_count: Arc<Mutex<usize>>,
}

impl MockProvider {
    /// Create a new MockProvider with a fixed response for all prompts
    pub fn new(response: impl Into<String>) -> Self {
        Self {
            default_response: response.into(),
            responses: Arc::new(Mutex::new(HashMap::new())),
            call_count: Arc::new(Mutex::new(0)),
        }
    }
    
    /// Add a specific response for a given prompt
    pub fn add_response(&mut self, prompt: impl Into<String>, response: impl Into<String>) {
        self.responses
            .lock()
            .unwrap()
            .insert(prompt.into(), response.into());
    }
    
    /// Get the number of times generate was called
    pub fn call_count(&self) -> usize {
        *self.call_count.lock().unwrap()
    }
    
    /// Reset the call count
    pub fn reset_call_count(&self) {
        *self.call_count.lock().unwrap() = 0;
    }
    
    /// Configure to return an error for a specific prompt
    pub fn add_error(&mut self, prompt: impl Into<String>) {
        self.responses
            .lock()
            .unwrap()
            .insert(prompt.into(), "ERROR".to_string());
    }
}

impl Default for MockProvider {
    fn default() -> Self {
        Self::new("Default mock response")
    }
}

impl LlmProviderTrait for MockProvider {
    type Error = LlmError;
    
    fn generate(&self, prompt: &str) -> Result<String, Self::Error> {
        // Increment call count
        *self.call_count.lock().unwrap() += 1;
        
        // Check if we have a specific response for this prompt
        let responses = self.responses.lock().unwrap();
        if let Some(response) = responses.get(prompt) {
            if response == "ERROR" {
                return Err(LlmError::Other("Mock error".to_string()));
            }
            return Ok(response.clone());
        }
        
        // Return default response
        Ok(self.default_response.clone())
    }
    
    fn generate_structured(&self, prompt: &str, _schema: &str) -> Result<String, Self::Error> {
        // For now, structured generation uses the same logic as regular generation
        // In a real implementation, this would validate against the schema
        self.generate(prompt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_mock_provider_default() {
        let provider = MockProvider::new("Test response");
        let result = provider.generate("any prompt");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "Test response");
    }
    
    #[test]
    fn test_mock_provider_specific_responses() {
        let mut provider = MockProvider::default();
        provider.add_response("hello", "world");
        provider.add_response("foo", "bar");
        
        assert_eq!(provider.generate("hello").unwrap(), "world");
        assert_eq!(provider.generate("foo").unwrap(), "bar");
        assert_eq!(provider.generate("unknown").unwrap(), "Default mock response");
    }
    
    #[test]
    fn test_mock_provider_call_count() {
        let provider = MockProvider::new("test");
        
        assert_eq!(provider.call_count(), 0);
        
        provider.generate("prompt1").unwrap();
        assert_eq!(provider.call_count(), 1);
        
        provider.generate("prompt2").unwrap();
        assert_eq!(provider.call_count(), 2);
        
        provider.reset_call_count();
        assert_eq!(provider.call_count(), 0);
    }
    
    #[test]
    fn test_mock_provider_error() {
        let mut provider = MockProvider::default();
        provider.add_error("bad prompt");
        
        let result = provider.generate("bad prompt");
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), LlmError::Other(_)));
    }
    
    #[test]
    fn test_mock_provider_structured() {
        let provider = MockProvider::new("structured response");
        let result = provider.generate_structured("prompt", "schema");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "structured response");
    }
    
    #[test]
    fn test_mock_provider_clone() {
        let provider1 = MockProvider::new("test");
        let provider2 = provider1.clone();
        
        provider1.generate("test").unwrap();
        
        // Both should share the same call count due to Arc
        assert_eq!(provider1.call_count(), 1);
        assert_eq!(provider2.call_count(), 1);
    }
}
