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
//! - `OpenAiCompatProvider`: OpenAI, OpenRouter, DeepSeek, and anything else
//!   speaking OpenAI's chat completions format
//! - `AnthropicProvider`: Anthropic's Messages API
//! - `GeminiProvider`: Google's Generative Language API
//!
//! # What the hosted providers do not do yet
//!
//! `generate_structured` ignores its `schema` argument on every provider in
//! this crate, hosted ones included: it calls `generate` and returns the text.
//! Each vendor constrains decoding differently — `response_format`,
//! `output_config.format`, `responseSchema` — and the trait says nothing about
//! what a `schema` string contains, so honoring it would mean inventing three
//! incompatible contracts. The Extractor does not call it. Tracked as an open
//! slice on the roadmap.
//!
//! Streaming, tool use and multi-turn conversation are likewise absent. The
//! trait is one prompt in, one string out, and these providers implement
//! exactly that.
//!
//! # Keys
//!
//! Every hosted provider takes its key as a constructor argument, with a
//! `*_from_env` alternative that reads the vendor's conventional variable.
//! None of them derive `Debug`; each writes its own that redacts the key,
//! because provider structs end up inside error and tracing output.
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

pub mod anthropic;
pub mod gemini;
pub mod ollama;
pub mod openai_compat;

mod retry;
mod runtime;

use boswell_domain::traits::LlmProvider as LlmProviderTrait;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use thiserror::Error;

pub use anthropic::AnthropicProvider;
pub use gemini::GeminiProvider;
pub use ollama::OllamaProvider;
pub use openai_compat::OpenAiCompatProvider;

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

    /// The provider rejected the credential
    #[error("Authentication failed: {0}")]
    Authentication(String),

    /// The model declined to answer
    ///
    /// Distinct from an error: the call succeeded and the provider chose to
    /// return nothing. Anthropic reports this as `stop_reason: "refusal"` and
    /// Google as a block reason, both under an HTTP 200, so without this
    /// variant a decline reads as an empty answer.
    #[error("Model declined the request: {0}")]
    Refusal(String),

    /// Generic error
    #[error("LLM error: {0}")]
    Other(String),
}

/// Read an API key from the environment.
///
/// # Errors
///
/// [`LlmError::Authentication`] if `variable` is unset, or set to a value that
/// is empty once trimmed — an exported-but-blank variable is a likelier
/// mistake than a deliberate empty key, and failing here beats failing at the
/// far end with a 401.
pub(crate) fn key_from_env(variable: &str) -> Result<String, LlmError> {
    match std::env::var(variable) {
        Ok(key) if !key.trim().is_empty() => Ok(key),
        Ok(_) => Err(LlmError::Authentication(format!("{} is empty", variable))),
        Err(_) => Err(LlmError::Authentication(format!("{} is not set", variable))),
    }
}

/// Decide whether an HTTP failure is worth another attempt.
///
/// Shared by the hosted providers, which all draw the same line: a throttle or
/// a server fault may pass, a rejected key or an unknown model will not.
pub(crate) fn classify_http_failure(
    status: reqwest::StatusCode,
    body: &str,
    model: &str,
) -> retry::Attempt {
    use reqwest::StatusCode;

    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            retry::Attempt::Fatal(LlmError::Authentication(format!("HTTP {}", status)))
        }
        StatusCode::NOT_FOUND => {
            retry::Attempt::Fatal(LlmError::ModelNotAvailable(model.to_string()))
        }
        StatusCode::TOO_MANY_REQUESTS => retry::Attempt::Retry(LlmError::RateLimitExceeded),
        // 408 and 409 are transient by definition; the rest of 4xx is the
        // caller's mistake and will fail identically on every attempt.
        StatusCode::REQUEST_TIMEOUT | StatusCode::CONFLICT => retry::Attempt::Retry(
            LlmError::Communication(format!("HTTP {}: {}", status, body)),
        ),
        _ if status.is_server_error() => retry::Attempt::Retry(LlmError::Communication(format!(
            "HTTP {}: {}",
            status, body
        ))),
        _ => retry::Attempt::Fatal(LlmError::Communication(format!(
            "HTTP {}: {}",
            status, body
        ))),
    }
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
        assert_eq!(
            provider.generate("unknown").unwrap(),
            "Default mock response"
        );
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
