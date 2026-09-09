//! One provider for every endpoint that speaks OpenAI's chat completions API.
//!
//! OpenAI, OpenRouter and DeepSeek are three vendors, three billing
//! relationships and three sets of model names, but they are one wire format:
//! `POST {base}/chat/completions`, a bearer token, a list of messages in, a
//! list of choices out. Writing three near-identical clients would triple the
//! surface without adding a capability, so this is one client with three named
//! constructors and an open `new` for whatever OpenAI-compatible endpoint
//! comes next.
//!
//! Anthropic and Google do *not* speak this format. They have their own
//! modules ([`crate::anthropic`], [`crate::gemini`]) because their request and
//! response shapes genuinely differ, not because of vendor branding.
//!
//! # Examples
//!
//! ```no_run
//! use boswell_llm::OpenAiCompatProvider;
//!
//! // Named constructors carry the base URL, so the caller supplies only a key
//! // and a model.
//! let deepseek = OpenAiCompatProvider::deepseek("sk-...", "deepseek-chat");
//! let openrouter = OpenAiCompatProvider::openrouter("sk-or-...", "openai/gpt-4.1");
//! let openai = OpenAiCompatProvider::openai("sk-...", "gpt-4.1");
//!
//! // Anything else that speaks the same format:
//! let other = OpenAiCompatProvider::new("https://example.test/v1", "key", "some-model");
//! ```

use crate::retry::{with_retries, Attempt};
use crate::{runtime, LlmError};
use boswell_domain::traits::LlmProvider as LlmProviderTrait;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::Duration;

/// OpenAI's own endpoint.
pub const OPENAI_BASE_URL: &str = "https://api.openai.com/v1";

/// OpenRouter, which fronts many vendors behind this same format.
pub const OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";

/// DeepSeek's OpenAI-compatible endpoint.
pub const DEEPSEEK_BASE_URL: &str = "https://api.deepseek.com/v1";

/// Environment variable holding an OpenAI key.
pub const OPENAI_KEY_VAR: &str = "OPENAI_API_KEY";

/// Environment variable holding an OpenRouter key.
pub const OPENROUTER_KEY_VAR: &str = "OPENROUTER_API_KEY";

/// Environment variable holding a DeepSeek key.
pub const DEEPSEEK_KEY_VAR: &str = "DEEPSEEK_API_KEY";

/// Default timeout for a hosted completion (120 seconds).
///
/// Ten times the local Ollama timeout. A hosted reasoning model on a long
/// prompt routinely spends more than thirty seconds before its first byte.
pub const DEFAULT_TIMEOUT_SECS: u64 = 120;

/// Default number of attempts.
pub const DEFAULT_MAX_RETRIES: u32 = 3;

/// A provider for any endpoint speaking OpenAI's chat completions format.
pub struct OpenAiCompatProvider {
    base_url: String,
    api_key: String,
    model: String,
    max_tokens: Option<u32>,
    client: reqwest::Client,
    max_retries: u32,
}

/// Deliberately hand-written: a derived `Debug` would print the API key, and
/// provider structs end up inside error and tracing output.
impl fmt::Debug for OpenAiCompatProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenAiCompatProvider")
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("max_tokens", &self.max_tokens)
            .field("max_retries", &self.max_retries)
            .field("api_key", &"<redacted>")
            .finish()
    }
}

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}

#[derive(Serialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    #[serde(default)]
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatResponseMessage,
}

#[derive(Deserialize)]
struct ChatResponseMessage {
    /// Absent when the model returned only tool calls, which this provider
    /// does not ask for and cannot use.
    content: Option<String>,
}

impl OpenAiCompatProvider {
    /// Create a provider against any OpenAI-compatible endpoint.
    ///
    /// `base_url` is the prefix that `/chat/completions` hangs off — for
    /// OpenAI that is `https://api.openai.com/v1`, not the bare host. A
    /// trailing slash is trimmed.
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
            .build()
            .expect("failed to build the HTTP client");

        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
            model: model.into(),
            max_tokens: None,
            client,
            max_retries: DEFAULT_MAX_RETRIES,
        }
    }

    /// A provider against OpenAI.
    pub fn openai(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self::new(OPENAI_BASE_URL, api_key, model)
    }

    /// A provider against OpenRouter.
    pub fn openrouter(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self::new(OPENROUTER_BASE_URL, api_key, model)
    }

    /// A provider against DeepSeek.
    pub fn deepseek(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self::new(DEEPSEEK_BASE_URL, api_key, model)
    }

    /// A provider against OpenAI, reading `OPENAI_API_KEY`.
    ///
    /// # Errors
    ///
    /// [`LlmError::Authentication`] if the variable is unset or empty.
    pub fn openai_from_env(model: impl Into<String>) -> Result<Self, LlmError> {
        Ok(Self::openai(crate::key_from_env(OPENAI_KEY_VAR)?, model))
    }

    /// A provider against OpenRouter, reading `OPENROUTER_API_KEY`.
    ///
    /// # Errors
    ///
    /// [`LlmError::Authentication`] if the variable is unset or empty.
    pub fn openrouter_from_env(model: impl Into<String>) -> Result<Self, LlmError> {
        Ok(Self::openrouter(
            crate::key_from_env(OPENROUTER_KEY_VAR)?,
            model,
        ))
    }

    /// A provider against DeepSeek, reading `DEEPSEEK_API_KEY`.
    ///
    /// # Errors
    ///
    /// [`LlmError::Authentication`] if the variable is unset or empty.
    pub fn deepseek_from_env(model: impl Into<String>) -> Result<Self, LlmError> {
        Ok(Self::deepseek(
            crate::key_from_env(DEEPSEEK_KEY_VAR)?,
            model,
        ))
    }

    /// Set the maximum number of attempts.
    pub fn with_max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = max_retries;
        self
    }

    /// Set the request timeout.
    ///
    /// The default suits a hosted endpoint. A local model behind this same
    /// format is a different animal: it may spend a minute loading weights
    /// before it emits a first token, and a reasoning model then generates its
    /// whole chain of thought before the answer.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .expect("failed to build the HTTP client");
        self
    }

    /// Cap the tokens the model may generate.
    ///
    /// Left unset by default, because the field is not portable: OpenAI's
    /// reasoning models reject `max_tokens` in favor of
    /// `max_completion_tokens`, while OpenRouter and DeepSeek accept it.
    /// Omitting it lets every endpoint apply its own default.
    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }

    /// Generate text.
    ///
    /// # Errors
    ///
    /// - [`LlmError::Authentication`] if the key is rejected
    /// - [`LlmError::ModelNotAvailable`] if the endpoint does not know the model
    /// - [`LlmError::RateLimitExceeded`] if throttled through every attempt
    /// - [`LlmError::Communication`] on network failure or a server error
    /// - [`LlmError::InvalidResponse`] if the body does not parse or is empty
    pub async fn generate(&self, prompt: &str) -> Result<String, LlmError> {
        let url = format!("{}/chat/completions", self.base_url);

        let request_body = ChatRequest {
            model: self.model.clone(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: prompt.to_string(),
            }],
            stream: false,
            max_tokens: self.max_tokens,
        };

        with_retries(self.max_retries, || async {
            let response = match self
                .client
                .post(&url)
                .bearer_auth(&self.api_key)
                .json(&request_body)
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
                let body = response
                    .text()
                    .await
                    .unwrap_or_else(|_| "Unknown error".to_string());
                return crate::classify_http_failure(status, &body, &self.model);
            }

            match response.json::<ChatResponse>().await {
                Ok(parsed) => match parsed.choices.into_iter().next() {
                    Some(choice) => match choice.message.content {
                        Some(text) => Attempt::Done(text),
                        None => Attempt::Fatal(LlmError::InvalidResponse(
                            "Response carried no text content".to_string(),
                        )),
                    },
                    None => Attempt::Fatal(LlmError::InvalidResponse(
                        "Response carried no choices".to_string(),
                    )),
                },
                Err(e) => Attempt::Fatal(LlmError::InvalidResponse(format!(
                    "Failed to parse response: {}",
                    e
                ))),
            }
        })
        .await
    }
}

impl LlmProviderTrait for OpenAiCompatProvider {
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
    fn named_constructors_carry_their_endpoints() {
        assert_eq!(
            OpenAiCompatProvider::openai("k", "m").base_url,
            OPENAI_BASE_URL
        );
        assert_eq!(
            OpenAiCompatProvider::openrouter("k", "m").base_url,
            OPENROUTER_BASE_URL
        );
        assert_eq!(
            OpenAiCompatProvider::deepseek("k", "m").base_url,
            DEEPSEEK_BASE_URL
        );
    }

    #[test]
    fn a_trailing_slash_does_not_become_a_double_slash() {
        let provider = OpenAiCompatProvider::new("https://example.test/v1/", "k", "m");
        assert_eq!(provider.base_url, "https://example.test/v1");
    }

    #[test]
    fn max_tokens_is_unset_until_asked_for() {
        let provider = OpenAiCompatProvider::openai("k", "m");
        assert_eq!(provider.max_tokens, None);
        assert_eq!(provider.with_max_tokens(512).max_tokens, Some(512));
    }

    #[test]
    fn debug_output_does_not_carry_the_key() {
        let rendered = format!(
            "{:?}",
            OpenAiCompatProvider::deepseek("sk-secret-value", "deepseek-chat")
        );
        assert!(!rendered.contains("sk-secret-value"), "got: {}", rendered);
        assert!(rendered.contains("<redacted>"));
    }

    #[test]
    fn an_unset_key_is_an_authentication_error_not_a_panic() {
        // A variable name no environment will have set.
        let result = crate::key_from_env("BOSWELL_TEST_ABSENT_KEY_VAR");
        assert!(matches!(result, Err(LlmError::Authentication(_))));
    }

    /// The one test in this crate that talks to a real chat-completions
    /// endpoint. Ollama serves this format at `/v1` alongside its native API,
    /// which makes it the only way to exercise the whole request-and-parse
    /// path — bearer auth, the `messages` array, `choices[0].message.content`
    /// — without a vendor account or a bill.
    ///
    /// Ignored by default: it loads a model, which on a laptop means several
    /// gigabytes of memory and a wait. Run it deliberately:
    ///
    /// ```text
    /// ollama pull granite4.2:8b
    /// cargo test -p boswell-llm --  --ignored openai_compat
    /// ```
    #[tokio::test]
    #[ignore]
    async fn talks_to_a_real_chat_completions_endpoint() {
        let provider = OpenAiCompatProvider::new(
            "http://localhost:11434/v1",
            // Ollama ignores the value but the OpenAI-compatible path still
            // wants the header.
            "ollama",
            "granite4.2:8b",
        )
        .with_max_retries(1)
        // Cold-loading eight billion parameters from disk happens inside this
        // timeout, and a reasoning model then thinks before it answers.
        .with_timeout(Duration::from_secs(600));

        let response = provider
            .generate("Reply with the single word: OK")
            .await
            .expect("Ollama should answer when it is running and the model is pulled");

        assert!(!response.trim().is_empty());
    }

    #[tokio::test]
    async fn an_unreachable_endpoint_reports_a_communication_error() {
        let provider =
            OpenAiCompatProvider::new("http://127.0.0.1:1/v1", "k", "m").with_max_retries(1);

        let result = provider.generate("test").await;
        assert!(matches!(result, Err(LlmError::Communication(_))));
    }
}
