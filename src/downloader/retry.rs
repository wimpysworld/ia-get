//! Retry policies, exponential backoff, `Retry-After` parsing, and interruptible shutdown waits.

use std::collections::hash_map::RandomState;
use std::future::Future;
use std::hash::BuildHasher;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime};

use colored::Colorize;
use reqwest::{RequestBuilder, Response, StatusCode, Url};
use tokio::sync::Notify;

use super::redirect::{send_following_redirects, RedirectSendOutcome};
use crate::constants::{
    INITIAL_RETRY_DELAY_MS, MAX_REDIRECTS, MAX_RETRIES, MAX_RETRY_AFTER_SECS, MAX_RETRY_DELAY_SECS,
};
use crate::error::IaGetError;
use crate::utils::{sanitize_url_for_log, VerboseLogger};
use crate::Result;

/// Configuration controlling retry attempts and backoff delays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    pub max_retries: u32,
    pub initial_delay: Duration,
    pub max_delay: Duration,
    pub max_retry_after: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: MAX_RETRIES,
            initial_delay: Duration::from_millis(INITIAL_RETRY_DELAY_MS),
            max_delay: Duration::from_secs(MAX_RETRY_DELAY_SECS),
            max_retry_after: Duration::from_secs(MAX_RETRY_AFTER_SECS),
        }
    }
}

/// Context passed to retry callbacks before waiting for the next attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryEvent<'a> {
    pub attempt: u32,
    pub max_retries: u32,
    pub delay: Duration,
    pub reason: &'a str,
}

/// Shared shutdown signal that wakes waiting tasks immediately on Ctrl+C.
#[derive(Clone, Debug)]
pub struct ShutdownSignal {
    running: Arc<AtomicBool>,
    notify: Arc<Notify>,
}

impl ShutdownSignal {
    /// Creates a new active shutdown signal.
    pub fn new() -> Self {
        Self {
            running: Arc::new(AtomicBool::new(true)),
            notify: Arc::new(Notify::new()),
        }
    }

    /// Returns `true` while shutdown has not been triggered.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Marks shutdown as triggered and wakes all waiting tasks immediately.
    pub fn trigger(&self) {
        self.running.store(false, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    /// Returns the underlying atomic flag indicating whether work should continue.
    pub fn running_flag(&self) -> &Arc<AtomicBool> {
        &self.running
    }
}

impl Default for ShutdownSignal {
    fn default() -> Self {
        Self::new()
    }
}

static SHUTDOWN_SIGNAL: OnceLock<ShutdownSignal> = OnceLock::new();

/// Installs a process-wide Ctrl+C handler once and returns the shared shutdown signal.
pub fn install_signal_handler() -> ShutdownSignal {
    SHUTDOWN_SIGNAL
        .get_or_init(|| {
            let signal = ShutdownSignal::new();
            let handler_signal = signal.clone();

            match ctrlc::set_handler(move || {
                handler_signal.trigger();
                println!(
                    "\n{} Received Ctrl+C, finishing current operation...",
                    "✘".red().bold()
                );
            }) {
                Ok(()) | Err(ctrlc::Error::MultipleHandlers) => {}
                Err(err) => {
                    eprintln!("Warning: failed to install Ctrl+C handler: {}", err);
                }
            }

            signal
        })
        .clone()
}

pub(super) fn interrupted_error() -> IaGetError {
    std::io::Error::new(
        std::io::ErrorKind::Interrupted,
        "Download interrupted by user",
    )
    .into()
}

/// Sleeps for `duration`, waking immediately with an interrupted error if `signal` triggers.
pub async fn sleep_interruptible(duration: Duration, signal: &ShutdownSignal) -> Result<()> {
    if !signal.is_running() {
        return Err(interrupted_error());
    }
    if duration.is_zero() {
        return Ok(());
    }
    await_interruptible(tokio::time::sleep(duration), signal).await
}

pub(super) async fn await_interruptible<Fut, T>(future: Fut, signal: &ShutdownSignal) -> Result<T>
where
    Fut: Future<Output = T>,
{
    let notified = signal.notify.notified();
    tokio::pin!(notified);
    notified.as_mut().enable();

    if !signal.is_running() {
        return Err(interrupted_error());
    }

    tokio::select! {
        biased;
        () = &mut notified => Err(interrupted_error()),
        output = future => Ok(output),
    }
}

/// Returns whether an HTTP status code represents a transient failure worth retrying.
pub fn is_retryable_status(status: StatusCode) -> bool {
    status == StatusCode::REQUEST_TIMEOUT
        || status == StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error()
}

/// Returns whether a `reqwest` error represents a transient failure worth retrying.
pub fn is_retryable_reqwest_error(err: &reqwest::Error) -> bool {
    !err.is_builder() && !err.is_redirect() && err.status().is_none_or(is_retryable_status)
}

pub(super) fn parse_ascii_u64(input: &str) -> Option<u64> {
    if input.is_empty() || !input.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    input.parse::<u64>().ok()
}

/// Parses an RFC 9110 Section 10.2.3 `Retry-After` header (delta-seconds or HTTP-date)
/// and clamps the resulting delay to `max_delay`.
pub fn parse_retry_after(value: &str, now: SystemTime, max_delay: Duration) -> Option<Duration> {
    let trimmed = value.trim_matches(|c| c == ' ' || c == '\t');
    if trimmed.is_empty() || !trimmed.is_ascii() || trimmed.bytes().any(|b| b.is_ascii_control()) {
        return None;
    }

    if trimmed.bytes().all(|b| b.is_ascii_digit()) {
        return Some(match trimmed.parse::<u64>() {
            Ok(secs) => Duration::from_secs(secs).min(max_delay),
            Err(_) => max_delay,
        });
    }

    let target = httpdate::parse_http_date(trimmed).ok()?;
    let delta = target.duration_since(now).unwrap_or(Duration::ZERO);
    Some(delta.min(max_delay))
}

fn random_u64() -> u64 {
    RandomState::new().hash_one(0u64)
}

/// Computes equal-jitter exponential backoff in `[base/2, base]` for a 1-based `attempt`
/// using an explicit random `sample`.
pub fn compute_backoff_delay_with_sample(
    attempt: u32,
    policy: &RetryPolicy,
    sample: u64,
) -> Duration {
    let initial_nanos = policy.initial_delay.as_nanos();
    let max_nanos = policy.max_delay.as_nanos();
    let base_nanos = if initial_nanos == 0 || max_nanos == 0 {
        0
    } else {
        let shift = attempt.saturating_sub(1);
        if shift >= 128 {
            max_nanos
        } else {
            let factor = 1u128 << shift;
            initial_nanos.saturating_mul(factor).min(max_nanos)
        }
    };

    let half = base_nanos / 2;
    let spread = base_nanos - half;
    let denom = u128::from(u64::MAX);
    let sample_u128 = u128::from(sample);
    let jitter = (spread / denom) * sample_u128 + ((spread % denom) * sample_u128) / denom;
    let total_nanos = half + jitter;

    Duration::new(
        (total_nanos / 1_000_000_000) as u64,
        (total_nanos % 1_000_000_000) as u32,
    )
}

pub(super) fn compute_retry_delay_with_source(
    attempt: u32,
    retry_after_header: Option<&str>,
    now: SystemTime,
    policy: &RetryPolicy,
) -> (Duration, &'static str) {
    if let Some(delay) =
        retry_after_header.and_then(|value| parse_retry_after(value, now, policy.max_retry_after))
    {
        return (delay, "retry-after");
    }

    (
        compute_backoff_delay_with_sample(attempt, policy, random_u64()),
        "backoff",
    )
}

enum AttemptOutcome {
    Ready(Response),
    Retry {
        delay: Duration,
        delay_source: &'static str,
        reason: String,
    },
}

async fn send_attempt<F>(
    initial_url: &Url,
    build_request: &mut F,
    retry_count: u32,
    policy: &RetryPolicy,
    signal: &ShutdownSignal,
    logger: &VerboseLogger,
) -> Result<AttemptOutcome>
where
    F: FnMut(&Url) -> Result<RequestBuilder>,
{
    match send_following_redirects(initial_url, signal, MAX_REDIRECTS, logger, build_request)
        .await?
    {
        RedirectSendOutcome::Response(response) => {
            let status = response.status();
            if is_retryable_status(status) {
                if retry_count >= policy.max_retries {
                    logger.log(
                        "retry",
                        format_args!(
                            "exhausted ({}/{}) for {} after HTTP {}",
                            retry_count,
                            policy.max_retries,
                            sanitize_url_for_log(initial_url),
                            status
                        ),
                    );
                    return match response.error_for_status() {
                        Ok(resp) => Ok(AttemptOutcome::Ready(resp)),
                        Err(err) => Err(err.without_url().into()),
                    };
                }

                let retry_after = response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|value| value.to_str().ok());
                let (delay, delay_source) = compute_retry_delay_with_source(
                    retry_count + 1,
                    retry_after,
                    SystemTime::now(),
                    policy,
                );
                let reason = format!("HTTP {}", status);
                drop(response);
                Ok(AttemptOutcome::Retry {
                    delay,
                    delay_source,
                    reason,
                })
            } else {
                let response = response
                    .error_for_status()
                    .map_err(reqwest::Error::without_url)?;
                Ok(AttemptOutcome::Ready(response))
            }
        }
        RedirectSendOutcome::TransportError(err) => {
            let err = err.without_url();
            if !is_retryable_reqwest_error(&err) || retry_count >= policy.max_retries {
                if is_retryable_reqwest_error(&err) {
                    logger.log(
                        "retry",
                        format_args!(
                            "exhausted ({}/{}) for {}: {err}",
                            retry_count,
                            policy.max_retries,
                            sanitize_url_for_log(initial_url),
                        ),
                    );
                }
                return Err(err.into());
            }

            let (delay, delay_source) =
                compute_retry_delay_with_source(retry_count + 1, None, SystemTime::now(), policy);
            let reason = err.to_string();
            Ok(AttemptOutcome::Retry {
                delay,
                delay_source,
                reason,
            })
        }
    }
}

/// Logs a retry attempt, notifies `on_retry`, and sleeps interruptibly for `delay`.
#[allow(clippy::too_many_arguments)]
async fn log_and_wait_retry<H>(
    retry_count: &mut u32,
    policy: &RetryPolicy,
    initial_url: &Url,
    delay: Duration,
    delay_source: &str,
    reason: &str,
    signal: &ShutdownSignal,
    logger: &VerboseLogger,
    on_retry: &mut H,
) -> Result<()>
where
    H: FnMut(&RetryEvent<'_>),
{
    *retry_count += 1;
    logger.log(
        "retry",
        format_args!(
            "attempt {}/{} for {} after {}; waiting {:.2}s ({})",
            *retry_count,
            policy.max_retries,
            sanitize_url_for_log(initial_url),
            reason,
            delay.as_secs_f64(),
            delay_source
        ),
    );
    on_retry(&RetryEvent {
        attempt: *retry_count,
        max_retries: policy.max_retries,
        delay,
        reason,
    });
    sleep_interruptible(delay, signal).await
}

/// Sends an HTTP request with retries and diagnostic logging on transient transport errors and
/// retryable HTTP statuses, following redirects manually per attempt and honouring `Retry-After`
/// headers and `ShutdownSignal`.
pub async fn send_with_retry_with_logger<F, H>(
    initial_url: &Url,
    policy: &RetryPolicy,
    signal: &ShutdownSignal,
    logger: &VerboseLogger,
    mut build_request: F,
    mut on_retry: H,
) -> Result<Response>
where
    F: FnMut(&Url) -> Result<RequestBuilder>,
    H: FnMut(&RetryEvent<'_>),
{
    let mut retry_count: u32 = 0;

    loop {
        match send_attempt(
            initial_url,
            &mut build_request,
            retry_count,
            policy,
            signal,
            logger,
        )
        .await?
        {
            AttemptOutcome::Ready(response) => return Ok(response),
            AttemptOutcome::Retry {
                delay,
                delay_source,
                reason,
            } => {
                log_and_wait_retry(
                    &mut retry_count,
                    policy,
                    initial_url,
                    delay,
                    delay_source,
                    &reason,
                    signal,
                    logger,
                    &mut on_retry,
                )
                .await?;
            }
        }
    }
}

/// Fetches a text response body with retries and diagnostic logging on transient transport errors,
/// retryable HTTP statuses, and transient body read errors, following redirects manually per
/// attempt and honouring `Retry-After` and `ShutdownSignal`.
pub async fn fetch_text_with_retry_with_logger<F, H>(
    initial_url: &Url,
    policy: &RetryPolicy,
    signal: &ShutdownSignal,
    logger: &VerboseLogger,
    mut build_request: F,
    mut on_retry: H,
) -> Result<String>
where
    F: FnMut(&Url) -> Result<RequestBuilder>,
    H: FnMut(&RetryEvent<'_>),
{
    let mut retry_count: u32 = 0;

    loop {
        match send_attempt(
            initial_url,
            &mut build_request,
            retry_count,
            policy,
            signal,
            logger,
        )
        .await?
        {
            AttemptOutcome::Retry {
                delay,
                delay_source,
                reason,
            } => {
                log_and_wait_retry(
                    &mut retry_count,
                    policy,
                    initial_url,
                    delay,
                    delay_source,
                    &reason,
                    signal,
                    logger,
                    &mut on_retry,
                )
                .await?;
            }
            AttemptOutcome::Ready(response) => {
                match await_interruptible(response.text(), signal).await? {
                    Ok(text) => return Ok(text),
                    Err(err) => {
                        let err = err.without_url();
                        if !is_retryable_reqwest_error(&err) || retry_count >= policy.max_retries {
                            if is_retryable_reqwest_error(&err) {
                                logger.log(
                                    "retry",
                                    format_args!(
                                        "exhausted ({}/{}) for {} reading body: {err}",
                                        retry_count,
                                        policy.max_retries,
                                        sanitize_url_for_log(initial_url),
                                    ),
                                );
                            }
                            return Err(err.into());
                        }
                        let (delay, delay_source) = compute_retry_delay_with_source(
                            retry_count + 1,
                            None,
                            SystemTime::now(),
                            policy,
                        );
                        let reason = err.to_string();
                        log_and_wait_retry(
                            &mut retry_count,
                            policy,
                            initial_url,
                            delay,
                            delay_source,
                            &reason,
                            signal,
                            logger,
                            &mut on_retry,
                        )
                        .await?;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::UNIX_EPOCH;

    use super::*;
    use crate::downloader::http_client_builder;
    use crate::downloader::tests::DownloadFixture;
    use crate::test_support::{
        fast_retry_policy, md5_hex, slow_retry_policy, MockResponse, MockServer,
    };
    use tokio::net::TcpListener;

    fn utc_time(year: i64, month: u8, day: u8, hour: u8, minute: u8, second: u8) -> SystemTime {
        let y = year - i64::from(month <= 2);
        let era = y.div_euclid(400);
        let yoe = y.rem_euclid(400);
        let m = i64::from(month);
        let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + i64::from(day) - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        let days = era * 146_097 + doe - 719_468;
        let secs =
            days * 86_400 + i64::from(hour) * 3600 + i64::from(minute) * 60 + i64::from(second);
        if secs >= 0 {
            UNIX_EPOCH + Duration::from_secs(secs as u64)
        } else {
            UNIX_EPOCH - Duration::from_secs(secs.unsigned_abs())
        }
    }

    #[test]
    fn test_parse_retry_after_valid_deltas_and_http_dates() {
        let now = utc_time(2026, 5, 1, 12, 0, 0);
        let cap = Duration::from_secs(60);
        let base_1994 = utc_time(1994, 11, 6, 8, 49, 0);

        let cases = [
            // Delta-seconds
            ("0", now, cap, Duration::ZERO),
            ("00", now, cap, Duration::ZERO),
            ("5", now, cap, Duration::from_secs(5)),
            ("  42  ", now, cap, Duration::from_secs(42)),
            ("60", now, cap, cap),
            ("120", now, cap, cap),
            ("999999999999999999999999999999999999", now, cap, cap),
            ("10", now, Duration::ZERO, Duration::ZERO),
            // IMF-fixdate: future, equal, past, far-future clamp, subsecond `now`
            (
                "Sun, 06 Nov 1994 08:49:37 GMT",
                base_1994,
                cap,
                Duration::from_secs(37),
            ),
            (
                "Sun, 06 Nov 1994 08:49:37 GMT",
                utc_time(1994, 11, 6, 8, 49, 37),
                cap,
                Duration::ZERO,
            ),
            (
                "Sun, 06 Nov 1994 08:49:37 GMT",
                utc_time(1994, 11, 6, 9, 0, 0),
                cap,
                Duration::ZERO,
            ),
            ("Sun, 06 Nov 1994 09:49:37 GMT", base_1994, cap, cap),
            (
                "Sun, 06 Nov 1994 08:49:05 GMT",
                base_1994 + Duration::from_millis(250),
                cap,
                Duration::from_millis(4_750),
            ),
            // Valid leap years (divisible by 400 and divisible by 4 not 100)
            (
                "Tue, 29 Feb 2000 00:00:30 GMT",
                utc_time(2000, 2, 29, 0, 0, 0),
                cap,
                Duration::from_secs(30),
            ),
            (
                "Thu, 29 Feb 2024 00:00:15 GMT",
                utc_time(2024, 2, 29, 0, 0, 0),
                cap,
                Duration::from_secs(15),
            ),
            // RFC 850: current century, past century pivot, and century wrap
            (
                "Sunday, 06-Nov-94 08:49:37 GMT",
                base_1994,
                cap,
                Duration::from_secs(37),
            ),
            (
                "Sunday, 06-Nov-94 08:49:37 GMT",
                utc_time(2026, 1, 1, 0, 0, 0),
                cap,
                Duration::ZERO,
            ),
            (
                "Saturday, 01-Jan-00 00:00:30 GMT",
                utc_time(1999, 12, 31, 23, 59, 50),
                cap,
                Duration::from_secs(40),
            ),
            // asctime(): single-digit padded day and two-digit day
            (
                "Sun Nov  6 08:49:37 1994",
                base_1994,
                cap,
                Duration::from_secs(37),
            ),
            (
                "Wed Nov 16 08:49:37 1994",
                utc_time(1994, 11, 16, 8, 49, 17),
                cap,
                Duration::from_secs(20),
            ),
            ("Wed Nov 16 08:49:37 1994", base_1994, cap, cap),
        ];

        for (header, base, max_cap, expected) in cases {
            assert_eq!(
                parse_retry_after(header, base, max_cap),
                Some(expected),
                "header={header:?}"
            );
        }
    }

    #[test]
    fn test_parse_retry_after_invalid_inputs() {
        let now = utc_time(2026, 5, 1, 12, 0, 0);
        let cap = Duration::from_secs(60);

        for invalid in [
            "",
            "   ",
            "-5",
            "+5",
            "1.5",
            "10s",
            "abc",
            "1 2",
            "5\u{00a0}",
            // Non-GMT timezones
            "Sun, 06 Nov 1994 08:49:37 UTC",
            "Sun, 06 Nov 1994 08:49:37 EST",
            "Sun, 06 Nov 1994 08:49:37 +0000",
            "Sunday, 06-Nov-94 08:49:37 UTC",
            "Sun Nov  6 08:49:37 1994 GMT",
            // Invalid weekday / month
            "Sux, 06 Nov 1994 08:49:37 GMT",
            "Funday, 06-Nov-94 08:49:37 GMT",
            "Sun, 06 Foo 1994 08:49:37 GMT",
            "Sun Foo  6 08:49:37 1994",
            // Invalid leap-year and calendar dates
            "Wed, 29 Feb 2023 00:00:00 GMT",
            "Thu, 29 Feb 1900 00:00:00 GMT",
            "Mon, 29 Feb 2100 00:00:00 GMT",
            "Sun, 00 Nov 1994 08:49:37 GMT",
            "Sun, 31 Apr 1994 08:49:37 GMT",
            "Sun, 32 Jan 1994 08:49:37 GMT",
            "Sun Nov  0 08:49:37 1994",
            // Invalid times
            "Sun, 06 Nov 1994 24:00:00 GMT",
            "Sun, 06 Nov 1994 08:60:00 GMT",
            "Sun, 06 Nov 1994 08:49:60 GMT",
            // Missing punctuation / wrong spacing
            "Sun 06 Nov 1994 08:49:37 GMT",
            "Sun Nov 6 08:49:37 1994",
        ] {
            assert_eq!(
                parse_retry_after(invalid, now, cap),
                None,
                "expected None for {invalid:?}"
            );
        }
    }

    #[test]
    fn test_compute_backoff_and_retry_delay() {
        let policy = RetryPolicy {
            max_retries: 5,
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(10),
            max_retry_after: Duration::from_secs(60),
        };

        // Attempt 0 saturates exponent to 0; attempts 1..=5 double and cap at 10s; large attempts saturate cleanly.
        for (attempt, sample, expected) in [
            (0, 0, Duration::from_millis(500)),
            (1, 0, Duration::from_millis(500)),
            (1, u64::MAX, Duration::from_secs(1)),
            (2, 0, Duration::from_secs(1)),
            (2, u64::MAX, Duration::from_secs(2)),
            (3, 0, Duration::from_secs(2)),
            (3, u64::MAX, Duration::from_secs(4)),
            (4, 0, Duration::from_secs(4)),
            (4, u64::MAX, Duration::from_secs(8)),
            (5, 0, Duration::from_secs(5)),
            (5, u64::MAX, Duration::from_secs(10)),
            (64, 0, Duration::from_secs(5)),
            (64, u64::MAX, Duration::from_secs(10)),
            (128, 0, Duration::from_secs(5)),
            (128, u64::MAX, Duration::from_secs(10)),
            (1000, 0, Duration::from_secs(5)),
            (1000, u64::MAX, Duration::from_secs(10)),
            (u32::MAX, 0, Duration::from_secs(5)),
            (u32::MAX, u64::MAX, Duration::from_secs(10)),
        ] {
            assert_eq!(
                compute_backoff_delay_with_sample(attempt, &policy, sample),
                expected,
                "attempt={attempt} sample={sample}"
            );
        }

        let mid = compute_backoff_delay_with_sample(1, &policy, u64::MAX / 2);
        assert!(
            mid >= Duration::from_millis(749) && mid <= Duration::from_millis(751),
            "expected ~750ms midpoint, got {mid:?}"
        );

        let zero_policy = RetryPolicy {
            initial_delay: Duration::ZERO,
            ..policy
        };
        assert_eq!(
            compute_backoff_delay_with_sample(1, &zero_policy, u64::MAX),
            Duration::ZERO
        );

        // compute_retry_delay_with_source prefers valid Retry-After (clamped) and falls back to jittered backoff
        let now = utc_time(2026, 5, 1, 12, 0, 0);
        let retry_policy = RetryPolicy {
            max_retries: 3,
            initial_delay: Duration::from_secs(2),
            max_delay: Duration::from_secs(16),
            max_retry_after: Duration::from_secs(30),
        };
        assert_eq!(
            compute_retry_delay_with_source(1, Some("12"), now, &retry_policy),
            (Duration::from_secs(12), "retry-after")
        );
        assert_eq!(
            compute_retry_delay_with_source(1, Some("120"), now, &retry_policy),
            (Duration::from_secs(30), "retry-after")
        );
        let (fallback_none, source_none) =
            compute_retry_delay_with_source(1, None, now, &retry_policy);
        assert_eq!(source_none, "backoff");
        assert!(
            fallback_none >= Duration::from_secs(1) && fallback_none <= Duration::from_secs(2),
            "expected fallback in [1s, 2s], got {fallback_none:?}"
        );
        let (fallback_invalid, source_invalid) =
            compute_retry_delay_with_source(2, Some("not-a-date"), now, &retry_policy);
        assert_eq!(source_invalid, "backoff");
        assert!(
            fallback_invalid >= Duration::from_secs(2)
                && fallback_invalid <= Duration::from_secs(4),
            "expected fallback in [2s, 4s], got {fallback_invalid:?}"
        );
    }

    #[test]
    fn test_is_retryable_status_classification() {
        for retryable in [
            StatusCode::REQUEST_TIMEOUT,
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::INTERNAL_SERVER_ERROR,
            StatusCode::BAD_GATEWAY,
            StatusCode::SERVICE_UNAVAILABLE,
            StatusCode::GATEWAY_TIMEOUT,
        ] {
            assert!(
                is_retryable_status(retryable),
                "expected {retryable} to be retryable"
            );
        }

        for non_retryable in [
            StatusCode::OK,
            StatusCode::PARTIAL_CONTENT,
            StatusCode::FOUND,
            StatusCode::BAD_REQUEST,
            StatusCode::UNAUTHORIZED,
            StatusCode::FORBIDDEN,
            StatusCode::NOT_FOUND,
            StatusCode::GONE,
            StatusCode::RANGE_NOT_SATISFIABLE,
        ] {
            assert!(
                !is_retryable_status(non_retryable),
                "expected {non_retryable} not to be retryable"
            );
        }
    }

    #[tokio::test]
    async fn test_is_retryable_reqwest_error_classification() {
        let client = http_client_builder().build().unwrap();

        // Builder error (invalid URL) is permanent and must not be retried
        let builder_err = client.get("://not-a-url").send().await.unwrap_err();
        assert!(builder_err.is_builder());
        assert!(!is_retryable_reqwest_error(&builder_err));

        // Connection refused is a transient transport error and must be retried
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let closed_addr = listener.local_addr().unwrap();
        drop(listener);
        let connect_err = client
            .get(format!("http://{closed_addr}/"))
            .send()
            .await
            .unwrap_err();
        assert!(is_retryable_reqwest_error(&connect_err));
    }

    #[tokio::test]
    async fn test_sleep_interruptible() {
        // Completes when running
        let running = ShutdownSignal::new();
        assert!(running.is_running());
        assert!(running.running_flag().load(Ordering::SeqCst));
        sleep_interruptible(Duration::ZERO, &running).await.unwrap();
        sleep_interruptible(Duration::from_millis(10), &running)
            .await
            .unwrap();

        // Already triggered returns immediately
        let triggered = ShutdownSignal::new();
        triggered.trigger();
        assert!(!triggered.is_running());
        let start = std::time::Instant::now();
        let err = sleep_interruptible(Duration::from_secs(60), &triggered)
            .await
            .unwrap_err();
        assert!(start.elapsed() < Duration::from_secs(1));
        assert!(matches!(err, IaGetError::FileSystem(_)));
        assert!(err.to_string().contains("Download interrupted by user"));

        // Triggered mid-wait wakes promptly
        let mid_wait = ShutdownSignal::new();
        let trigger_signal = mid_wait.clone();
        let trigger_task = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            trigger_signal.trigger();
        });
        let start = std::time::Instant::now();
        let err = sleep_interruptible(Duration::from_secs(60), &mid_wait)
            .await
            .unwrap_err();
        trigger_task.await.unwrap();
        assert!(start.elapsed() < Duration::from_secs(2));
        assert!(err.to_string().contains("Download interrupted by user"));
    }

    #[tokio::test]
    async fn test_send_and_fetch_text_with_retry_behavior() {
        let client = http_client_builder().build().unwrap();
        let policy = fast_retry_policy(3);
        let signal = ShutdownSignal::new();
        let logger = VerboseLogger::disabled();

        // Transient 503 + 429 (with Retry-After: 0) then 200 succeeds
        let server = MockServer::start(vec![
            MockResponse::status("503 Service Unavailable", b"busy"),
            MockResponse::retry_after("429 Too Many Requests", "0", b"slow down"),
            MockResponse::ok(b"<files/>"),
        ])
        .await;
        let mut events = Vec::new();
        let meta_url = server.parsed_url("/meta.xml");
        let body = fetch_text_with_retry_with_logger(
            &meta_url,
            &policy,
            &signal,
            &logger,
            |target_url| Ok(client.get(target_url.clone())),
            |event| {
                events.push((
                    event.attempt,
                    event.max_retries,
                    event.delay,
                    event.reason.to_string(),
                ));
            },
        )
        .await
        .unwrap();

        let requests = server.finish().await;
        assert_eq!(body, "<files/>");
        assert_eq!(requests.len(), 3);
        assert_eq!(events.len(), 2);
        assert_eq!((events[0].0, events[0].1), (1, 3));
        assert!(events[0].3.contains("503"));
        assert_eq!((events[1].0, events[1].2), (2, Duration::ZERO));
        assert!(events[1].3.contains("429"));

        // Permanent 404 fails immediately without retrying
        let server_404 =
            MockServer::start(vec![MockResponse::status("404 Not Found", b"missing")]).await;
        let mut retry_calls = 0;
        let missing_url = server_404.parsed_url("/missing");
        let err = send_with_retry_with_logger(
            &missing_url,
            &policy,
            &signal,
            &logger,
            |target_url| Ok(client.get(target_url.clone())),
            |_| retry_calls += 1,
        )
        .await
        .unwrap_err();
        assert_eq!(server_404.finish().await.len(), 1);
        assert_eq!(retry_calls, 0);
        assert!(matches!(err, IaGetError::Network(_)));

        // Exhausting max_retries on persistent 503 returns error
        let small_policy = fast_retry_policy(2);
        let server_503 = MockServer::start(vec![
            MockResponse::status("503 Service Unavailable", b"1"),
            MockResponse::status("503 Service Unavailable", b"2"),
            MockResponse::status("503 Service Unavailable", b"3"),
        ])
        .await;
        let mut attempts = Vec::new();
        let down_url = server_503.parsed_url("/down");
        let err = send_with_retry_with_logger(
            &down_url,
            &small_policy,
            &signal,
            &logger,
            |target_url| Ok(client.get(target_url.clone())),
            |event| attempts.push(event.attempt),
        )
        .await
        .unwrap_err();
        assert_eq!(server_503.finish().await.len(), 3);
        assert_eq!(attempts, vec![1, 2]);
        assert!(matches!(err, IaGetError::Network(_)));

        // Triggering ShutdownSignal during retry sleep aborts immediately
        let server_cancel = MockServer::start(vec![MockResponse::retry_after(
            "429 Too Many Requests",
            "60",
            b"rate limited",
        )])
        .await;
        let cancel_signal = ShutdownSignal::new();
        let trigger_signal = cancel_signal.clone();
        let start = std::time::Instant::now();
        let rate_limited_url = server_cancel.parsed_url("/rate-limited");
        let err = send_with_retry_with_logger(
            &rate_limited_url,
            &slow_retry_policy(2),
            &cancel_signal,
            &logger,
            |target_url| Ok(client.get(target_url.clone())),
            |_| trigger_signal.trigger(),
        )
        .await
        .unwrap_err();
        assert_eq!(server_cancel.finish().await.len(), 1);
        assert!(start.elapsed() < Duration::from_secs(2));
        assert!(err.to_string().contains("Download interrupted by user"));
    }

    #[tokio::test]
    async fn test_download_retryable_statuses_and_retry_after_recover_on_200() {
        // Large backoff (30s) with 50ms Retry-After cap proves Retry-After headers are
        // parsed and clamped rather than falling back to slow backoff or sleeping seconds.
        let clamped_retry_after_policy = RetryPolicy {
            max_retries: 2,
            initial_delay: Duration::from_secs(30),
            max_delay: Duration::from_secs(30),
            max_retry_after: Duration::from_millis(50),
        };
        let backoff_policy = fast_retry_policy(2);

        let cases: [(&str, &str, Option<&str>, &RetryPolicy, Duration); 5] = [
            (
                "429-retry-after-0",
                "429 Too Many Requests",
                Some("0"),
                &clamped_retry_after_policy,
                Duration::ZERO,
            ),
            (
                "429-retry-after-delta-clamped",
                "429 Too Many Requests",
                Some("999999"),
                &clamped_retry_after_policy,
                Duration::from_millis(40),
            ),
            (
                "503-retry-after-imf-fixdate",
                "503 Service Unavailable",
                Some("Thu, 01 Jan 2099 00:00:00 GMT"),
                &clamped_retry_after_policy,
                Duration::from_millis(40),
            ),
            (
                "408-backoff",
                "408 Request Timeout",
                None,
                &backoff_policy,
                Duration::ZERO,
            ),
            (
                "502-backoff",
                "502 Bad Gateway",
                None,
                &backoff_policy,
                Duration::ZERO,
            ),
        ];

        for (label, status_line, retry_after, policy, min_elapsed) in cases {
            let payload = format!("recovered payload for {label}");
            let expected_md5 = md5_hex(payload.as_bytes());
            let first_response = match retry_after {
                Some(header) => MockResponse::retry_after(status_line, header, b"transient"),
                None => MockResponse::status(status_line, b"transient"),
            };
            let server =
                MockServer::start(vec![first_response, MockResponse::ok(payload.as_bytes())]).await;
            let fx = DownloadFixture::new(&format!("retry-{label}"));

            let start = std::time::Instant::now();
            fx.download_with_policy(
                &server.url("/item.bin"),
                "item.bin",
                Some(&expected_md5),
                None,
                policy,
            )
            .await
            .unwrap_or_else(|err| panic!("case {label}: expected recovery on 200, got {err}"));
            let elapsed = start.elapsed();

            let requests = server.finish().await;
            assert_eq!(requests.len(), 2, "case {label}");
            assert!(
                elapsed >= min_elapsed && elapsed < Duration::from_secs(1),
                "case {label}: unexpected elapsed {elapsed:?} (min {min_elapsed:?})"
            );
            fx.assert_downloaded("item.bin", payload.as_bytes());
        }
    }

    #[tokio::test]
    async fn test_download_ctrl_c_during_retry_wait_exits_promptly() {
        let server = MockServer::start(vec![
            MockResponse::retry_after("429 Too Many Requests", "30", b"rate limited"),
            MockResponse::ok(b"should never be downloaded"),
        ])
        .await;

        let fx = DownloadFixture::new("ctrl-c-retry-wait");
        let policy = slow_retry_policy(3);

        let requests_handle = server.requests_handle();
        let trigger_signal = fx.signal.clone();
        let trigger_task = tokio::spawn(async move {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
            while tokio::time::Instant::now() < deadline {
                if !requests_handle.lock().await.is_empty() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
            trigger_signal.trigger();
        });

        let start = std::time::Instant::now();
        let err = fx
            .download_with_policy(&server.url("/item.bin"), "item.bin", None, None, &policy)
            .await
            .unwrap_err();
        let elapsed = start.elapsed();
        trigger_task.await.unwrap();

        let requests = server.abort().await;
        assert!(
            elapsed < Duration::from_secs(1),
            "Ctrl+C during 30s Retry-After wait should exit promptly, took {elapsed:?}"
        );
        assert_eq!(requests.len(), 1);
        assert!(matches!(err, IaGetError::FileSystem(_)));
        assert!(
            err.to_string().contains("Download interrupted by user"),
            "unexpected error: {err}"
        );
        assert!(!fx.part_path("item.bin").exists());
        assert!(!fx.join("item.bin").exists());
    }

    #[test]
    fn test_install_signal_handler_repeated_calls_share_atomic_bool() {
        let first = install_signal_handler();
        let second = install_signal_handler();
        let third = install_signal_handler();

        assert!(first.is_running());
        assert!(second.is_running());
        assert!(third.is_running());
        assert!(Arc::ptr_eq(first.running_flag(), second.running_flag()));
        assert!(Arc::ptr_eq(second.running_flag(), third.running_flag()));
        assert!(Arc::ptr_eq(&first.notify, &second.notify));
    }
}
