//! Retry and backoff for failing requests: the exponential delay schedule,
//! a +/-20% jitter, the Retry-After cap, and the per-file [`RetryTracker`]
//! that owns the retry count and the interruptible wait.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::Result;
use crate::display::{print_max_retries_exceeded, print_retry_notice, print_retry_wait};
use crate::error::IaGetError;

/// Maximum number of retry attempts for a single failing request
pub(crate) const MAX_RETRIES: u32 = 10;

/// Initial delay between retries in milliseconds (doubles with each retry)
const INITIAL_RETRY_DELAY_MS: u64 = 5_000;

/// Upper bound for the exponential backoff delay in milliseconds
const MAX_RETRY_DELAY_MS: u64 = 60_000;

/// Upper bound (in seconds) for a server-provided Retry-After value
const MAX_RETRY_AFTER_SECS: u64 = 900;

// Numerical constants for the linear congruential generator used to add jitter
const LCG_MULTIPLIER: u64 = 6364136223846793005;
const LCG_INCREMENT: u64 = 1442695040888963407;

/// How often interruptible waits (retry backoff, a stalled body read) wake
/// up to check for a Ctrl+C, so a long server-requested wait (up to 15 min)
/// or a stalled transfer does not outlive the user's request to stop
pub(crate) const INTERRUPT_CHECK_INTERVAL: Duration = Duration::from_millis(500);

/// Applies +/-20% jitter to a delay so that many clients do not retry in sync.
///
/// Uses a linear congruential generator seeded from the current time instead
/// of pulling in a `rand` dependency.
fn jitter_ms(value_ms: u64) -> u64 {
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E37_79B9_7F4A_7C15);
    let x = seed
        .wrapping_mul(LCG_MULTIPLIER)
        .wrapping_add(LCG_INCREMENT);
    let factor = 80u64 + (x >> 33) % 41; // 80..=120 percent
    value_ms.saturating_mul(factor) / 100
}

/// Exponential backoff delay for a given 1-based retry attempt.
///
/// Starts at `INITIAL_RETRY_DELAY_MS`, doubles per attempt, is capped at
/// `MAX_RETRY_DELAY_MS`, and then has +/-20% jitter applied.
pub(crate) fn backoff_delay(attempt: u32) -> Duration {
    let exp = attempt.saturating_sub(1).min(20);
    let base_ms = INITIAL_RETRY_DELAY_MS.saturating_mul(1u64 << exp);
    let capped_ms = base_ms.min(MAX_RETRY_DELAY_MS);
    Duration::from_millis(jitter_ms(capped_ms))
}

/// Parses a Retry-After header value (RFC 7231): a delta-seconds integer,
/// or an HTTP-date read as "that moment minus now" (a date already in the
/// past means wait no more). The result is capped at `MAX_RETRY_AFTER_SECS`;
/// an unparseable value yields `None`.
pub(crate) fn parse_retry_after(value: &str) -> Option<u64> {
    let trimmed = value.trim();
    if !trimmed.is_empty() && trimmed.bytes().all(|b| b.is_ascii_digit()) {
        // A digit-only value is a seconds count: in range it parses,
        // beyond u64 it clamps to the cap — either way it must not fall
        // through to the HTTP-date parse
        return trimmed
            .parse::<u64>()
            .map(|secs| secs.min(MAX_RETRY_AFTER_SECS))
            .ok()
            .or(Some(MAX_RETRY_AFTER_SECS));
    }
    let date = httpdate::parse_http_date(trimmed).ok()?;
    let now = SystemTime::now();
    Some(
        date.duration_since(now)
            .unwrap_or_default()
            .as_secs()
            .min(MAX_RETRY_AFTER_SECS),
    )
}

/// Tracks how many times a file transfer has been retried and how long to
/// wait before each retry, so retry call sites stay short.
pub(crate) struct RetryTracker {
    count: u32,
    delay: fn(u32) -> Duration,
}

impl RetryTracker {
    pub(crate) fn new(delay: fn(u32) -> Duration) -> Self {
        Self { count: 0, delay }
    }

    /// Records a failed attempt, prints the retry notice and waits.
    ///
    /// Returns an error once `MAX_RETRIES` has been exhausted or the user
    /// interrupted the run while the wait was in progress.
    pub(crate) async fn record(
        &mut self,
        kind: &str,
        detail: &str,
        retry_after_secs: Option<u64>,
        running: &Arc<AtomicBool>,
    ) -> Result<()> {
        self.count += 1;

        if self.count > MAX_RETRIES {
            print_max_retries_exceeded(MAX_RETRIES);
            return Err(IaGetError::Network {
                detail: format!("{kind}: {detail} (maximum retries {MAX_RETRIES} exceeded)"),
                source: None,
            });
        }

        let delay = retry_after_secs
            .map(Duration::from_secs)
            .unwrap_or_else(|| (self.delay)(self.count));

        print_retry_notice(kind, self.count, MAX_RETRIES, detail);
        print_retry_wait(&delay, retry_after_secs.is_some());

        // A zero delay (e.g. Retry-After: 0) skips the wait loop below
        // entirely: check the flag here so a stop already requested is
        // honored before the next request goes out.
        if !running.load(Ordering::SeqCst) {
            return Err(IaGetError::Interrupted);
        }

        // Sleep in slices and check the flag at each slice boundary: a
        // Ctrl+C during a long wait (a server-requested Retry-After of up
        // to 15 min) must stop the retry loop without sleeping the whole
        // delay out.
        let mut remaining = delay;
        while remaining > Duration::ZERO {
            let slice = remaining.min(INTERRUPT_CHECK_INTERVAL);
            tokio::time::sleep(slice).await;
            remaining -= slice;
            if !running.load(Ordering::SeqCst) {
                return Err(IaGetError::Interrupted);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_retry_after_seconds() {
        assert_eq!(parse_retry_after("30"), Some(30));
        assert_eq!(parse_retry_after("  5 "), Some(5));
        assert_eq!(parse_retry_after("0"), Some(0));
        assert_eq!(parse_retry_after("999999"), Some(MAX_RETRY_AFTER_SECS));
        // All-digit but beyond u64: a seconds count that overflows, clamped
        // to the cap (not a misparse into the HTTP-date form)
        assert_eq!(
            parse_retry_after("18446744073709551616"),
            Some(MAX_RETRY_AFTER_SECS)
        );
        assert_eq!(parse_retry_after(""), None);
        assert_eq!(parse_retry_after("next tuesday"), None);
    }

    #[test]
    fn parse_retry_after_http_date() {
        // A past HTTP-date means "wait no more" (RFC 7231: a Retry-After
        // date earlier than the current time is equivalent to 0)
        assert_eq!(parse_retry_after("Wed, 21 Oct 2015 07:28:00 GMT"), Some(0));

        // A date a few minutes out yields that many seconds (the
        // parse-to-compare window allows a second of drift)
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("valid clock")
            .as_secs();
        let date = httpdate::fmt_http_date(UNIX_EPOCH + Duration::from_secs(now + 300));
        let secs = parse_retry_after(&date).expect("an HTTP-date must parse");
        assert!(
            (299..=301).contains(&secs),
            "a 300s-out date must yield ~300s, got {secs}"
        );

        // Beyond the cap the date is clamped
        let far = httpdate::fmt_http_date(UNIX_EPOCH + Duration::from_secs(now + 86_400));
        assert_eq!(parse_retry_after(&far), Some(MAX_RETRY_AFTER_SECS));
    }

    #[test]
    fn backoff_delay_stays_within_bounds() {
        let d1 = backoff_delay(1);
        assert!(
            (4000..=6000).contains(&d1.as_millis()),
            "attempt 1: {:?}",
            d1
        );
        let d2 = backoff_delay(2);
        assert!(
            (8000..=12000).contains(&d2.as_millis()),
            "attempt 2: {:?}",
            d2
        );
        let d60 = backoff_delay(60);
        assert!(
            (48000..=72000).contains(&d60.as_millis()),
            "attempt 60 must be capped at 60s ±20%: {:?}",
            d60
        );
    }
}
