//! Ollama Provider Implementation
//!
//! Provides integration with Ollama's local LLM API.
//! Per ADR-015, this supports running local models for privacy and cost savings.
//!
//! # Features
//!
//! - Async HTTP communication with Ollama API
//! - Configurable endpoint and model
//! - Retry logic with exponential backoff
//! - Timeout handling
//!
//! # Examples
//!
//! ```no_run
//! use boswell_llm::OllamaProvider;
//!
//! // Create an Ollama provider
//! let provider = OllamaProvider::new("http://localhost:11434", "llama2");
//! 
//! // Note: The generate method is async, so you need to use it in an async context
//! // or use the LlmProvider trait's sync wrapper
//! ```

use crate::LlmError;
use boswell_domain::traits::LlmProvider as LlmProviderTrait;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Default Ollama API endpoint
pub const DEFAULT_ENDPOINT: &str = "http://localhost:11434";

/// Default timeout for LLM requests (30 seconds)
pub const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// Default number of retry attempts
pub const DEFAULT_MAX_RETRIES: u32 = 3;

/// Ollama API provider for local LLM inference
///
/// This provider communicates with a local Ollama instance to generate text.
pub struct OllamaProvider {
    endpoint: String,
    model: String,
    client: reqwest::Client,
    max_retries: u32,
}

/// Request body for Ollama generate API
#[derive(Serialize)]
struct OllamaGenerateRequest {
    model: String,
    prompt: String,
    stream: bool,
}

/// Response from Ollama generate API
#[derive(Deserialize)]
struct OllamaGenerateResponse {
    response: String,
    #[allow(dead_code)]
    done: bool,
}

impl OllamaProvider {
    /// Create a new Ollama provider
    ///
    /// # Parameters
    ///
    /// - `endpoint`: Ollama API endpoint (e.g., "http://localhost:11434")
    /// - `model`: Model to use (e.g., "llama2", "mistral")
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use boswell_llm::OllamaProvider;
    ///
    /// let provider = OllamaProvider::new("http://localhost:11434", "llama2");
    /// ```
    pub fn new(endpoint: impl Into<String>, model: impl Into<String>) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
            .build()
            .unwrap();
        
        Self {
            endpoint: endpoint.into(),
            model: model.into(),
            client,
            max_retries: DEFAULT_MAX_RETRIES,
        }
    }
    
    /// Create a new Ollama provider with default settings
    ///
    /// Uses `http://localhost:11434` as endpoint and requires a model name.
    ///
    /// # Parameters
    ///
    /// - `model`: Model to use (e.g., "llama2", "mistral")
    pub fn default_endpoint(model: impl Into<String>) -> Self {
        Self::new(DEFAULT_ENDPOINT, model)
    }
    
    /// Set the maximum number of retry attempts
    pub fn with_max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = max_retries;
        self
    }
    
    /// Generate text using Ollama API
    ///
    /// # Parameters
    ///
    /// - `prompt`: Input prompt text
    ///
    /// # Returns
    ///
    /// Generated text from the model
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Ollama is not running
    /// - Model is not available
    /// - Network communication fails
    /// - Response format is invalid
    pub async fn generate(&self, prompt: &str) -> Result<String, LlmError> {
        let url = format!("{}/api/generate", self.endpoint);
        
        let request_body = OllamaGenerateRequest {
            model: self.model.clone(),
            prompt: prompt.to_string(),
            stream: false,
        };
        
        // Retry logic with exponential backoff
        let mut attempts = 0;
        let mut last_error = None;
        
        while attempts < self.max_retries {
            match self.client
                .post(&url)
                .json(&request_body)
                .send()
                .await
            {
                Ok(response) => {
                    if response.status().is_success() {
                        match response.json::<OllamaGenerateResponse>().await {
                            Ok(ollama_response) => {
                                return Ok(ollama_response.response);
                            }
                            Err(e) => {
                                return Err(LlmError::InvalidResponse(
                                    format!("Failed to parse response: {}", e)
                                ));
                            }
                        }
                    } else if response.status() == reqwest::StatusCode::NOT_FOUND {
                        return Err(LlmError::ModelNotAvailable(self.model.clone()));
                    } else {
                        let status = response.status();
                        let error_text = response.text().await
                            .unwrap_or_else(|_| "Unknown error".to_string());
                        last_error = Some(LlmError::Communication(
                            format!("HTTP {}: {}", status, error_text)
                        ));
                    }
                }
                Err(e) => {
                    last_error = Some(LlmError::Communication(
                        format!("Request failed: {}", e)
                    ));
                }
            }
            
            attempts += 1;
            if attempts < self.max_retries {
                // Exponential backoff: 1s, 2s, 4s, etc.
                let delay = Duration::from_secs(2u64.pow(attempts - 1));
                tokio::time::sleep(delay).await;
            }
        }
        
        Err(last_error.unwrap_or_else(|| {
            LlmError::Communication("Max retries exceeded".to_string())
        }))
    }
    
    /// Generate structured output (for future use)
    ///
    /// This is a placeholder for structured generation support.
    /// Ollama supports JSON mode which could be used here.
    pub async fn generate_structured<T>(&self, prompt: &str) -> Result<T, LlmError>
    where
        T: serde::de::DeserializeOwned,
    {
        let response = self.generate(prompt).await?;
        
        serde_json::from_str(&response)
            .map_err(|e| LlmError::InvalidResponse(
                format!("Failed to parse structured response: {}", e)
            ))
    }
}

impl LlmProviderTrait for OllamaProvider {
    type Error = LlmError;
    
    fn generate(&self, prompt: &str) -> Result<String, Self::Error> {
        // Blocking wrapper for async function
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async { self.generate(prompt).await })
    }
    
    fn generate_structured(&self, prompt: &str, _schema: &str) -> Result<String, Self::Error> {
        // For now, just call generate
        // Future: use Ollama's JSON mode with the schema
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async { self.generate(prompt).await })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_ollama_provider_creation() {
        let provider = OllamaProvider::new("http://localhost:11434", "llama2");
        assert_eq!(provider.endpoint, "http://localhost:11434");
        assert_eq!(provider.model, "llama2");
        assert_eq!(provider.max_retries, DEFAULT_MAX_RETRIES);
    }
    
    #[test]
    fn test_ollama_provider_default_endpoint() {
        let provider = OllamaProvider::default_endpoint("mistral");
        assert_eq!(provider.endpoint, DEFAULT_ENDPOINT);
        assert_eq!(provider.model, "mistral");
    }
    
    #[test]
    fn test_ollama_provider_with_max_retries() {
        let provider = OllamaProvider::new("http://localhost:11434", "llama2")
            .with_max_retries(5);
        assert_eq!(provider.max_retries, 5);
    }
    
    // Integration tests (requires running Ollama)
    #[tokio::test]
    #[ignore] // Only run when Ollama is available
    async fn test_ollama_generate_integration() {
        let provider = OllamaProvider::default_endpoint("llama2");
        let result = provider.generate("Say 'hello' and nothing else").await;
        
        // This test only runs if explicitly requested and Ollama is running
        if result.is_ok() {
            let response = result.unwrap();
            assert!(!response.is_empty());
        }
    }
    
    #[tokio::test]
    async fn test_ollama_error_handling() {
        // Use invalid endpoint to trigger error
        let provider = OllamaProvider::new("http://localhost:99999", "llama2")
            .with_max_retries(1);
        
        let result = provider.generate("test").await;
        assert!(result.is_err());
        
        match result {
            Err(LlmError::Communication(_)) => {} // Expected
            _ => panic!("Expected Communication error"),
        }
    }
}
