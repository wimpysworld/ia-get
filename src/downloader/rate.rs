//! Optional throughput throttling (`--limit-rate`): paces the bytes written
//! to a `.part` file to a target rate, so a download does not saturate the
//! connection. Disabled by default (no limit).

use std::time::{Duration, Instant};

use crate::Result;
use crate::error::IaGetError;

/// Multipliers for the human-readable rate suffixes, in bytes
const KB: f64 = 1024.0;
const MB: f64 = KB * 1024.0;
const GB: f64 = MB * 1024.0;

/// Throttles a transfer to a target rate in bytes/second.
///
/// A `None` limit (or zero) disables throttling. Pacing keeps the transfer
/// at or below the limit without starving it: each chunk extends a schedule
/// by the time its bytes would take at the limit, and the caller sleeps
/// whenever that schedule runs ahead of the real clock.
pub(crate) struct RateLimiter {
    /// Target throughput in bytes per second; `None` disables throttling.
    limit: Option<u64>,
    /// The instant the transfer is "scheduled" to have delivered its bytes so
    /// far; the pacing deadline the next chunk waits for.
    deadline: Instant,
}

impl RateLimiter {
    pub(crate) fn new(limit: Option<u64>) -> Self {
        Self {
            limit,
            deadline: Instant::now(),
        }
    }

    /// Sleeps as long as needed so that, by the time this returns, `bytes`
    /// delivered by the caller stay at or below the configured rate. A
    /// disabled limiter returns immediately.
    pub(crate) async fn pace(&mut self, bytes: u64) {
        let Some(limit) = self.limit.filter(|&limit| limit > 0) else {
            return;
        };
        if bytes == 0 {
            return;
        }

        // The time this chunk's bytes may take at the limit.
        let budget = Duration::from_secs_f64(bytes as f64 / limit as f64);
        let next = self.deadline + budget;
        let now = Instant::now();

        if next > now {
            // The schedule is ahead of the real clock: wait it out.
            tokio::time::sleep(next - now).await;
            self.deadline = next;
        } else {
            // The server (or an earlier wait) already took at least as long
            // as the budget: do not bank the surplus, reset the baseline.
            self.deadline = now;
        }
    }
}

/// Parses a `--limit-rate` value into bytes/second.
///
/// Accepts a bare byte count or one with a case-insensitive `K`/`M`/`G`
/// suffix, optionally followed by `B` (e.g. "1M", "512KB", "2048", "0").
/// `0` yields `0`, which the limiter treats as "no limit".
pub fn parse_rate(input: &str) -> Result<u64> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(IaGetError::InvalidRate(input.to_string()));
    }

    let (number, multiplier) = split_suffix(trimmed);
    let value: f64 = number.parse().map_err(|_| {
        IaGetError::InvalidRate(format!(
            "invalid number in rate limit {input:?} (expected e.g. \"1M\", \"512K\" or bytes/second)"
        ))
    })?;
    if !value.is_finite() || value < 0.0 {
        return Err(IaGetError::InvalidRate(input.to_string()));
    }

    Ok((value * multiplier).round() as u64)
}

/// Splits a rate string into its (owned) numeric part and byte multiplier.
/// The input must already be non-empty; an unknown or missing suffix means
/// the value is a plain bytes/second count.
fn split_suffix(input: &str) -> (String, f64) {
    let lower = input.to_ascii_lowercase();
    if let Some(number) = lower.strip_suffix("kb") {
        (number.to_string(), KB)
    } else if let Some(number) = lower.strip_suffix("mb") {
        (number.to_string(), MB)
    } else if let Some(number) = lower.strip_suffix("gb") {
        (number.to_string(), GB)
    } else if let Some(number) = lower.strip_suffix('k') {
        (number.to_string(), KB)
    } else if let Some(number) = lower.strip_suffix('m') {
        (number.to_string(), MB)
    } else if let Some(number) = lower.strip_suffix('g') {
        (number.to_string(), GB)
    } else if let Some(number) = lower.strip_suffix('b') {
        (number.to_string(), 1.0)
    } else {
        (input.to_string(), 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rate_accepts_plain_bytes() {
        assert_eq!(parse_rate("2048").unwrap(), 2048);
        assert_eq!(parse_rate("0").unwrap(), 0);
        assert_eq!(parse_rate("  512  ").unwrap(), 512);
    }

    #[test]
    fn parse_rate_accepts_k_m_g_suffixes() {
        assert_eq!(parse_rate("1K").unwrap(), 1024);
        assert_eq!(parse_rate("512kb").unwrap(), 512 * 1024);
        assert_eq!(parse_rate("1M").unwrap(), 1024 * 1024);
        assert_eq!(parse_rate("2MB").unwrap(), 2 * 1024 * 1024);
        assert_eq!(parse_rate("1G").unwrap(), 1024 * 1024 * 1024);
    }

    #[test]
    fn parse_rate_accepts_fractional_rates() {
        assert_eq!(parse_rate("1.5M").unwrap(), (1.5 * MB).round() as u64);
        assert_eq!(parse_rate("0.5K").unwrap(), 512);
    }

    #[test]
    fn parse_rate_is_case_insensitive_on_the_suffix() {
        assert_eq!(parse_rate("1m").unwrap(), 1024 * 1024);
        assert_eq!(parse_rate("2Gb").unwrap(), 2 * GB as u64);
    }

    #[test]
    fn parse_rate_rejects_non_numeric_input() {
        for input in ["", "  ", "fast", "1X", "1.2.3M", "-5"] {
            assert!(
                parse_rate(input).is_err(),
                "{input:?} must not parse as a rate"
            );
        }
    }

    #[tokio::test]
    async fn disabled_limiter_never_waits() {
        for limit in [None, Some(0)] {
            let mut rate = RateLimiter::new(limit);
            let start = Instant::now();
            rate.pace(10_000).await;
            assert!(start.elapsed() < Duration::from_millis(200));
        }
    }

    #[tokio::test]
    async fn enabled_limiter_paces_to_the_limit() {
        // 1_000_000 bytes/s: a 1MB chunk should take ~1s to pace. The
        // assertion window keeps the test from being flaky on a slow box
        // while still catching a limiter that does not wait at all.
        let mut rate = RateLimiter::new(Some(1_000_000));

        let start = Instant::now();
        rate.pace(1_000_000).await;
        let elapsed = start.elapsed();

        assert!(
            elapsed >= Duration::from_millis(900),
            "a 1MB chunk at 1MB/s must pace to at least ~1s, took {elapsed:?}"
        );
        assert!(
            elapsed <= Duration::from_millis(1_500),
            "pacing must not overshoot the budget far, took {elapsed:?}"
        );
    }
}
