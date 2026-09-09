//! Google Gemini provider, against the Generative Language API.
//!
//! A third wire format. The model ID is part of the URL path rather than the
//! body, the key travels in `x-goog-api-key`, the prompt is nested two levels
//! deep in `contents[].parts[].text`, and a blocked prompt comes back as an
//! HTTP 200 with no candidates at all.
//!
//! The key goes in a header rather than the `?key=` query parameter the
//! quickstarts use. Query strings end up in proxy logs and crash reports;
//! headers do not.
//!
//! # Examples
//!
//! ```no_run
//! use boswell_llm::GeminiProvider;
//!
//! let provider = GeminiProvider::new("AIza...", "gemini-3.7-flash");
//! ```

use crate::retry::{with_retries, Attempt};
use crate::{runtime, LlmError};
use boswell_domain::traits::LlmProvider as LlmProviderTrait;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::Duration;

/// Base URL for the Generative Language API.
pub const GEMINI_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";

/// Environment variable holding a Gemini key.
pub const GEMINI_KEY_VAR: &str = "GEMINI_API_KEY";

/// Default timeout (120 seconds).
pub const DEFAULT_TIMEOUT_SECS: u64 = 120;

/// Default number of attempts.
pub const DEFAULT_MAX_RETRIES: u32 = 3;

/// A provider for Google's Gemini models.
pub struct GeminiProvider {
    base_url: String,
    api_key: String,
    model: String,
    client: reqwest::Client,
    max_retries: u32,
}

/// Hand-written so the API key cannot reach a log through `{:?}`.
impl fmt::Debug for GeminiProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GeminiProvider")
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("max_retries", &self.max_retries)
            .field("api_key", &"<redacted>")
            .finish()
    }
}

#[derive(Serialize)]
struct GenerateContentRequest {
    contents: Vec<Content>,
}

#[derive(Serialize)]
struct Content {
    role: String,
    parts: Vec<Part>,
}

#[derive(Serialize)]
struct Part {
    text: String,
}

#[derive(Deserialize)]
struct GenerateContentResponse {
    #[serde(default)]
    candidates: Vec<Candidate>,
    #[serde(default, rename = "promptFeedback")]
    prompt_feedback: Option<PromptFeedback>,
}

#[derive(Deserialize)]
struct Candidate {
    #[serde(default)]
    content: Option<ResponseContent>,
    #[serde(default, rename = "finishReason")]
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct ResponseContent {
    #[serde(default)]
    parts: Vec<ResponsePart>,
}

#[derive(Deserialize)]
struct ResponsePart {
    #[serde(default)]
    text: Option<String>,
}

#[derive(Deserialize)]
struct PromptFeedback {
    #[serde(default, rename = "blockReason")]
    block_reason: Option<String>,
}

impl GeminiProvider {
    /// Create a provider.
    ///
    /// `model` is an exact model ID, such as `gemini-3.7-flash`. Google's
    /// model line moves quickly; there is deliberately no default, because a
    /// default here would be a model name that quietly goes stale.
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
            .build()
            .expect("failed to build the HTTP client");

        Self {
            base_url: GEMINI_BASE_URL.to_string(),
            api_key: api_key.into(),
            model: model.into(),
            client,
            max_retries: DEFAULT_MAX_RETRIES,
        }
    }

    /// Create a provider, reading `GEMINI_API_KEY`.
    ///
    /// # Errors
    ///
    /// [`LlmError::Authentication`] if the variable is unset or empty.
    pub fn from_env(model: impl Into<String>) -> Result<Self, LlmError> {
        Ok(Self::new(crate::key_from_env(GEMINI_KEY_VAR)?, model))
    }

    /// Point the provider at a different base URL — a gateway or a proxy.
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into().trim_end_matches('/').to_string();
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
    /// - [`LlmError::Refusal`] if the prompt was blocked or the candidate
    ///   stopped on a safety finish reason
    /// - [`LlmError::Communication`] on network failure or a server error
    /// - [`LlmError::InvalidResponse`] if the body does not parse or carries
    ///   no text
    pub async fn generate(&self, prompt: &str) -> Result<String, LlmError> {
        let url = format!("{}/models/{}:generateContent", self.base_url, self.model);

        let request_body = GenerateContentRequest {
            contents: vec![Content {
                role: "user".to_string(),
                parts: vec![Part {
                    text: prompt.to_string(),
                }],
            }],
        };

        let url = &url;
        let body = &request_body;

        with_retries(self.max_retries, move || async move {
            let response = match self
                .client
                .post(url)
                .header("x-goog-api-key", &self.api_key)
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

            let parsed = match response.json::<GenerateContentResponse>().await {
                Ok(parsed) => parsed,
                Err(e) => {
                    return Attempt::Fatal(LlmError::InvalidResponse(format!(
                        "Failed to parse response: {}",
                        e
                    )))
                }
            };

            // A blocked prompt is an HTTP 200 with an empty candidate list.
            // Without this check it would surface as "no text", which sends
            // the operator looking for a parsing bug.
            if let Some(reason) = parsed
                .prompt_feedback
                .as_ref()
                .and_then(|feedback| feedback.block_reason.as_deref())
            {
                return Attempt::Fatal(LlmError::Refusal(format!("prompt blocked: {}", reason)));
            }

            let candidate = match parsed.candidates.into_iter().next() {
                Some(candidate) => candidate,
                None => {
                    return Attempt::Fatal(LlmError::InvalidResponse(
                        "Response carried no candidates".to_string(),
                    ))
                }
            };

            let text: String = candidate
                .content
                .map(|content| {
                    content
                        .parts
                        .iter()
                        .filter_map(|part| part.text.as_deref())
                        .collect()
                })
                .unwrap_or_default();

            if text.is_empty() {
                // An empty answer with a non-STOP finish reason is a refusal
                // or a truncation, not a parsing problem.
                return match candidate.finish_reason.as_deref() {
                    Some(reason) if reason != "STOP" => {
                        Attempt::Fatal(LlmError::Refusal(format!("generation stopped: {}", reason)))
                    }
                    _ => Attempt::Fatal(LlmError::InvalidResponse(
                        "Response carried no text".to_string(),
                    )),
                };
            }

            Attempt::Done(text)
        })
        .await
    }
}

impl LlmProviderTrait for GeminiProvider {
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
        let provider = GeminiProvider::new("k", "gemini-3.7-flash");
        assert_eq!(provider.base_url, GEMINI_BASE_URL);
        assert_eq!(provider.model, "gemini-3.7-flash");
        assert_eq!(provider.max_retries, DEFAULT_MAX_RETRIES);
    }

    #[test]
    fn a_trailing_slash_does_not_become_a_double_slash() {
        let provider = GeminiProvider::new("k", "m").with_base_url("https://example.test/v1beta/");
        assert_eq!(provider.base_url, "https://example.test/v1beta");
    }

    #[test]
    fn debug_output_does_not_carry_the_key() {
        let rendered = format!("{:?}", GeminiProvider::new("AIza-secret", "m"));
        assert!(!rendered.contains("AIza-secret"), "got: {}", rendered);
        assert!(rendered.contains("<redacted>"));
    }

    #[test]
    fn a_blocked_prompt_parses_as_feedback_not_as_a_candidate() {
        let parsed: GenerateContentResponse =
            serde_json::from_str(r#"{"promptFeedback":{"blockReason":"SAFETY"}}"#).unwrap();

        assert!(parsed.candidates.is_empty());
        assert_eq!(
            parsed
                .prompt_feedback
                .and_then(|feedback| feedback.block_reason)
                .as_deref(),
            Some("SAFETY")
        );
    }

    #[tokio::test]
    async fn an_unreachable_endpoint_reports_a_communication_error() {
        let provider = GeminiProvider::new("k", "m")
            .with_base_url("http://127.0.0.1:1/v1beta")
            .with_max_retries(1);

        let result = provider.generate("test").await;
        assert!(matches!(result, Err(LlmError::Communication(_))));
    }
}
