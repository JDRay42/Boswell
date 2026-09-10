//! Retry policy for RPCs the SDK makes against an instance.
//!
//! Two failures get a second chance, and they are not the same failure.
//!
//! An `Unauthenticated` status means the session token expired. The request was
//! rejected before the handler ran, so repeating it after re-establishing the
//! session is always safe. That path gets exactly one attempt, because a second
//! rejection with a freshly issued token is a real authorization failure rather
//! than an expiry.
//!
//! A transient transport status — the server is unreachable, overloaded, or shed
//! the request — gets exponential backoff, and only for calls marked
//! [`Idempotency::Safe`]. Nothing on the wire carries an idempotency key, so a
//! repeated `Assert` is a second claim and a repeated `QueryProcedures` is a
//! second execution receipt. Until the protocol can identify a retried request,
//! mutating RPCs fail fast.

use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tonic::{Code, Status};

/// How the SDK spaces out retries of a failed RPC.
///
/// The default is three retries starting at 100ms and doubling to a 5s ceiling,
/// with jitter. Build a different one with the `with_*` methods, or turn the
/// backoff path off entirely with [`RetryPolicy::none`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RetryPolicy {
    /// Retries attempted after the initial call, per RPC.
    pub max_retries: u32,
    /// Delay before the first retry.
    pub initial_backoff: Duration,
    /// Ceiling the delay grows to and stops at.
    pub max_backoff: Duration,
    /// Factor the delay is multiplied by after each retry.
    pub multiplier: f64,
    /// Spread the delay so that clients failing together do not retry together.
    pub jitter: bool,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_secs(5),
            multiplier: 2.0,
            jitter: true,
        }
    }
}

impl RetryPolicy {
    /// A policy that never backs off.
    ///
    /// Session reconnection still happens: that is a correctness behavior the
    /// client has always had, not part of the backoff budget.
    pub const fn none() -> Self {
        Self {
            max_retries: 0,
            initial_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_millis(100),
            multiplier: 1.0,
            jitter: false,
        }
    }

    /// Set the number of retries attempted after the initial call.
    pub const fn with_max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = max_retries;
        self
    }

    /// Set the delay before the first retry.
    pub const fn with_initial_backoff(mut self, initial_backoff: Duration) -> Self {
        self.initial_backoff = initial_backoff;
        self
    }

    /// Set the ceiling the delay grows to.
    pub const fn with_max_backoff(mut self, max_backoff: Duration) -> Self {
        self.max_backoff = max_backoff;
        self
    }

    /// Set the factor the delay is multiplied by after each retry.
    pub const fn with_multiplier(mut self, multiplier: f64) -> Self {
        self.multiplier = multiplier;
        self
    }

    /// Take the jitter off, making the delays exactly the doubling sequence.
    ///
    /// Useful in tests that assert on timing. In production it makes every
    /// client that lost the same server come back at the same instant.
    pub const fn without_jitter(mut self) -> Self {
        self.jitter = false;
        self
    }

    /// The un-jittered delay before retry number `attempt`, counting from 1.
    ///
    /// `initial_backoff * multiplier^(attempt - 1)`, clamped to `max_backoff`.
    fn base_backoff(&self, attempt: u32) -> Duration {
        // `as i32` would wrap a large attempt count to a negative exponent and
        // hand back a delay *shorter* than the initial one. Saturate instead:
        // `powi(i32::MAX)` overflows to infinity, which the clamp below catches.
        let exponent = i32::try_from(attempt.saturating_sub(1)).unwrap_or(i32::MAX);
        let grown = self.initial_backoff.as_secs_f64() * self.multiplier.powi(exponent);
        if !grown.is_finite() || grown >= self.max_backoff.as_secs_f64() {
            return self.max_backoff;
        }
        Duration::from_secs_f64(grown)
    }

    /// The delay before retry number `attempt`, with jitter applied if enabled.
    ///
    /// Equal jitter: the delay lands somewhere in the top half of the range,
    /// `[base / 2, base]`. Full jitter would allow a near-zero wait, which
    /// defeats the point of backing off at all on the first retry.
    fn backoff(&self, attempt: u32) -> Duration {
        let base = self.base_backoff(attempt);
        if !self.jitter {
            return base;
        }
        let half = base / 2;
        half + half.mul_f64(jitter_fraction())
    }
}

/// Whether repeating an RPC can duplicate a side effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Idempotency {
    /// Read-only, or otherwise safe to send twice.
    Safe,
    /// Repeating it may write a second claim or issue a second receipt.
    Unsafe,
}

/// What to do about a failed RPC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RetryAction {
    /// Re-establish the session, then send the request again.
    Reconnect,
    /// Wait this long, then send the request again.
    Backoff(Duration),
    /// Give up and return the error to the caller.
    Fail,
}

/// The statuses a retry can plausibly clear, given a moment.
///
/// `Internal` and `Unknown` are deliberately absent: they say nothing about
/// whether the handler ran, so repeating them is as likely to compound the
/// problem as to fix it.
fn is_transient(code: Code) -> bool {
    matches!(
        code,
        Code::Unavailable | Code::DeadlineExceeded | Code::ResourceExhausted | Code::Aborted
    )
}

/// The per-call retry budget. One of these lives for the duration of a single
/// SDK method, across however many attempts that method makes.
#[derive(Debug)]
pub(crate) struct RetryState {
    policy: RetryPolicy,
    reconnected: bool,
    attempts: u32,
}

impl RetryState {
    /// A fresh budget under `policy`.
    pub(crate) fn new(policy: RetryPolicy) -> Self {
        Self {
            policy,
            reconnected: false,
            attempts: 0,
        }
    }

    /// Decide what `status` warrants, charging the decision against the budget.
    pub(crate) fn on_error(&mut self, status: &Status, idempotency: Idempotency) -> RetryAction {
        if status.code() == Code::Unauthenticated {
            if self.reconnected {
                return RetryAction::Fail;
            }
            self.reconnected = true;
            return RetryAction::Reconnect;
        }

        if idempotency == Idempotency::Unsafe
            || !is_transient(status.code())
            || self.attempts >= self.policy.max_retries
        {
            return RetryAction::Fail;
        }

        self.attempts += 1;
        RetryAction::Backoff(self.policy.backoff(self.attempts))
    }
}

/// A fraction in `[0, 1)` to spread a backoff by.
///
/// The clock's nanosecond field, not a PRNG. Jitter needs clients that failed
/// together to stop waking together; it does not need to be unpredictable, and
/// pulling in a random number generator to get that would be a dependency spent
/// on nothing.
fn jitter_fraction() -> f64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    f64::from(nanos) / 1_000_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transient() -> Status {
        Status::new(Code::Unavailable, "instance is down")
    }

    #[test]
    fn base_backoff_doubles_from_the_initial_delay() {
        let policy = RetryPolicy::default().without_jitter();

        assert_eq!(policy.base_backoff(1), Duration::from_millis(100));
        assert_eq!(policy.base_backoff(2), Duration::from_millis(200));
        assert_eq!(policy.base_backoff(3), Duration::from_millis(400));
    }

    #[test]
    fn base_backoff_stops_at_the_ceiling() {
        let policy = RetryPolicy::default().without_jitter();

        assert_eq!(policy.base_backoff(20), Duration::from_secs(5));
        // Far enough out that the unclamped value overflows to infinity.
        assert_eq!(policy.base_backoff(u32::MAX), Duration::from_secs(5));
    }

    #[test]
    fn jitter_keeps_the_delay_in_the_top_half_of_the_range() {
        let policy = RetryPolicy::default();

        for attempt in 1..=4 {
            let base = policy.base_backoff(attempt);
            let jittered = policy.backoff(attempt);
            assert!(
                jittered >= base / 2 && jittered <= base,
                "attempt {attempt}: {jittered:?} outside [{:?}, {base:?}]",
                base / 2
            );
        }
    }

    #[test]
    fn transient_failures_back_off_until_the_budget_runs_out() {
        let policy = RetryPolicy::default().without_jitter().with_max_retries(2);
        let mut state = RetryState::new(policy);

        assert_eq!(
            state.on_error(&transient(), Idempotency::Safe),
            RetryAction::Backoff(Duration::from_millis(100))
        );
        assert_eq!(
            state.on_error(&transient(), Idempotency::Safe),
            RetryAction::Backoff(Duration::from_millis(200))
        );
        assert_eq!(
            state.on_error(&transient(), Idempotency::Safe),
            RetryAction::Fail
        );
    }

    #[test]
    fn mutating_calls_do_not_back_off() {
        let mut state = RetryState::new(RetryPolicy::default());

        assert_eq!(
            state.on_error(&transient(), Idempotency::Unsafe),
            RetryAction::Fail
        );
    }

    #[test]
    fn a_mutating_call_still_reconnects_once() {
        let mut state = RetryState::new(RetryPolicy::default());
        let expired = Status::new(Code::Unauthenticated, "session expired");

        assert_eq!(
            state.on_error(&expired, Idempotency::Unsafe),
            RetryAction::Reconnect
        );
        assert_eq!(
            state.on_error(&expired, Idempotency::Unsafe),
            RetryAction::Fail
        );
    }

    #[test]
    fn reconnecting_does_not_spend_the_backoff_budget() {
        let policy = RetryPolicy::default().without_jitter().with_max_retries(1);
        let mut state = RetryState::new(policy);

        assert_eq!(
            state.on_error(
                &Status::new(Code::Unauthenticated, "session expired"),
                Idempotency::Safe
            ),
            RetryAction::Reconnect
        );
        assert_eq!(
            state.on_error(&transient(), Idempotency::Safe),
            RetryAction::Backoff(Duration::from_millis(100))
        );
    }

    #[test]
    fn non_transient_failures_fail_immediately() {
        let mut state = RetryState::new(RetryPolicy::default());

        for code in [
            Code::InvalidArgument,
            Code::NotFound,
            Code::PermissionDenied,
            Code::Internal,
            Code::Unknown,
        ] {
            assert_eq!(
                state.on_error(&Status::new(code, "nope"), Idempotency::Safe),
                RetryAction::Fail,
                "{code:?} should not be retried"
            );
        }
    }

    #[test]
    fn a_zero_retry_policy_never_backs_off() {
        let mut state = RetryState::new(RetryPolicy::none());

        assert_eq!(
            state.on_error(&transient(), Idempotency::Safe),
            RetryAction::Fail
        );
    }
}
