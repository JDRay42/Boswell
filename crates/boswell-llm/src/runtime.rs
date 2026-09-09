//! The bridge between the synchronous [`LlmProvider`] trait and async HTTP.
//!
//! [`boswell_domain::traits::LlmProvider`] is synchronous; every provider here
//! reaches the network asynchronously. Something has to sit between them.
//!
//! Two things this module fixes over building a fresh
//! `tokio::runtime::Runtime` per call. It builds one runtime for the process
//! rather than one per prompt. And it returns an error, instead of panicking,
//! when the sync trait is called from inside an async task — `block_on` panics
//! there, and a panic inside an extraction worker takes down more than the one
//! claim it was working on.
//!
//! [`LlmProvider`]: boswell_domain::traits::LlmProvider

use crate::LlmError;
use std::future::Future;
use std::sync::OnceLock;
use tokio::runtime::Runtime;

/// Drive `future` to completion from a synchronous caller.
///
/// # Errors
///
/// Returns [`LlmError::Other`] if called from within an async task, where
/// blocking on a runtime is not allowed. Callers on an async path should await
/// the provider's own `generate` instead, or wrap this in
/// `tokio::task::spawn_blocking`.
pub(crate) fn block_on<F>(future: F) -> Result<String, LlmError>
where
    F: Future<Output = Result<String, LlmError>>,
{
    if tokio::runtime::Handle::try_current().is_ok() {
        return Err(LlmError::Other(
            "the synchronous LlmProvider trait cannot be called from inside an async task; \
             await the provider's own generate(), or wrap this call in \
             tokio::task::spawn_blocking"
                .to_string(),
        ));
    }

    bridge().block_on(future)
}

/// The one runtime this process uses to service synchronous provider calls.
fn bridge() -> &'static Runtime {
    static BRIDGE: OnceLock<Runtime> = OnceLock::new();

    BRIDGE.get_or_init(|| {
        Runtime::new().expect("failed to start the Tokio runtime backing LlmProvider calls")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drives_a_future_from_a_synchronous_caller() {
        let result = block_on(async { Ok("answer".to_string()) });
        assert_eq!(result.unwrap(), "answer");
    }

    #[test]
    fn reuses_one_runtime() {
        assert!(block_on(async { Ok("first".to_string()) }).is_ok());
        assert!(block_on(async { Ok("second".to_string()) }).is_ok());
        assert!(std::ptr::eq(bridge(), bridge()));
    }

    #[tokio::test]
    async fn refuses_to_block_inside_an_async_task() {
        let result = block_on(async { Ok("unreachable".to_string()) });

        match result {
            Err(LlmError::Other(message)) => {
                assert!(message.contains("spawn_blocking"), "got: {}", message)
            }
            other => panic!("expected a refusal, got {:?}", other),
        }
    }
}
