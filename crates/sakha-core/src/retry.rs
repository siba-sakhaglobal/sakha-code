//! Retry policy: exponential backoff with full jitter, per spec `modules/22-error-taxonomy-retry.md`.
//!
//! - Base 500ms, cap 60s, max 5 attempts for `Transient` errors.
//! - Respects `Retry-After` when the caller supplies one.
//! - Retries are policy, not scattered `match` arms: callers ask `next_delay`
//!   and get `None` when they should stop.

use std::time::Duration;

use crate::error::{ErrorClass, SakhaError};

/// Trait for pluggable retry policies. The default implementation is
/// `ExponentialBackoff`, but callers (e.g. tests) may substitute others.
pub trait RetryPolicy: Send + Sync {
    /// Returns the delay before the next attempt, or `None` if no further
    /// retry should be attempted (exhausted attempts, non-retryable error,
    /// or budget veto handled by the caller).
    fn next_delay(&self, attempt: u32, err: &SakhaError) -> Option<Duration>;
}

/// Exponential backoff with full jitter (AWS-style): `delay = random(0, min(cap, base * 2^attempt))`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExponentialBackoff {
    pub base: Duration,
    pub cap: Duration,
    pub max_attempts: u32,
}

impl Default for ExponentialBackoff {
    fn default() -> Self {
        Self {
            base: Duration::from_millis(500),
            cap: Duration::from_secs(60),
            max_attempts: 5,
        }
    }
}

impl ExponentialBackoff {
    pub fn new(base: Duration, cap: Duration, max_attempts: u32) -> Self {
        Self { base, cap, max_attempts }
    }

    /// Computes the deterministic upper bound for a given attempt (no jitter applied).
    pub fn bound_for_attempt(&self, attempt: u32) -> Duration {
        let exp = attempt.min(31); // avoid overflow in 2^exp
        let scaled = self.base.as_millis().saturating_mul(1u128 << exp);
        let capped = scaled.min(self.cap.as_millis());
        Duration::from_millis(capped as u64)
    }

    /// Applies full jitter: uniform random in `[0, bound]`.
    fn jittered(&self, bound: Duration) -> Duration {
        if bound.is_zero() {
            return bound;
        }
        let max_millis = bound.as_millis().max(1) as u64;
        let jittered_millis = rand::random::<u64>() % (max_millis + 1);
        Duration::from_millis(jittered_millis)
    }
}

impl RetryPolicy for ExponentialBackoff {
    fn next_delay(&self, attempt: u32, err: &SakhaError) -> Option<Duration> {
        if !err.retryable {
            return None;
        }
        if err.class != ErrorClass::Transient {
            return None;
        }
        if attempt >= self.max_attempts {
            return None;
        }
        let bound = self.bound_for_attempt(attempt);
        Some(self.jittered(bound))
    }
}

/// A retry policy that honors an explicit `Retry-After` duration (e.g. parsed
/// from an HTTP header) before falling back to exponential backoff.
#[derive(Debug, Clone, Copy)]
pub struct RetryAfterOrBackoff {
    pub retry_after: Option<Duration>,
    pub backoff: ExponentialBackoff,
}

impl RetryAfterOrBackoff {
    pub fn new(retry_after: Option<Duration>, backoff: ExponentialBackoff) -> Self {
        Self { retry_after, backoff }
    }
}

impl RetryPolicy for RetryAfterOrBackoff {
    fn next_delay(&self, attempt: u32, err: &SakhaError) -> Option<Duration> {
        if !err.retryable || err.class != ErrorClass::Transient {
            return None;
        }
        if attempt >= self.backoff.max_attempts {
            return None;
        }
        if let Some(retry_after) = self.retry_after {
            return Some(retry_after);
        }
        self.backoff.next_delay(attempt, err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transient_err() -> SakhaError {
        SakhaError::transient("test", "boom")
    }

    #[test]
    fn backoff_sequence_stays_within_jitter_bounds() {
        let policy = ExponentialBackoff::default();
        for attempt in 0..policy.max_attempts {
            let bound = policy.bound_for_attempt(attempt);
            for _ in 0..50 {
                let delay = policy.next_delay(attempt, &transient_err()).expect("should retry");
                assert!(delay <= bound, "delay {delay:?} exceeded bound {bound:?} at attempt {attempt}");
            }
        }
    }

    #[test]
    fn backoff_stops_after_max_attempts() {
        let policy = ExponentialBackoff::default();
        assert!(policy.next_delay(policy.max_attempts, &transient_err()).is_none());
        assert!(policy.next_delay(policy.max_attempts + 10, &transient_err()).is_none());
    }

    #[test]
    fn backoff_never_exceeds_cap() {
        let policy = ExponentialBackoff::new(Duration::from_millis(500), Duration::from_secs(60), 20);
        // A high attempt number would overflow without capping/min-with-cap.
        let bound = policy.bound_for_attempt(19);
        assert_eq!(bound, Duration::from_secs(60));
    }

    #[test]
    fn non_retryable_error_never_retries() {
        let policy = ExponentialBackoff::default();
        let err = SakhaError::permission("test", "denied");
        assert!(policy.next_delay(0, &err).is_none());
    }

    #[test]
    fn non_idempotent_tool_should_not_use_auto_retry_policy() {
        // Simulates "non-idempotent tool never auto-retried": the caller is
        // responsible for not invoking next_delay for non-idempotent tool calls;
        // here we assert that an InvalidInput class (used for failed validation,
        // which never should be blindly retried) yields no delay regardless.
        let policy = ExponentialBackoff::default();
        let err = SakhaError::invalid_input("sakha-tools", "bad args");
        assert!(policy.next_delay(0, &err).is_none());
    }

    #[test]
    fn retry_after_is_honored_exactly() {
        let policy = RetryAfterOrBackoff::new(Some(Duration::from_secs(7)), ExponentialBackoff::default());
        let delay = policy.next_delay(0, &transient_err()).expect("should retry");
        assert_eq!(delay, Duration::from_secs(7));
    }

    #[test]
    fn retry_after_falls_back_to_backoff_when_absent() {
        let policy = RetryAfterOrBackoff::new(None, ExponentialBackoff::default());
        let bound = policy.backoff.bound_for_attempt(0);
        let delay = policy.next_delay(0, &transient_err()).expect("should retry");
        assert!(delay <= bound);
    }
}
