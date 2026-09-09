//! Anthropic Messages API provider.
//!
//! Not an OpenAI-compatible endpoint, which is why this is its own module:
//! the key travels in `x-api-key` rather than a bearer token, the API is
//! versioned by a request header, `max_tokens` is mandatory, and the response
//! body is a list of typed content blocks rather than a single string.
//!
//! # Examples
//!
//! ```no_run
//! use boswell_llm::AnthropicProvider;
//!
//! let provider = AnthropicProvider::new("sk-ant-...", "claude-opus-5");
//! ```

use crate::retry::{with_retries, Attempt};
use crate::{runtime, LlmError};
use boswell_domain::traits::LlmProvider as LlmProviderTrait;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::Duration;

/// The Messages API endpoint.
pub const ANTHROPIC_MESSAGES_URL: &str = "https://api.anthropic.com/v1/messages";

/// The API version this provider is written against.
///
/// Anthropic versions the API by request header, not by URL path. Pinning it
/// here is what stops a server-side change from silently altering the response
/// shape this module parses.
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Environment variable holding an Anthropic key.
pub const ANTHROPIC_KEY_VAR: &str = "ANTHROPIC_API_KEY";

/// Default cap on generated tokens.
///
/// The API requires the field, so there is no "let the server decide" option.
/// This is sized for a non-streaming request: large enough that an extraction
/// answer is not truncated mid-JSON, small enough to stay under the client
/// timeout.
pub const DEFAULT_MAX_TOKENS: u32 = 16_000;

/// Default timeout (120 seconds).
pub const DEFAULT_TIMEOUT_SECS: u64 = 120;

/// Default number of attempts.
pub const DEFAULT_MAX_RETRIES: u32 = 3;

/// A provider for Anthropic's Messages API.
pub struct AnthropicProvider {
    endpoint: String,
    api_key: String,
    model: String,
    max_tokens: u32,
    client: reqwest::Client,
    max_retries: u32,
}

/// Hand-written so the API key cannot reach a log through `{:?}`.
impl fmt::Debug for AnthropicProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnthropicProvider")
            .field("endpoint", &self.endpoint)
            .field("model", &self.model)
            .field("max_tokens", &self.max_tokens)
            .field("max_retries", &self.max_retries)
            .field("api_key", &"<redacted>")
            .finish()
    }
}

#[derive(Serialize)]
struct MessagesRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<RequestMessage>,
}

#[derive(Serialize)]
struct RequestMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct MessagesResponse {
    #[serde(default)]
    content: Vec<ContentBlock>,
    #[serde(default)]
    stop_reason: Option<String>,
}

/// Only text blocks carry an answer. Thinking blocks and tool-use blocks are
/// other variants of the same array, so the `type` tag has to be read rather
/// than assumed.
#[derive(Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    block_type: String,
    #[serde(default)]
    text: Option<String>,
}

impl AnthropicProvider {
    /// Create a provider.
    ///
    /// `model` is an exact model ID, such as `claude-opus-5`. Model IDs are
    /// not derivable — never append a date suffix to one.
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
            .build()
            .expect("failed to build the HTTP client");

        Self {
            endpoint: ANTHROPIC_MESSAGES_URL.to_string(),
            api_key: api_key.into(),
            model: model.into(),
            max_tokens: DEFAULT_MAX_TOKENS,
            client,
            max_retries: DEFAULT_MAX_RETRIES,
        }
    }

    /// Create a provider, reading `ANTHROPIC_API_KEY`.
    ///
    /// # Errors
    ///
    /// [`LlmError::Authentication`] if the variable is unset or empty.
    pub fn from_env(model: impl Into<String>) -> Result<Self, LlmError> {
        Ok(Self::new(crate::key_from_env(ANTHROPIC_KEY_VAR)?, model))
    }

    /// Point the provider at a different endpoint — a gateway or a proxy.
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = endpoint.into();
        self
    }

    /// Set the cap on generated tokens.
    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = max_tokens;
        self
    }

    /// Set the maximum number of attempts.
    pub fn with_max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = max_retries;
        self
    }

    /// Generate text.
    ///
    /// # Errors
    ///
    /// - [`LlmError::Authentication`] if the key is rejected
    /// - [`LlmError::ModelNotAvailable`] if the model ID is unknown
    /// - [`LlmError::RateLimitExceeded`] if throttled through every attempt
    /// - [`LlmError::Refusal`] if a safety classifier declined the request,
    ///   which arrives as an HTTP 200 and would otherwise read as empty output
    /// - [`LlmError::Communication`] on network failure or a server error
    /// - [`LlmError::InvalidResponse`] if the body does not parse or carries
    ///   no text block
    pub async fn generate(&self, prompt: &str) -> Result<String, LlmError> {
        let request_body = MessagesRequest {
            model: self.model.clone(),
            max_tokens: self.max_tokens,
            messages: vec![RequestMessage {
                role: "user".to_string(),
                content: prompt.to_string(),
            }],
        };

        let endpoint = &self.endpoint;
        let body = &request_body;

        with_retries(self.max_retries, move || async move {
            let response = match self
                .client
                .post(endpoint)
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", ANTHROPIC_VERSION)
                .json(body)
                .send()
                .await
            {
                Ok(response) => response,
                Err(e) => {
                    return Attempt::Retry(LlmError::Communication(format!(
                        "Request failed: {}",
                        e
                    )))
                }
            };

            let status = response.status();
            if !status.is_success() {
                let error_body = response
                    .text()
                    .await
                    .unwrap_or_else(|_| "Unknown error".to_string());
                return crate::classify_http_failure(status, &error_body, &self.model);
            }

            let parsed = match response.json::<MessagesResponse>().await {
                Ok(parsed) => parsed,
                Err(e) => {
                    return Attempt::Fatal(LlmError::InvalidResponse(format!(
                        "Failed to parse response: {}",
                        e
                    )))
                }
            };

            // A refusal is a successful HTTP call that produced no answer.
            // Reading `content` first would return an empty string and let a
            // declined request pass for a model that had nothing to say.
            if parsed.stop_reason.as_deref() == Some("refusal") {
                return Attempt::Fatal(LlmError::Refusal(format!(
                    "{} declined the request",
                    self.model
                )));
            }

            let text: String = parsed
                .content
                .iter()
                .filter(|block| block.block_type == "text")
                .filter_map(|block| block.text.as_deref())
                .collect();

            if text.is_empty() {
                return Attempt::Fatal(LlmError::InvalidResponse(
                    "Response carried no text block".to_string(),
                ));
            }

            Attempt::Done(text)
        })
        .await
    }
}

impl LlmProviderTrait for AnthropicProvider {
    type Error = LlmError;

    fn generate(&self, prompt: &str) -> Result<String, Self::Error> {
        runtime::block_on(self.generate(prompt))
    }

    /// Schema-constrained decoding is not wired up; see the crate docs.
    fn generate_structured(&self, prompt: &str, _schema: &str) -> Result<String, Self::Error> {
        runtime::block_on(self.generate(prompt))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_the_documented_ones() {
        let provider = AnthropicProvider::new("k", "claude-opus-5");
        assert_eq!(provider.endpoint, ANTHROPIC_MESSAGES_URL);
        assert_eq!(provider.model, "claude-opus-5");
        assert_eq!(provider.max_tokens, DEFAULT_MAX_TOKENS);
        assert_eq!(provider.max_retries, DEFAULT_MAX_RETRIES);
    }

    #[test]
    fn builders_override_the_defaults() {
        let provider = AnthropicProvider::new("k", "claude-opus-5")
            .with_endpoint("https://gateway.test/v1/messages")
            .with_max_tokens(1024)
            .with_max_retries(1);

        assert_eq!(provider.endpoint, "https://gateway.test/v1/messages");
        assert_eq!(provider.max_tokens, 1024);
        assert_eq!(provider.max_retries, 1);
    }

    #[test]
    fn debug_output_does_not_carry_the_key() {
        let rendered = format!("{:?}", AnthropicProvider::new("sk-ant-secret", "m"));
        assert!(!rendered.contains("sk-ant-secret"), "got: {}", rendered);
        assert!(rendered.contains("<redacted>"));
    }

    #[test]
    fn only_text_blocks_contribute_to_the_answer() {
        let parsed: MessagesResponse = serde_json::from_str(
            r#"{"content":[
                 {"type":"thinking","thinking":"ignored"},
                 {"type":"text","text":"first"},
                 {"type":"text","text":" second"}
               ],"stop_reason":"end_turn"}"#,
        )
        .unwrap();

        let text: String = parsed
            .content
            .iter()
            .filter(|block| block.block_type == "text")
            .filter_map(|block| block.text.as_deref())
            .collect();

        assert_eq!(text, "first second");
    }

    #[tokio::test]
    async fn an_unreachable_endpoint_reports_a_communication_error() {
        let provider = AnthropicProvider::new("k", "claude-opus-5")
            .with_endpoint("http://127.0.0.1:1/v1/messages")
            .with_max_retries(1);

        let result = provider.generate("test").await;
        assert!(matches!(result, Err(LlmError::Communication(_))));
    }
}
