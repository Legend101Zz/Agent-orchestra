//! Rate-limit signal detection and blocking exponential backoff (issue #7).
//!
//! ## Detection is ours, on purpose
//!
//! No `--help` corpus tells us what a harness prints when a provider throttles
//! it, so — unlike a capability ([`crate::probe`]) — rate-limit detection can't
//! be probed; it is a **maintained per-adapter signal table**, a sibling to the
//! per-adapter invocation recipes in [`crate::invocation`] (the binding #16
//! decision, §4). The exact per-harness strings are that record's open question
//! 1: they are seeded here from documented provider errors and refined against
//! fixtures (issue #7's rate-limit fixture), **never guessed as if measured**.
//! Matching is case-insensitive substring containment over the worker's
//! captured stdout+stderr.
//!
//! ## Backoff via `backon`, blocking
//!
//! The retry driver is `backon`'s **blocking** `ExponentialBuilder` +
//! `StdSleeper` — no async runtime, matching orc-core's sync design (again the
//! #16 decision; `backoff` is unmaintained, `tokio-retry` would drag in tokio).
//! [`run_with_backoff`] wraps any fallible attempt in exponential-plus-jitter
//! retry and calls a notifier before each sleep so callers can emit the
//! user-visible ORC WARNING.
//!
//! A provider-supplied `retry-after` hint is *parsed and surfaced* in the
//! warning (so the operator sees what the harness asked for), but the blocking
//! `call()` path has no per-error `adjust()` — that is async-only — so the sleep
//! itself follows the exponential schedule. This is an honest, documented limit
//! of the sync path, not a silent gap.

use std::time::Duration;

use backon::{BlockingRetryable, ExponentialBuilder};

/// Signals shared by every harness/provider (documented throttling errors).
const COMMON_SIGNALS: &[&str] = &[
    "rate limit",
    "rate_limit",
    "ratelimit",
    "429",
    "too many requests",
    "quota exceeded",
    "resource_exhausted",
    "resource exhausted",
    "overloaded",
    "throttl", // throttle / throttled / throttling
    "retry after",
    "retry-after",
    "try again later",
];

/// Per-adapter extra signals beyond [`COMMON_SIGNALS`].
///
/// Seeded from each provider's documented error surface; refine with fixtures.
fn adapter_signals(adapter: &str) -> &'static [&'static str] {
    match adapter {
        // Anthropic: `overloaded_error` (529) and `rate_limit_error` (429).
        "claude" => &["overloaded_error", "rate_limit_error"],
        // OpenAI/Codex: `rate_limit_exceeded`, RPM/TPM ceilings.
        "codex" => &["rate_limit_exceeded", "requests per min", "tokens per min"],
        // MiniMax/pi: documented rate-limit status text.
        "pi" => &["request limit", "concurrency limit"],
        // Hermes and OpenCode add nothing beyond the common set yet.
        _ => &[],
    }
}

/// A detected rate-limit signal in a worker's captured output.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RateLimitSignal {
    /// Adapter whose signal table matched.
    pub adapter: String,
    /// The lowercase pattern that matched.
    pub matched: String,
    /// Seconds the harness asked us to wait, when it announced a hint.
    pub retry_after: Option<u64>,
}

/// Detect a rate-limit signal for `adapter` in captured worker output.
///
/// Returns the first matching signal (adapter-specific patterns take priority
/// over the common set), with any parsed `retry-after` hint attached.
#[must_use]
pub fn detect(adapter: &str, text: &str) -> Option<RateLimitSignal> {
    if text.is_empty() {
        return None;
    }
    let lower = text.to_ascii_lowercase();
    let matched = adapter_signals(adapter)
        .iter()
        .chain(COMMON_SIGNALS)
        .find(|pattern| lower.contains(**pattern))?;
    Some(RateLimitSignal {
        adapter: adapter.to_owned(),
        matched: (*matched).to_owned(),
        retry_after: parse_retry_after(&lower),
    })
}

/// Whether captured output for `adapter` shows a rate-limit signal.
#[must_use]
pub fn is_rate_limited(adapter: &str, text: &str) -> bool {
    detect(adapter, text).is_some()
}

/// Parse a `retry-after`/"try again in N" seconds hint from lowercased text.
fn parse_retry_after(lower: &str) -> Option<u64> {
    const KEYS: &[&str] = &[
        "retry-after",
        "retry_after",
        "retryafter",
        "retry after",
        "try again in",
        "please retry in",
        "please try again in",
    ];
    for key in KEYS {
        if let Some(index) = lower.find(key) {
            let window: String = lower[index + key.len()..].chars().take(24).collect();
            if let Some(seconds) = digits_after(&window) {
                return Some(seconds);
            }
        }
    }
    None
}

/// First run of digits after leading separators; `None` if a word intervenes.
fn digits_after(window: &str) -> Option<u64> {
    let mut started = false;
    let mut value = String::new();
    for ch in window.chars() {
        if ch.is_ascii_digit() {
            started = true;
            value.push(ch);
        } else if started {
            break;
        } else if ch.is_ascii_alphabetic() {
            return None;
        }
    }
    value.parse::<u64>().ok()
}

/// Exponential backoff configuration for one retryable operation.
#[derive(Clone, Copy, Debug)]
pub struct BackoffPolicy {
    /// Delay before the first retry.
    pub min_delay: Duration,
    /// Ceiling for any single backoff delay.
    pub max_delay: Duration,
    /// Growth factor applied between retries.
    pub factor: f32,
    /// Maximum number of retries (total attempts = `max_retries` + 1).
    pub max_retries: usize,
    /// Whether to add random jitter within `(0, delay)`.
    pub jitter: bool,
}

impl BackoffPolicy {
    /// The production schedule: 2s → 60s, ×2, jittered, up to 4 retries.
    #[must_use]
    pub const fn production() -> Self {
        Self {
            min_delay: Duration::from_secs(2),
            max_delay: Duration::from_secs(60),
            factor: 2.0,
            max_retries: 4,
            jitter: true,
        }
    }

    /// A millisecond-scale schedule for deterministic tests.
    #[must_use]
    pub const fn fast() -> Self {
        Self {
            min_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(8),
            factor: 2.0,
            max_retries: 3,
            jitter: false,
        }
    }

    fn builder(&self) -> ExponentialBuilder {
        let builder = ExponentialBuilder::default()
            .with_factor(self.factor)
            .with_min_delay(self.min_delay)
            .with_max_delay(self.max_delay)
            .with_max_times(self.max_retries);
        if self.jitter {
            builder.with_jitter()
        } else {
            builder
        }
    }
}

impl Default for BackoffPolicy {
    fn default() -> Self {
        Self::production()
    }
}

/// Run `attempt` under exponential backoff, retrying only when `is_retryable`.
///
/// `notify` is invoked once before each backoff sleep with the failing error and
/// the delay `backon` chose, so the caller can surface an ORC WARNING. Retries
/// stop at the policy's `max_retries`; the last error is returned unchanged.
pub fn run_with_backoff<T, E, A, R, N>(
    policy: &BackoffPolicy,
    attempt: A,
    is_retryable: R,
    notify: N,
) -> Result<T, E>
where
    A: FnMut() -> Result<T, E>,
    R: FnMut(&E) -> bool,
    N: FnMut(&E, Duration),
{
    attempt
        .retry(policy.builder())
        .when(is_retryable)
        .notify(notify)
        .call()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn detects_common_and_adapter_specific_signals() {
        // Common signal, any adapter.
        assert!(is_rate_limited("hermes", "HTTP 429 Too Many Requests"));
        assert!(is_rate_limited(
            "opencode",
            "provider is overloaded, slow down"
        ));
        // Adapter-specific: claude's overloaded_error / rate_limit_error.
        let claude = detect(
            "claude",
            r#"{"type":"error","error":{"type":"rate_limit_error"}}"#,
        )
        .expect("claude rate_limit_error detected");
        assert_eq!(claude.matched, "rate_limit_error");
        // Codex RPM ceiling.
        assert!(is_rate_limited(
            "codex",
            "Error: rate_limit_exceeded (requests per min)"
        ));
        // A clean success is not a signal.
        assert!(!is_rate_limited(
            "pi",
            "done: wrote 3 files, all tests pass"
        ));
    }

    #[test]
    fn parses_retry_after_hints_in_several_shapes() {
        assert_eq!(
            detect("hermes", "429 rate limit; retry-after: 30")
                .and_then(|signal| signal.retry_after),
            Some(30)
        );
        assert_eq!(
            detect(
                "codex",
                r#"{"error":"rate_limit_exceeded","retry_after":12}"#
            )
            .and_then(|signal| signal.retry_after),
            Some(12)
        );
        assert_eq!(
            detect("pi", "Too many requests, please try again in 5 seconds")
                .and_then(|signal| signal.retry_after),
            Some(5)
        );
        // A signal with no numeric hint still detects, hint is None.
        assert_eq!(
            detect("hermes", "rate limit reached, try again later")
                .and_then(|signal| signal.retry_after),
            None
        );
    }

    #[test]
    fn backoff_retries_until_success_then_stops() {
        let calls = Cell::new(0_usize);
        let notified = Cell::new(0_usize);
        let result: Result<&str, &str> = run_with_backoff(
            &BackoffPolicy::fast(),
            || {
                calls.set(calls.get() + 1);
                if calls.get() < 3 {
                    Err("rate limited")
                } else {
                    Ok("ok")
                }
            },
            |_error| true,
            |_error, _delay| notified.set(notified.get() + 1),
        );
        assert_eq!(result, Ok("ok"));
        assert_eq!(calls.get(), 3, "two failures then a success");
        assert_eq!(notified.get(), 2, "one warning per backoff sleep");
    }

    #[test]
    fn backoff_gives_up_after_max_retries_and_returns_last_error() {
        let calls = Cell::new(0_usize);
        let result: Result<(), &str> = run_with_backoff(
            &BackoffPolicy::fast(),
            || {
                calls.set(calls.get() + 1);
                Err("still limited")
            },
            |_error| true,
            |_error, _delay| {},
        );
        assert_eq!(result, Err("still limited"));
        // fast() allows 3 retries → 4 total attempts.
        assert_eq!(calls.get(), 4);
    }

    #[test]
    fn a_non_retryable_error_is_returned_without_retrying() {
        let calls = Cell::new(0_usize);
        let result: Result<(), &str> = run_with_backoff(
            &BackoffPolicy::fast(),
            || {
                calls.set(calls.get() + 1);
                Err("fatal")
            },
            |_error| false,
            |_error, _delay| {},
        );
        assert_eq!(result, Err("fatal"));
        assert_eq!(calls.get(), 1, "a non-retryable error never retries");
    }
}
