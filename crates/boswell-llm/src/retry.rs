//! Shared retry policy for HTTP-backed providers.
//!
//! Every provider in this crate talks to a remote endpoint over HTTP, and every
//! one of them wants the same thing: try again on the failures that are worth
//! trying again, and give up immediately on the ones that are not. Retrying a
//! bad API key three times only wastes two seconds and tells the operator
//! nothing new.

use crate::LlmError;
use std::future::Future;
use std::time::Duration;

/// What a single attempt at a provider call concluded.
pub(crate) enum Attempt {
    /// The call succeeded. This is the model's text.
    Done(String),
    /// The call failed in a way another attempt might survive: a timeout, a
    /// 5xx, a rate limit.
    Retry(LlmError),
    /// The call failed in a way no number of attempts will fix: a rejected
    /// key, an unknown model, a malformed response.
    Fatal(LlmError),
}

/// Run `attempt` until it succeeds, fails fatally, or runs out of tries.
///
/// Backoff doubles from one second. `max_retries` counts total attempts, not
/// retries after the first, which is how [`crate::OllamaProvider`] has always
/// counted them.
pub(crate) async fn with_retries<F, Fut>(
    max_retries: u32,
    mut attempt: F,
) -> Result<String, LlmError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Attempt>,
{
    let mut attempts = 0;
    let mut last_error = None;

    while attempts < max_retries {
        match attempt().await {
            Attempt::Done(text) => return Ok(text),
            Attempt::Fatal(error) => return Err(error),
            Attempt::Retry(error) => last_error = Some(error),
        }

        attempts += 1;
        if attempts < max_retries {
            tokio::time::sleep(Duration::from_secs(2u64.pow(attempts - 1))).await;
        }
    }

    Err(last_error.unwrap_or_else(|| LlmError::Communication("Max retries exceeded".to_string())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[tokio::test]
    async fn returns_the_first_success() {
        let calls = Cell::new(0);
        let result = with_retries(3, || {
            calls.set(calls.get() + 1);
            async { Attempt::Done("answer".to_string()) }
        })
        .await;

        assert_eq!(result.unwrap(), "answer");
        assert_eq!(calls.get(), 1, "a success must not be retried");
    }

    #[tokio::test]
    async fn gives_up_immediately_on_a_fatal_error() {
        let calls = Cell::new(0);
        let result = with_retries(3, || {
            calls.set(calls.get() + 1);
            async { Attempt::Fatal(LlmError::Authentication("bad key".to_string())) }
        })
        .await;

        assert!(matches!(result, Err(LlmError::Authentication(_))));
        assert_eq!(calls.get(), 1, "a rejected key is not worth a second call");
    }

    // Paused clock: these two exercise the backoff path, and a real one would
    // add three seconds of sleeping to every `cargo test` run.
    #[tokio::test(start_paused = true)]
    async fn retries_until_the_budget_runs_out_and_reports_the_last_error() {
        let calls = Cell::new(0);
        let result = with_retries(3, || {
            calls.set(calls.get() + 1);
            let attempt_number = calls.get();
            async move { Attempt::Retry(LlmError::Communication(format!("try {}", attempt_number))) }
        })
        .await;

        match result {
            Err(LlmError::Communication(message)) => assert_eq!(message, "try 3"),
            other => panic!("expected the last error, got {:?}", other),
        }
        assert_eq!(calls.get(), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn a_retry_that_later_succeeds_returns_the_success() {
        let calls = Cell::new(0);
        let result = with_retries(3, || {
            calls.set(calls.get() + 1);
            let attempt_number = calls.get();
            async move {
                if attempt_number < 2 {
                    Attempt::Retry(LlmError::RateLimitExceeded)
                } else {
                    Attempt::Done("answer".to_string())
                }
            }
        })
        .await;

        assert_eq!(result.unwrap(), "answer");
        assert_eq!(calls.get(), 2);
    }
}
