//! Module for handling file downloads, verification, and related operations.

use std::collections::hash_map::RandomState;
use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::future::Future;
use std::hash::{BuildHasher, Hash};
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use colored::*;
use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::{Client, ClientBuilder, RequestBuilder, Response, StatusCode, Url};
use tokio::sync::Notify;

use crate::constants::{
    INITIAL_RETRY_DELAY_MS, MAX_REDIRECTS, MAX_RETRIES, MAX_RETRY_AFTER_SECS, MAX_RETRY_DELAY_SECS,
};
use crate::cookie::CookieSource;
use crate::error::IaGetError; // Import IaGetError for explicit error conversion
use crate::utils::{
    create_progress_bar, format_duration, format_size, format_transfer_rate, sanitize_url_for_log,
    VerboseLogger,
};
use crate::Result; // Import utility functions

/// Buffer size for file operations (8KB)
const BUFFER_SIZE: usize = 8192;

/// File size threshold for showing hash progress bar (2MB)
const LARGE_FILE_THRESHOLD: u64 = 2 * 1024 * 1024;

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

/// Idle stall threshold beyond which [`RateLimiter`] resets its pacing window
/// so network stalls cannot accumulate multi-second unthrottled bursts.
const RATE_LIMIT_STALL_RESET_WINDOW: Duration = Duration::from_millis(100);

/// Configuration controlling download retries, bandwidth throttling, and verbose logging.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DownloadOptions {
    pub retry_policy: RetryPolicy,
    pub limit_rate: Option<u64>,
    pub verbose: bool,
}

/// Parses a bandwidth limit string into bytes per second.
///
/// Accepts positive ASCII integers with an optional case-insensitive binary (1024-based)
/// unit suffix: none or `B` (`1`), `K`/`KB` (`1024`), `M`/`MB` (`1024^2`), `G`/`GB` (`1024^3`).
pub fn parse_rate_limit(input: &str) -> Result<u64> {
    if input.is_empty() {
        return Err(IaGetError::InvalidRateLimit {
            input: input.to_string(),
            reason: "rate must not be empty".to_string(),
        });
    }

    if input.chars().any(char::is_whitespace) {
        return Err(IaGetError::InvalidRateLimit {
            input: input.to_string(),
            reason: "rate must not contain whitespace".to_string(),
        });
    }

    let digit_end = input.bytes().take_while(|b| b.is_ascii_digit()).count();
    if digit_end == 0 {
        let reason = if input.starts_with('+') || input.starts_with('-') {
            "rate must be a positive integer without a sign"
        } else if input.starts_with('.') {
            "decimal values are not supported; use a whole number"
        } else {
            "expected a positive integer with an optional unit suffix (B, K, KB, M, MB, G, GB)"
        };
        return Err(IaGetError::InvalidRateLimit {
            input: input.to_string(),
            reason: reason.to_string(),
        });
    }

    let (num_str, suffix) = input.split_at(digit_end);
    let multiplier: u64 = if suffix.is_empty() || suffix.eq_ignore_ascii_case("B") {
        1
    } else if suffix.eq_ignore_ascii_case("K") || suffix.eq_ignore_ascii_case("KB") {
        1024
    } else if suffix.eq_ignore_ascii_case("M") || suffix.eq_ignore_ascii_case("MB") {
        1024 * 1024
    } else if suffix.eq_ignore_ascii_case("G") || suffix.eq_ignore_ascii_case("GB") {
        1024 * 1024 * 1024
    } else if suffix.starts_with('.') {
        return Err(IaGetError::InvalidRateLimit {
            input: input.to_string(),
            reason: "decimal values are not supported; use a whole number".to_string(),
        });
    } else {
        return Err(IaGetError::InvalidRateLimit {
            input: input.to_string(),
            reason: format!("unknown unit suffix '{suffix}'; expected B, K, KB, M, MB, G, or GB"),
        });
    };

    let value = num_str
        .parse::<u64>()
        .map_err(|_| IaGetError::InvalidRateLimit {
            input: input.to_string(),
            reason: "rate exceeds maximum supported value".to_string(),
        })?;

    if value == 0 {
        return Err(IaGetError::InvalidRateLimit {
            input: input.to_string(),
            reason: "rate must be greater than zero".to_string(),
        });
    }

    value
        .checked_mul(multiplier)
        .ok_or_else(|| IaGetError::InvalidRateLimit {
            input: input.to_string(),
            reason: "rate exceeds maximum supported value".to_string(),
        })
}

/// Paces chunked transfers to an average byte-per-second ceiling.
#[derive(Debug, Clone)]
pub struct RateLimiter {
    bytes_per_sec: u64,
    started_at: Instant,
    bytes_transferred: u64,
}

impl RateLimiter {
    /// Creates a rate limiter targeting `bytes_per_sec`, starting at the current instant.
    pub fn new(bytes_per_sec: u64) -> Self {
        Self::with_start(bytes_per_sec, Instant::now())
    }

    /// Creates a rate limiter targeting `bytes_per_sec` with an explicit window start instant.
    pub fn with_start(bytes_per_sec: u64, started_at: Instant) -> Self {
        Self {
            bytes_per_sec,
            started_at,
            bytes_transferred: 0,
        }
    }

    fn expected_duration_for_bytes(bytes: u64, bytes_per_sec: u64) -> Duration {
        let nanos = (u128::from(bytes) * 1_000_000_000) / u128::from(bytes_per_sec);
        let secs = (nanos / 1_000_000_000) as u64;
        let subsec_nanos = (nanos % 1_000_000_000) as u32;
        Duration::new(secs, subsec_nanos)
    }

    /// Records `chunk_bytes` transferred at `now` and returns the delay required to stay
    /// within the configured rate ceiling.
    pub fn delay_for_chunk(&mut self, chunk_bytes: u64, now: Instant) -> Duration {
        if chunk_bytes == 0 || self.bytes_per_sec == 0 {
            return Duration::ZERO;
        }

        let mut elapsed = now.saturating_duration_since(self.started_at);
        let prev_expected =
            Self::expected_duration_for_bytes(self.bytes_transferred, self.bytes_per_sec);
        if elapsed > prev_expected.saturating_add(RATE_LIMIT_STALL_RESET_WINDOW) {
            self.started_at = now;
            self.bytes_transferred = 0;
            elapsed = Duration::ZERO;
        }

        self.bytes_transferred = self.bytes_transferred.saturating_add(chunk_bytes);
        let expected =
            Self::expected_duration_for_bytes(self.bytes_transferred, self.bytes_per_sec);
        expected.saturating_sub(elapsed)
    }

    /// Sleeps as needed to honour the configured rate limit for `chunk_bytes`,
    /// waking immediately if `signal` triggers.
    pub async fn throttle(&mut self, chunk_bytes: u64, signal: &ShutdownSignal) -> Result<()> {
        let delay = self.delay_for_chunk(chunk_bytes, Instant::now());
        if delay > Duration::ZERO {
            sleep_interruptible(delay, signal).await?;
        }
        Ok(())
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

fn interrupted_error() -> IaGetError {
    std::io::Error::new(
        std::io::ErrorKind::Interrupted,
        "Download interrupted by user",
    )
    .into()
}

/// Sleeps for `duration`, waking immediately with an interrupted error if `signal` triggers.
pub async fn sleep_interruptible(duration: Duration, signal: &ShutdownSignal) -> Result<()> {
    let notified = signal.notify.notified();
    tokio::pin!(notified);
    notified.as_mut().enable();

    if !signal.is_running() {
        return Err(interrupted_error());
    }

    if duration.is_zero() {
        return Ok(());
    }

    tokio::select! {
        biased;
        () = &mut notified => Err(interrupted_error()),
        () = tokio::time::sleep(duration) => Ok(()),
    }
}

async fn await_interruptible<Fut, T>(future: Fut, signal: &ShutdownSignal) -> Result<T>
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

fn parse_fixed_u8(input: &str, expected_len: usize) -> Option<u8> {
    if input.len() != expected_len || !input.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    input.parse::<u8>().ok()
}

fn parse_fixed_i64(input: &str, expected_len: usize) -> Option<i64> {
    if input.len() != expected_len || !input.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    input.parse::<i64>().ok()
}

fn is_short_weekday(input: &str) -> bool {
    matches!(input, "Mon" | "Tue" | "Wed" | "Thu" | "Fri" | "Sat" | "Sun")
}

fn is_long_weekday(input: &str) -> bool {
    matches!(
        input,
        "Monday" | "Tuesday" | "Wednesday" | "Thursday" | "Friday" | "Saturday" | "Sunday"
    )
}

fn parse_month(input: &str) -> Option<u8> {
    match input {
        "Jan" => Some(1),
        "Feb" => Some(2),
        "Mar" => Some(3),
        "Apr" => Some(4),
        "May" => Some(5),
        "Jun" => Some(6),
        "Jul" => Some(7),
        "Aug" => Some(8),
        "Sep" => Some(9),
        "Oct" => Some(10),
        "Nov" => Some(11),
        "Dec" => Some(12),
        _ => None,
    }
}

fn parse_hms(input: &str) -> Option<(u8, u8, u8)> {
    let bytes = input.as_bytes();
    if bytes.len() != 8 || bytes[2] != b':' || bytes[5] != b':' {
        return None;
    }
    let hour = parse_fixed_u8(&input[0..2], 2)?;
    let minute = parse_fixed_u8(&input[3..5], 2)?;
    let second = parse_fixed_u8(&input[6..8], 2)?;
    if hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    Some((hour, minute, second))
}

fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

fn days_in_month(year: i64, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

fn days_from_civil(year: i64, month: u8, day: u8) -> i64 {
    let y = year - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let m = i64::from(month);
    let d = i64::from(day);
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_year_from_days(days_since_epoch: i64) -> i64 {
    let z = days_since_epoch + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = mp + if mp < 10 { 3 } else { -9 };
    y + i64::from(m <= 2)
}

fn system_time_to_unix_nanos(time: SystemTime) -> i128 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => {
            i128::from(duration.as_secs()) * 1_000_000_000 + i128::from(duration.subsec_nanos())
        }
        Err(err) => {
            let duration = err.duration();
            -(i128::from(duration.as_secs()) * 1_000_000_000 + i128::from(duration.subsec_nanos()))
        }
    }
}

fn parse_imf_fixdate(input: &str) -> Option<(i64, u8, u8, u8, u8, u8)> {
    if input.len() != 29 {
        return None;
    }
    let bytes = input.as_bytes();
    if !is_short_weekday(&input[0..3])
        || &input[3..5] != ", "
        || bytes[7] != b' '
        || bytes[11] != b' '
        || bytes[16] != b' '
        || &input[25..29] != " GMT"
    {
        return None;
    }
    let day = parse_fixed_u8(&input[5..7], 2)?;
    let month = parse_month(&input[8..11])?;
    let year = parse_fixed_i64(&input[12..16], 4)?;
    let (hour, minute, second) = parse_hms(&input[17..25])?;
    Some((year, month, day, hour, minute, second))
}

fn parse_rfc850_date(input: &str, now_year: i64) -> Option<(i64, u8, u8, u8, u8, u8)> {
    let (weekday, rest) = input.split_once(", ")?;
    if !is_long_weekday(weekday) || rest.len() != 22 {
        return None;
    }
    let bytes = rest.as_bytes();
    if bytes[2] != b'-' || bytes[6] != b'-' || bytes[9] != b' ' || &rest[18..22] != " GMT" {
        return None;
    }
    let day = parse_fixed_u8(&rest[0..2], 2)?;
    let month = parse_month(&rest[3..6])?;
    let two_digit_year = parse_fixed_i64(&rest[7..9], 2)?;
    let (hour, minute, second) = parse_hms(&rest[10..18])?;

    let mut year = (now_year / 100) * 100 + two_digit_year;
    if year > now_year + 50 {
        year -= 100;
    } else if year + 50 <= now_year {
        year += 100;
    }

    Some((year, month, day, hour, minute, second))
}

fn parse_asctime_date(input: &str) -> Option<(i64, u8, u8, u8, u8, u8)> {
    if input.len() != 24 {
        return None;
    }
    let bytes = input.as_bytes();
    if !is_short_weekday(&input[0..3])
        || bytes[3] != b' '
        || bytes[7] != b' '
        || bytes[10] != b' '
        || bytes[19] != b' '
    {
        return None;
    }
    let month = parse_month(&input[4..7])?;
    let day_str = &input[8..10];
    let day = if let Some(single_digit) = day_str.strip_prefix(' ') {
        parse_fixed_u8(single_digit, 1)?
    } else {
        parse_fixed_u8(day_str, 2)?
    };
    let (hour, minute, second) = parse_hms(&input[11..19])?;
    let year = parse_fixed_i64(&input[20..24], 4)?;
    Some((year, month, day, hour, minute, second))
}

/// Parses an RFC 9110 Section 10.2.3 `Retry-After` header (delta-seconds or HTTP-date)
/// and clamps the resulting delay to `max_delay`.
pub fn parse_retry_after(value: &str, now: SystemTime, max_delay: Duration) -> Option<Duration> {
    let trimmed = value.trim_matches(|c| c == ' ' || c == '\t');
    if trimmed.is_empty() || !trimmed.is_ascii() {
        return None;
    }

    if trimmed.bytes().all(|b| b.is_ascii_digit()) {
        return Some(match parse_ascii_u64(trimmed) {
            Some(secs) => Duration::from_secs(secs).min(max_delay),
            None => max_delay,
        });
    }

    let now_nanos = system_time_to_unix_nanos(now);
    let now_secs = now_nanos.div_euclid(1_000_000_000);
    let now_days = now_secs.div_euclid(86_400) as i64;
    let now_year = civil_year_from_days(now_days);

    let (year, month, day, hour, minute, second) = parse_imf_fixdate(trimmed)
        .or_else(|| parse_rfc850_date(trimmed, now_year))
        .or_else(|| parse_asctime_date(trimmed))?;

    if day == 0 || day > days_in_month(year, month) {
        return None;
    }

    let target_secs = i128::from(days_from_civil(year, month, day)) * 86_400
        + i128::from(hour) * 3600
        + i128::from(minute) * 60
        + i128::from(second);
    let target_nanos = target_secs * 1_000_000_000;
    let delta_nanos = target_nanos - now_nanos;

    if delta_nanos <= 0 {
        return Some(Duration::ZERO);
    }

    let max_nanos =
        i128::from(max_delay.as_secs()) * 1_000_000_000 + i128::from(max_delay.subsec_nanos());
    let clamped_nanos = delta_nanos.min(max_nanos);
    let secs = (clamped_nanos / 1_000_000_000) as u64;
    let subsec_nanos = (clamped_nanos % 1_000_000_000) as u32;
    Some(Duration::new(secs, subsec_nanos))
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

fn compute_retry_delay_with_source(
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

/// Computes the retry delay for a 1-based `attempt`, preferring a valid `Retry-After` header
/// and falling back to jittered exponential backoff otherwise.
pub fn compute_retry_delay(
    attempt: u32,
    retry_after_header: Option<&str>,
    now: SystemTime,
    policy: &RetryPolicy,
) -> Duration {
    compute_retry_delay_with_source(attempt, retry_after_header, now, policy).0
}

/// Supported proxy URL schemes for environment-based proxy configuration.
const ALLOWED_PROXY_SCHEMES: &[&str] = &["http", "https", "socks4", "socks4a", "socks5", "socks5h"];

/// Validated HTTP, HTTPS, SOCKS, and bypass proxy configuration resolved from environment variables.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProxyConfig {
    pub http_proxy: Option<Url>,
    pub https_proxy: Option<Url>,
    pub all_proxy: Option<Url>,
    pub no_proxy: Option<String>,
}

fn is_rfc3986_scheme(scheme: &str) -> bool {
    let mut bytes = scheme.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    first.is_ascii_alphabetic()
        && bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
}

/// Redacts embedded credentials (`userinfo@`) from a proxy URL before including it in errors.
fn redact_proxy_value(raw: &str) -> String {
    let trimmed = raw.trim();
    let (scheme_prefix, rest) = match trimmed.find("://") {
        Some(idx) if is_rfc3986_scheme(&trimmed[..idx]) => trimmed.split_at(idx + 3),
        _ => ("", trimmed),
    };

    if let Some(at_idx) = rest.rfind('@') {
        format!("{scheme_prefix}<redacted>@{}", &rest[at_idx + 1..])
    } else {
        trimmed.to_string()
    }
}

fn invalid_proxy(var: &str, raw: &str, reason: impl Into<String>) -> IaGetError {
    IaGetError::InvalidProxy {
        var: var.to_string(),
        value: redact_proxy_value(raw),
        reason: reason.into(),
    }
}

fn validate_parsed_proxy_url(var: &str, raw_for_error: &str, url: &Url) -> Result<()> {
    let scheme = url.scheme();
    if !ALLOWED_PROXY_SCHEMES.contains(&scheme) {
        return Err(invalid_proxy(
            var,
            raw_for_error,
            format!(
                "unsupported scheme '{scheme}'; expected one of http, https, socks4, socks4a, socks5, socks5h"
            ),
        ));
    }

    if url.host_str().is_none_or(str::is_empty) {
        return Err(invalid_proxy(
            var,
            raw_for_error,
            "proxy URL must include a host",
        ));
    }

    if url.port() == Some(0) {
        return Err(invalid_proxy(
            var,
            raw_for_error,
            "proxy port must not be 0",
        ));
    }

    Ok(())
}

fn parse_proxy_url(var: &str, raw: &str) -> Result<Option<Url>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    if trimmed.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(invalid_proxy(
            var,
            trimmed,
            "proxy URL must not contain whitespace or control characters",
        ));
    }

    if !trimmed.contains("://") {
        return Err(invalid_proxy(
            var,
            trimmed,
            "proxy URL must include a scheme followed by '://'",
        ));
    }

    let url = Url::parse(trimmed)
        .map_err(|err| invalid_proxy(var, trimmed, format!("invalid URL ({err})")))?;

    if !trimmed
        .get(url.scheme().len()..)
        .is_some_and(|rest| rest.starts_with("://"))
    {
        return Err(invalid_proxy(
            var,
            trimmed,
            "proxy URL must include '://' immediately after the scheme",
        ));
    }

    validate_parsed_proxy_url(var, trimmed, &url)?;
    Ok(Some(url))
}

fn lookup_os_pair<F>(lookup: &F, lower: &str, upper: &str) -> Result<Option<(String, String)>>
where
    F: Fn(&str) -> Option<OsString>,
{
    let (var, raw) = if let Some(val) = lookup(lower) {
        (lower, val)
    } else if let Some(val) = lookup(upper) {
        (upper, val)
    } else {
        return Ok(None);
    };

    let value = raw.into_string().map_err(|_| IaGetError::InvalidProxy {
        var: var.to_string(),
        value: "<non-UTF-8>".to_string(),
        reason: "environment variable value is not valid UTF-8".to_string(),
    })?;

    Ok(Some((var.to_string(), value)))
}

impl ProxyConfig {
    /// Resolves and validates proxy configuration from a string lookup function.
    pub fn from_lookup<F>(lookup: F) -> Result<Self>
    where
        F: Fn(&str) -> Option<String>,
    {
        Self::from_os_lookup(|name| lookup(name).map(OsString::from))
    }

    /// Resolves and validates proxy configuration from an OS-string lookup function.
    pub fn from_os_lookup<F>(lookup: F) -> Result<Self>
    where
        F: Fn(&str) -> Option<OsString>,
    {
        let http_proxy = lookup_os_pair(&lookup, "http_proxy", "HTTP_PROXY")?
            .map(|(var, value)| parse_proxy_url(&var, &value))
            .transpose()?
            .flatten();

        let https_proxy = lookup_os_pair(&lookup, "https_proxy", "HTTPS_PROXY")?
            .map(|(var, value)| parse_proxy_url(&var, &value))
            .transpose()?
            .flatten();

        let all_proxy = lookup_os_pair(&lookup, "all_proxy", "ALL_PROXY")?
            .map(|(var, value)| parse_proxy_url(&var, &value))
            .transpose()?
            .flatten();

        let no_proxy = lookup_os_pair(&lookup, "no_proxy", "NO_PROXY")?.and_then(|(_, value)| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        });

        Ok(Self {
            http_proxy,
            https_proxy,
            all_proxy,
            no_proxy,
        })
    }

    /// Resolves and validates proxy configuration from the current process environment.
    pub fn from_env() -> Result<Self> {
        Self::from_os_lookup(|name| std::env::var_os(name))
    }
}

/// Returns a [`reqwest::ClientBuilder`] with automatic redirect following disabled
/// so redirects are handled manually with per-hop cookie resolution.
pub fn http_client_builder() -> ClientBuilder {
    Client::builder().redirect(reqwest::redirect::Policy::none())
}

fn build_reqwest_proxy(
    var: &str,
    url: &Url,
    no_proxy: Option<reqwest::NoProxy>,
    ctor: fn(Url) -> std::result::Result<reqwest::Proxy, reqwest::Error>,
) -> Result<reqwest::Proxy> {
    validate_parsed_proxy_url(var, url.as_str(), url)?;
    let proxy = ctor(url.clone())
        .map_err(|err| invalid_proxy(var, url.as_str(), err.without_url().to_string()))?;
    Ok(proxy.no_proxy(no_proxy))
}

/// Returns a [`reqwest::ClientBuilder`] with automatic redirect following disabled
/// and explicit proxy rules configured from `proxy_config`.
pub fn http_client_builder_with_proxy(proxy_config: &ProxyConfig) -> Result<ClientBuilder> {
    let mut builder = http_client_builder().no_proxy();
    let no_proxy = proxy_config
        .no_proxy
        .as_deref()
        .and_then(reqwest::NoProxy::from_string);

    if let Some(url) = &proxy_config.http_proxy {
        builder = builder.proxy(build_reqwest_proxy(
            "HTTP_PROXY",
            url,
            no_proxy.clone(),
            reqwest::Proxy::http,
        )?);
    }

    if let Some(url) = &proxy_config.https_proxy {
        builder = builder.proxy(build_reqwest_proxy(
            "HTTPS_PROXY",
            url,
            no_proxy.clone(),
            reqwest::Proxy::https,
        )?);
    }

    if let Some(url) = &proxy_config.all_proxy {
        builder = builder.proxy(build_reqwest_proxy(
            "ALL_PROXY",
            url,
            no_proxy,
            reqwest::Proxy::all,
        )?);
    }

    Ok(builder)
}

/// Returns a [`reqwest::ClientBuilder`] with automatic redirect following disabled
/// and proxy configuration loaded from the current process environment.
pub fn http_client_builder_from_env() -> Result<ClientBuilder> {
    let proxy_config = ProxyConfig::from_env()?;
    http_client_builder_with_proxy(&proxy_config)
}

fn is_redirect_status(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::MOVED_PERMANENTLY
            | StatusCode::FOUND
            | StatusCode::SEE_OTHER
            | StatusCode::TEMPORARY_REDIRECT
            | StatusCode::PERMANENT_REDIRECT
    )
}

fn resolve_redirect_target(current_url: &Url, location: &str) -> Result<Url> {
    let safe_current = sanitize_url_for_log(current_url);
    let location = location.trim();
    if location.is_empty() {
        return Err(IaGetError::Network(format!(
            "Redirect from {} has empty Location header",
            safe_current
        )));
    }

    let next_url = current_url.join(location).map_err(|err| {
        IaGetError::Network(format!(
            "Invalid redirect Location '{}' from {}: {}",
            redact_proxy_value(location),
            safe_current,
            err
        ))
    })?;
    let safe_next = sanitize_url_for_log(&next_url);

    if !matches!(next_url.scheme(), "http" | "https") {
        return Err(IaGetError::Network(format!(
            "Unsupported redirect scheme '{}' in Location '{}' from {}",
            next_url.scheme(),
            safe_next,
            safe_current
        )));
    }

    if current_url.scheme() == "https" && next_url.scheme() != "https" {
        return Err(IaGetError::Network(format!(
            "Refusing HTTPS-to-{} redirect downgrade from {} to {}",
            next_url.scheme(),
            safe_current,
            safe_next
        )));
    }

    Ok(next_url)
}

enum RedirectSendOutcome {
    Response(Response),
    TransportError(reqwest::Error),
}

async fn send_following_redirects<F>(
    initial_url: &Url,
    signal: &ShutdownSignal,
    max_redirects: usize,
    logger: &VerboseLogger,
    mut build_request: F,
) -> Result<RedirectSendOutcome>
where
    F: FnMut(&Url) -> Result<RequestBuilder>,
{
    let mut current_url = initial_url.clone();
    let mut redirects_followed: usize = 0;
    let mut visited = HashSet::new();
    let mut normalised_initial = current_url.clone();
    normalised_initial.set_fragment(None);
    visited.insert(normalised_initial);

    loop {
        let (client, request_result) = build_request(&current_url)?.build_split();
        let request = request_result.map_err(reqwest::Error::without_url)?;
        let method = request.method().clone();
        let cookies_label = if request.headers().contains_key(reqwest::header::COOKIE) {
            "yes"
        } else {
            "no"
        };
        let range = request
            .headers()
            .get(reqwest::header::RANGE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let safe_url = sanitize_url_for_log(request.url());

        if let Some(ref range_value) = range {
            logger.log(
                "http",
                format_args!(
                    "-> {method} {safe_url} (cookies={cookies_label}, range={range_value})"
                ),
            );
        } else {
            logger.log(
                "http",
                format_args!("-> {method} {safe_url} (cookies={cookies_label})"),
            );
        }

        let response = match await_interruptible(client.execute(request), signal).await? {
            Ok(response) => {
                logger.log(
                    "http",
                    format_args!(
                        "<- {} {method} {safe_url} (cookies={cookies_label})",
                        response.status()
                    ),
                );
                response
            }
            Err(err) => {
                let err = err.without_url();
                logger.log(
                    "http",
                    format_args!("<- error {method} {safe_url} (cookies={cookies_label}): {err}"),
                );
                return Ok(RedirectSendOutcome::TransportError(err));
            }
        };

        let status = response.status();
        if !is_redirect_status(status) {
            return Ok(RedirectSendOutcome::Response(response));
        }

        if redirects_followed >= max_redirects {
            logger.log(
                "redirect",
                format_args!("limit exceeded ({max_redirects}) while requesting {safe_url}"),
            );
            return Err(IaGetError::Network(format!(
                "Too many redirects (maximum {} exceeded) while requesting {}",
                max_redirects, safe_url
            )));
        }

        let location_header = match response.headers().get(reqwest::header::LOCATION) {
            Some(header) => header,
            None => {
                logger.log(
                    "redirect",
                    format_args!("{status} {safe_url} missing Location header"),
                );
                return Err(IaGetError::Network(format!(
                    "HTTP {} redirect from {} missing Location header",
                    status, safe_url
                )));
            }
        };

        let location = match location_header.to_str() {
            Ok(location) => location,
            Err(_) => {
                logger.log(
                    "redirect",
                    format_args!("{status} {safe_url} has non-UTF-8 Location header"),
                );
                return Err(IaGetError::Network(format!(
                    "HTTP {} redirect from {} has non-UTF-8 Location header",
                    status, safe_url
                )));
            }
        };

        let next_url = match resolve_redirect_target(&current_url, location) {
            Ok(next_url) => next_url,
            Err(err) => {
                logger.log("redirect", format_args!("error: {err}"));
                return Err(err);
            }
        };
        let next_safe = sanitize_url_for_log(&next_url);

        let mut normalised_next = next_url.clone();
        normalised_next.set_fragment(None);
        if !visited.insert(normalised_next) {
            logger.log(
                "redirect",
                format_args!("loop detected {safe_url} -> {next_safe}"),
            );
            return Err(IaGetError::Network(format!(
                "Redirect loop detected while following redirect to {}",
                next_safe
            )));
        }

        let hop = redirects_followed + 1;
        logger.log(
            "redirect",
            format_args!("{status} {safe_url} -> {next_safe} (hop {hop}/{max_redirects})"),
        );

        drop(response);
        redirects_followed = hop;
        current_url = next_url;
    }
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

/// Sends an HTTP request with retries on transient transport errors and retryable HTTP statuses,
/// following redirects manually per attempt and honouring `Retry-After` headers and `ShutdownSignal`.
pub async fn send_with_retry<F, H>(
    initial_url: &Url,
    policy: &RetryPolicy,
    signal: &ShutdownSignal,
    build_request: F,
    on_retry: H,
) -> Result<Response>
where
    F: FnMut(&Url) -> Result<RequestBuilder>,
    H: FnMut(&RetryEvent<'_>),
{
    send_with_retry_with_logger(
        initial_url,
        policy,
        signal,
        &VerboseLogger::disabled(),
        build_request,
        on_retry,
    )
    .await
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
                retry_count += 1;
                logger.log(
                    "retry",
                    format_args!(
                        "attempt {}/{} for {} after {}; waiting {:.2}s ({})",
                        retry_count,
                        policy.max_retries,
                        sanitize_url_for_log(initial_url),
                        reason,
                        delay.as_secs_f64(),
                        delay_source
                    ),
                );
                on_retry(&RetryEvent {
                    attempt: retry_count,
                    max_retries: policy.max_retries,
                    delay,
                    reason: &reason,
                });
                sleep_interruptible(delay, signal).await?;
            }
        }
    }
}

/// Fetches a text response body with retries on transient transport errors, retryable HTTP
/// statuses, and transient body read errors, following redirects manually per attempt and
/// honouring `Retry-After` and `ShutdownSignal`.
pub async fn fetch_text_with_retry<F, H>(
    initial_url: &Url,
    policy: &RetryPolicy,
    signal: &ShutdownSignal,
    build_request: F,
    on_retry: H,
) -> Result<String>
where
    F: FnMut(&Url) -> Result<RequestBuilder>,
    H: FnMut(&RetryEvent<'_>),
{
    fetch_text_with_retry_with_logger(
        initial_url,
        policy,
        signal,
        &VerboseLogger::disabled(),
        build_request,
        on_retry,
    )
    .await
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
                retry_count += 1;
                logger.log(
                    "retry",
                    format_args!(
                        "attempt {}/{} for {} after {}; waiting {:.2}s ({})",
                        retry_count,
                        policy.max_retries,
                        sanitize_url_for_log(initial_url),
                        reason,
                        delay.as_secs_f64(),
                        delay_source
                    ),
                );
                on_retry(&RetryEvent {
                    attempt: retry_count,
                    max_retries: policy.max_retries,
                    delay,
                    reason: &reason,
                });
                sleep_interruptible(delay, signal).await?;
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
                        retry_count += 1;
                        let (delay, delay_source) = compute_retry_delay_with_source(
                            retry_count,
                            None,
                            SystemTime::now(),
                            policy,
                        );
                        let reason = err.to_string();
                        logger.log(
                            "retry",
                            format_args!(
                                "attempt {}/{} for {} after {}; waiting {:.2}s ({})",
                                retry_count,
                                policy.max_retries,
                                sanitize_url_for_log(initial_url),
                                reason,
                                delay.as_secs_f64(),
                                delay_source
                            ),
                        );
                        on_retry(&RetryEvent {
                            attempt: retry_count,
                            max_retries: policy.max_retries,
                            delay,
                            reason: &reason,
                        });
                        sleep_interruptible(delay, signal).await?;
                    }
                }
            }
        }
    }
}

/// Parsed RFC 9110 Section 14.4 `Content-Range` header value for the `bytes` unit.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ContentRange {
    Bytes {
        start: u64,
        end: u64,
        complete_length: Option<u64>,
    },
    Unsatisfied {
        complete_length: u64,
    },
}

/// Action to take for an HTTP download response given the current local offset.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ResponseAction {
    WriteFromStart { total_expected_size: u64 },
    AppendFromOffset { total_expected_size: u64 },
    VerifyCompletePart,
    DiscardAndRefetchUnranged,
}

fn parse_ascii_u64(input: &str) -> Option<u64> {
    if input.is_empty() || !input.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    input.parse::<u64>().ok()
}

/// Parses an RFC 9110 Section 14.4 `Content-Range` header value for the `bytes` unit.
fn parse_content_range(header: &str) -> Option<ContentRange> {
    let trimmed = header.trim();
    let (unit, range_spec) = trimmed.split_once(' ')?;
    if !unit.eq_ignore_ascii_case("bytes") {
        return None;
    }

    let (range_part, total_part) = range_spec.split_once('/')?;

    if range_part == "*" {
        let complete_length = parse_ascii_u64(total_part)?;
        return Some(ContentRange::Unsatisfied { complete_length });
    }

    let (start_str, end_str) = range_part.split_once('-')?;
    let start = parse_ascii_u64(start_str)?;
    let end = parse_ascii_u64(end_str)?;

    if start > end {
        return None;
    }

    let complete_length = if total_part == "*" {
        None
    } else {
        let total = parse_ascii_u64(total_part)?;
        if end >= total {
            return None;
        }
        Some(total)
    };

    Some(ContentRange::Bytes {
        start,
        end,
        complete_length,
    })
}

/// Computes the total expected byte count for progress reporting without overflowing.
fn compute_progress_total(
    current_offset: u64,
    content_length: Option<u64>,
    complete_length: Option<u64>,
) -> u64 {
    complete_length.unwrap_or_else(|| current_offset.saturating_add(content_length.unwrap_or(0)))
}

/// Classifies an HTTP response for a fresh or resumed download attempt.
fn classify_download_response(
    status: StatusCode,
    content_range_header: Option<&str>,
    content_length: Option<u64>,
    current_offset: u64,
    has_expected_md5: bool,
) -> Result<ResponseAction> {
    match status {
        StatusCode::OK => Ok(ResponseAction::WriteFromStart {
            total_expected_size: compute_progress_total(0, content_length, None),
        }),
        StatusCode::PARTIAL_CONTENT => {
            if current_offset == 0 {
                return Err(IaGetError::Network(format!(
                    "HTTP error {}: unexpected partial content for unranged request",
                    status
                )));
            }

            match content_range_header.and_then(parse_content_range) {
                Some(ContentRange::Bytes {
                    start,
                    end: _,
                    complete_length,
                }) if start == current_offset => Ok(ResponseAction::AppendFromOffset {
                    total_expected_size: compute_progress_total(
                        current_offset,
                        content_length,
                        complete_length,
                    ),
                }),
                _ => Ok(ResponseAction::DiscardAndRefetchUnranged),
            }
        }
        StatusCode::RANGE_NOT_SATISFIABLE => {
            if current_offset == 0 {
                return Err(IaGetError::Network(format!(
                    "HTTP error {}: unexpected range not satisfiable for unranged request",
                    status
                )));
            }

            match content_range_header.and_then(parse_content_range) {
                Some(ContentRange::Unsatisfied { complete_length }) => {
                    if complete_length == current_offset {
                        Ok(ResponseAction::VerifyCompletePart)
                    } else {
                        Ok(ResponseAction::DiscardAndRefetchUnranged)
                    }
                }
                Some(ContentRange::Bytes { .. }) => Ok(ResponseAction::DiscardAndRefetchUnranged),
                None => {
                    if has_expected_md5 {
                        Ok(ResponseAction::VerifyCompletePart)
                    } else {
                        Ok(ResponseAction::DiscardAndRefetchUnranged)
                    }
                }
            }
        }
        _ => Err(IaGetError::Network(format!("HTTP error {}", status))),
    }
}

/// Canonicalises the destination root directory and verifies that it is a directory.
pub(crate) fn resolve_destination_root(root: &Path) -> Result<PathBuf> {
    let canonical = fs::canonicalize(root)?;
    if !fs::metadata(&canonical)?.is_dir() {
        return Err(IaGetError::FileSystem(format!(
            "Destination root '{}' is not a directory",
            root.display()
        )));
    }
    Ok(canonical)
}

/// Validates and prepares the destination output directory, creating missing directories
/// when needed and returning its canonical path.
pub fn prepare_output_dir(path: &Path) -> Result<PathBuf> {
    if path.as_os_str().is_empty() {
        return Err(IaGetError::FileSystem(
            "Output directory path must not be empty".to_string(),
        ));
    }

    match fs::metadata(path) {
        Ok(meta) if meta.is_dir() => resolve_destination_root(path),
        Ok(_) => Err(IaGetError::FileSystem(format!(
            "Output path '{}' exists and is not a directory",
            path.display()
        ))),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            if fs::symlink_metadata(path).is_ok() {
                return Err(IaGetError::FileSystem(format!(
                    "Output path '{}' is a broken symlink",
                    path.display()
                )));
            }

            fs::create_dir_all(path).map_err(|create_err| {
                IaGetError::FileSystem(format!(
                    "Failed to create output directory '{}': {}",
                    path.display(),
                    create_err
                ))
            })?;

            resolve_destination_root(path)
        }
        Err(err) => Err(IaGetError::FileSystem(format!(
            "Failed to access output directory '{}': {}",
            path.display(),
            err
        ))),
    }
}

/// Validates that a destination path is a non-empty relative path made solely of normal segments.
pub(crate) fn validate_relative_path(path: &Path) -> Result<PathBuf> {
    if path.as_os_str().is_empty() {
        return Err(IaGetError::FileSystem(
            "Destination path must not be empty".to_string(),
        ));
    }

    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(segment) => clean.push(segment),
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => {
                return Err(IaGetError::FileSystem(format!(
                    "Refusing unsafe destination path '{}'",
                    path.display()
                )));
            }
        }
    }

    if clean.as_os_str().is_empty() {
        return Err(IaGetError::FileSystem(format!(
            "Destination path '{}' has no filename component",
            path.display()
        )));
    }

    Ok(clean)
}

/// Validates that `rel_path` stays inside `canonical_root` with no symlinks or non-regular leaves.
pub(crate) fn validate_contained_path(
    canonical_root: &Path,
    rel_path: &Path,
    allow_missing_leaf: bool,
) -> Result<PathBuf> {
    let clean_rel = validate_relative_path(rel_path)?;
    let segments: Vec<&OsStr> = clean_rel
        .components()
        .map(|component| match component {
            Component::Normal(segment) => segment,
            _ => unreachable!("validate_relative_path permits only normal components"),
        })
        .collect();

    let (parents, leaf) = segments.split_at(segments.len() - 1);
    let leaf = leaf[0];

    let mut current = canonical_root.to_path_buf();
    for (index, segment) in parents.iter().enumerate() {
        current.push(segment);
        match fs::symlink_metadata(&current) {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    return Err(IaGetError::FileSystem(format!(
                        "Refusing symlinked directory component '{}'",
                        current.display()
                    )));
                }
                if !meta.file_type().is_dir() {
                    return Err(IaGetError::FileSystem(format!(
                        "Expected directory at '{}', found non-directory",
                        current.display()
                    )));
                }
                let canonical_current = fs::canonicalize(&current)?;
                if !canonical_current.starts_with(canonical_root) {
                    return Err(IaGetError::FileSystem(format!(
                        "Path '{}' escapes destination root '{}'",
                        current.display(),
                        canonical_root.display()
                    )));
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                if allow_missing_leaf {
                    for remaining in &parents[index + 1..] {
                        current.push(remaining);
                    }
                    current.push(leaf);
                    return Ok(current);
                }
                return Err(err.into());
            }
            Err(err) => return Err(err.into()),
        }
    }

    current.push(leaf);
    match fs::symlink_metadata(&current) {
        Ok(meta) => {
            if meta.file_type().is_symlink() {
                return Err(IaGetError::FileSystem(format!(
                    "Refusing symlink target '{}'",
                    current.display()
                )));
            }
            if !meta.file_type().is_file() {
                return Err(IaGetError::FileSystem(format!(
                    "Refusing non-regular file target '{}'",
                    current.display()
                )));
            }
            let canonical_leaf = fs::canonicalize(&current)?;
            if !canonical_leaf.starts_with(canonical_root) {
                return Err(IaGetError::FileSystem(format!(
                    "Target '{}' escapes destination root '{}'",
                    current.display(),
                    canonical_root.display()
                )));
            }
            Ok(current)
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            if allow_missing_leaf {
                Ok(current)
            } else {
                Err(err.into())
            }
        }
        Err(err) => Err(err.into()),
    }
}

/// Returns the platform-specific `O_NOFOLLOW` open flag on supported Unix targets.
#[cfg(unix)]
const fn unix_open_nofollow_flag() -> Option<i32> {
    #[cfg(all(
        target_os = "linux",
        any(
            target_arch = "x86_64",
            target_arch = "x86",
            target_arch = "riscv64",
            target_arch = "s390x",
        )
    ))]
    {
        Some(0o0400000)
    }
    #[cfg(all(target_os = "linux", any(target_arch = "aarch64", target_arch = "arm")))]
    {
        Some(0o0100000)
    }
    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly",
    ))]
    {
        Some(0x0100)
    }
    #[cfg(not(any(
        all(
            target_os = "linux",
            any(
                target_arch = "x86_64",
                target_arch = "x86",
                target_arch = "riscv64",
                target_arch = "s390x",
                target_arch = "aarch64",
                target_arch = "arm",
            )
        ),
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly",
    )))]
    {
        None
    }
}

/// Applies `O_NOFOLLOW` to `OpenOptions` on supported Unix targets.
fn configure_no_follow(options: &mut fs::OpenOptions) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        if let Some(flag) = unix_open_nofollow_flag() {
            options.custom_flags(flag);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = options;
    }
}

/// Opens a verified regular file inside `root` for reading without following symlinks.
pub(crate) fn open_regular_file_for_read(root: &Path, rel_path: &Path) -> Result<File> {
    let canonical_root = resolve_destination_root(root)?;
    let target_path = validate_contained_path(&canonical_root, rel_path, false)?;

    let mut options = fs::OpenOptions::new();
    options.read(true);
    configure_no_follow(&mut options);

    let file = options.open(&target_path)?;
    if !file.metadata()?.file_type().is_file() {
        return Err(IaGetError::FileSystem(format!(
            "Refusing non-regular file '{}'",
            target_path.display()
        )));
    }

    Ok(file)
}

/// Safely removes a contained regular `.part` file inside `root`, ignoring `NotFound`.
fn remove_part_file_safely(root: &Path, rel_part_path: &Path) -> Result<()> {
    let canonical_root = resolve_destination_root(root)?;
    let part_path = validate_contained_path(&canonical_root, rel_part_path, true)?;
    match fs::remove_file(&part_path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Builds the temporary `.part` path for a target file without replacing existing extensions.
pub(crate) fn part_file_path(file_path: &str) -> PathBuf {
    let mut part_path = OsString::from(file_path);
    part_path.push(".part");
    PathBuf::from(part_path)
}

/// Counter for generating unique temporary probe filenames during case-sensitivity detection.
static CASE_PROBE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Maximum directory entries inspected during read-only case-sensitivity probing.
const MAX_READ_ONLY_CASE_PROBE_ENTRIES: usize = 32;

/// Maximum attempts to create a unique temporary probe file before falling back.
const MAX_TEMP_CASE_PROBE_ATTEMPTS: usize = 16;

/// Case-sensitivity rule used when checking planned download paths for collisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathCaseSensitivity {
    /// Treat paths differing only in letter case as distinct (for example ext4, tmpfs, or
    /// case-sensitive APFS).
    CaseSensitive,
    /// Treat paths differing only in letter case as colliding (for example default APFS, NTFS,
    /// FAT32, or exFAT).
    CaseInsensitive,
}

/// Guard that removes a temporary filesystem probe file when dropped.
struct ProbeFileGuard(PathBuf);

impl Drop for ProbeFileGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Returns the default filesystem case sensitivity for the current target platform when a
/// directory cannot be probed directly.
const fn default_platform_case_sensitivity() -> PathCaseSensitivity {
    #[cfg(any(target_os = "macos", target_os = "ios", target_os = "windows"))]
    {
        PathCaseSensitivity::CaseInsensitive
    }
    #[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "windows")))]
    {
        PathCaseSensitivity::CaseSensitive
    }
}

/// Resolves `dir` (or its nearest existing ancestor directory) for filesystem probing.
fn resolve_existing_probe_dir(dir: &Path) -> Option<PathBuf> {
    if dir.as_os_str().is_empty() {
        return None;
    }

    let candidate = if dir.is_absolute() {
        dir.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(dir)
    };

    for ancestor in candidate.ancestors() {
        if ancestor.as_os_str().is_empty() {
            continue;
        }
        if fs::metadata(ancestor)
            .map(|meta| meta.is_dir())
            .unwrap_or(false)
        {
            return Some(ancestor.to_path_buf());
        }
    }

    None
}

/// Flips the ASCII letter case of `name`, returning `None` if `name` contains no ASCII letters.
fn toggle_ascii_case(name: &str) -> Option<String> {
    if !name.bytes().any(|b| b.is_ascii_alphabetic()) {
        return None;
    }

    let toggled: String = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_lowercase() {
                ch.to_ascii_uppercase()
            } else if ch.is_ascii_uppercase() {
                ch.to_ascii_lowercase()
            } else {
                ch
            }
        })
        .collect();
    Some(toggled)
}

/// Probes case sensitivity inside `probe_dir` without writing by inspecting existing entries.
fn probe_existing_child_case_sensitivity(probe_dir: &Path) -> Option<PathCaseSensitivity> {
    let entries = fs::read_dir(probe_dir).ok()?;

    for entry in entries.take(MAX_READ_ONLY_CASE_PROBE_ENTRIES) {
        let Ok(entry) = entry else {
            continue;
        };
        let file_name = entry.file_name();
        let Some(name_str) = file_name.to_str() else {
            continue;
        };
        let Some(toggled) = toggle_ascii_case(name_str) else {
            continue;
        };

        let toggled_path = probe_dir.join(&toggled);
        match fs::symlink_metadata(&toggled_path) {
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                // Re-verify the original entry still exists so a concurrent deletion in a shared
                // directory cannot be misclassified as case-sensitive.
                if fs::symlink_metadata(probe_dir.join(&file_name)).is_ok() {
                    return Some(PathCaseSensitivity::CaseSensitive);
                }
            }
            Ok(toggled_meta) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    if let Ok(orig_meta) = entry.metadata() {
                        if orig_meta.dev() == toggled_meta.dev()
                            && orig_meta.ino() == toggled_meta.ino()
                            && (orig_meta.is_dir() || orig_meta.nlink() == 1)
                        {
                            return Some(PathCaseSensitivity::CaseInsensitive);
                        }
                    }
                }
                #[cfg(not(unix))]
                {
                    let _ = toggled_meta;
                }
            }
            Err(_) => continue,
        }
    }

    None
}

/// Probes case sensitivity inside `probe_dir` by creating and immediately removing a unique
/// zero-byte file with `create_new(true)`.
fn probe_temp_file_case_sensitivity(probe_dir: &Path) -> Option<PathCaseSensitivity> {
    let pid = std::process::id();

    for _ in 0..MAX_TEMP_CASE_PROBE_ATTEMPTS {
        let seq = CASE_PROBE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let lower_path = probe_dir.join(format!(".ia-get-case-probe-{pid}-{seq}-a"));
        let upper_path = probe_dir.join(format!(".ia-get-case-probe-{pid}-{seq}-A"));

        match fs::symlink_metadata(&upper_path) {
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Ok(_) => continue,
            Err(_) => return None,
        }

        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        configure_no_follow(&mut options);

        let file = match options.open(&lower_path) {
            Ok(file) => file,
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return None,
        };
        let _guard = ProbeFileGuard(lower_path);
        drop(file);

        return match fs::symlink_metadata(&upper_path) {
            Ok(_) => Some(PathCaseSensitivity::CaseInsensitive),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                Some(PathCaseSensitivity::CaseSensitive)
            }
            Err(_) => None,
        };
    }

    None
}

/// Detects whether the filesystem hosting `dir` (or its nearest existing ancestor directory when
/// `dir` does not exist yet) treats filenames as case-sensitive or case-insensitive.
///
/// Tries a read-only probe of existing entries inside the directory first, then a temporary
/// probe file with guaranteed cleanup, and falls back to the platform default when the directory
/// is unreadable or unwritable.
pub fn detect_path_case_sensitivity(dir: &Path) -> PathCaseSensitivity {
    let Some(probe_dir) = resolve_existing_probe_dir(dir) else {
        return default_platform_case_sensitivity();
    };

    if let Some(sensitivity) = probe_existing_child_case_sensitivity(&probe_dir) {
        return sensitivity;
    }

    if let Some(sensitivity) = probe_temp_file_case_sensitivity(&probe_dir) {
        return sensitivity;
    }

    default_platform_case_sensitivity()
}

/// Normalises a relative destination path into a collision-detection key.
fn collision_path_key(path: &Path) -> PathBuf {
    validate_relative_path(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Formats a normalised relative collision path using `/` separators for consistent diagnostics.
fn format_collision_path(path: &Path) -> String {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(segment) => parts.push(segment.to_string_lossy()),
            _ => return path.display().to_string(),
        }
    }
    if parts.is_empty() {
        path.display().to_string()
    } else {
        parts.join("/")
    }
}

/// Normalises a relative destination path into a Unicode case-folded key for case-insensitive
/// collision detection.
fn case_folded_collision_key(path: &Path) -> String {
    format_collision_path(path).to_lowercase()
}

/// Checks planned `(entry_label, local_path)` pairs for final and `.part` staging collisions
/// using `make_key` to derive comparison keys while preserving exact display paths in errors.
fn validate_collision_keys<K, F>(
    entries: &[(&str, &str)],
    mut make_key: F,
    reason_suffix: &str,
) -> Result<()>
where
    K: Eq + Hash,
    F: FnMut(&Path) -> K,
{
    let mut final_paths: HashMap<K, String> = HashMap::new();
    let mut staging_paths: HashMap<K, String> = HashMap::new();

    for &(entry_label, local_path) in entries {
        let clean_final = collision_path_key(Path::new(local_path));
        let part_path = part_file_path(local_path);
        let clean_staging = collision_path_key(&part_path);

        let final_key = make_key(&clean_final);
        let staging_key = make_key(&clean_staging);

        if let Some(first_entry) = final_paths.get(&final_key) {
            return Err(IaGetError::PathCollision {
                local_path: format_collision_path(&clean_final),
                first_entry: first_entry.clone(),
                second_entry: entry_label.to_string(),
                reason: format!("both entries map to the same destination path{reason_suffix}"),
            });
        }

        if let Some(first_entry) = staging_paths.get(&final_key) {
            return Err(IaGetError::PathCollision {
                local_path: format_collision_path(&clean_final),
                first_entry: first_entry.clone(),
                second_entry: entry_label.to_string(),
                reason: format!(
                    "destination path collides with staging '.part' path{reason_suffix}"
                ),
            });
        }

        if let Some(first_entry) = final_paths.get(&staging_key) {
            return Err(IaGetError::PathCollision {
                local_path: format_collision_path(&clean_staging),
                first_entry: first_entry.clone(),
                second_entry: entry_label.to_string(),
                reason: format!(
                    "staging '.part' path collides with destination path{reason_suffix}"
                ),
            });
        }

        if let Some(first_entry) = staging_paths.get(&staging_key) {
            return Err(IaGetError::PathCollision {
                local_path: format_collision_path(&clean_staging),
                first_entry: first_entry.clone(),
                second_entry: entry_label.to_string(),
                reason: format!("both entries map to the same staging '.part' path{reason_suffix}"),
            });
        }

        final_paths.insert(final_key, entry_label.to_string());
        staging_paths.insert(staging_key, entry_label.to_string());
    }

    Ok(())
}

/// Validates that no two planned entries collide on either a final destination path
/// or a `.part` staging path under an explicit [`PathCaseSensitivity`] rule.
///
/// Each item is `(entry_label, local_path)`, where `entry_label` identifies the source
/// archive entry for error reporting and `local_path` is the local destination path.
pub fn validate_download_paths_with_case_sensitivity<'a, I>(
    entries: I,
    case_sensitivity: PathCaseSensitivity,
) -> Result<()>
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    let entries: Vec<(&'a str, &'a str)> = entries.into_iter().collect();
    validate_collision_keys(&entries, Path::to_path_buf, "")?;

    if case_sensitivity == PathCaseSensitivity::CaseInsensitive {
        validate_collision_keys(
            &entries,
            case_folded_collision_key,
            " on a case-insensitive filesystem",
        )?;
    }

    Ok(())
}

/// Guard that removes a temporary filesystem probe directory and its contents when dropped.
struct ProbeDirGuard(PathBuf);

impl Drop for ProbeDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Guard that removes every temporary probe file created directly inside a directory when dropped.
#[derive(Default)]
struct ProbeFilesGuard(Vec<PathBuf>);

impl ProbeFilesGuard {
    fn push(&mut self, path: PathBuf) {
        self.0.push(path);
    }
}

impl Drop for ProbeFilesGuard {
    fn drop(&mut self) {
        for path in self.0.drain(..).rev() {
            let _ = fs::remove_file(path);
        }
    }
}

/// Resolved destination root state used for per-parent-directory filesystem lookup checks.
#[derive(Debug, Clone)]
struct ProbeRoot {
    /// Existing directory whose filesystem lookup rules apply to the output root.
    probe_dir: Option<PathBuf>,
    /// Canonical path of the output root when it already exists as a directory on disk.
    /// Remains `None` when the output root does not exist yet so missing children are never
    /// probed against unrelated sibling directories in an existing ancestor.
    exact_on_disk_dir: Option<PathBuf>,
}

/// Result of resolving filename equivalence for distinct component names inside one directory.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DirEquivalenceResult {
    /// For each component index `i`, the smallest index `groups[i] <= i` whose name resolves to
    /// the same directory entry under that directory's filesystem lookup rules.
    groups: Vec<usize>,
    /// Reason suffix describing the lookup rule that detected a non-exact collision.
    reason_suffix: &'static str,
    /// When `Some`, `groups` came from a partial read-only case-folding check that could not
    /// verify every candidate name. If `check_level_groups` does not report a
    /// [`IaGetError::PathCollision`], validation must fail closed with this error message.
    unverified_error: Option<String>,
}

/// Abstraction over per-directory filesystem lookup rules so [`validate_download_paths_for_dir`]
/// probes real host directories while unit tests deterministically exercise mixed-mount and
/// Unicode-normalisation directory trees.
trait DirectoryLookup {
    /// Resolves the output root for probing without creating missing directories.
    fn resolve_root(&self, dir: &Path) -> ProbeRoot;

    /// Resolves an existing, non-symlink child directory inside `parent_on_disk` for `segment`,
    /// returning `None` if it does not exist, is a symlink, or escapes `canonical_root`.
    fn resolve_child_dir(
        &self,
        canonical_root: Option<&Path>,
        parent_on_disk: &Path,
        segment: &str,
    ) -> Option<PathBuf>;

    /// Partitions `names` (distinct component names at a single directory level) into equivalence
    /// classes under `probe_dir`'s filesystem lookup rules, failing closed when distinctness
    /// cannot be verified.
    fn resolve_equivalences(
        &self,
        probe_dir: Option<&Path>,
        names: &[String],
        candidates: &[usize],
    ) -> Result<DirEquivalenceResult>;
}

/// Production [`DirectoryLookup`] backed by the host OS filesystem.
struct OsDirectoryLookup {
    allow_temp_subdir_probe: bool,
}

impl OsDirectoryLookup {
    const PRODUCTION: Self = Self {
        allow_temp_subdir_probe: true,
    };

    #[cfg(test)]
    const fn without_temp_subdir_probe() -> Self {
        Self {
            allow_temp_subdir_probe: false,
        }
    }
}

/// Resolves `dir` into its existing probe directory and, when `dir` itself exists as a directory,
/// its canonical on-disk path.
fn resolve_probe_root(dir: &Path) -> ProbeRoot {
    if dir.as_os_str().is_empty() {
        return ProbeRoot {
            probe_dir: None,
            exact_on_disk_dir: None,
        };
    }

    if fs::metadata(dir).map(|meta| meta.is_dir()).unwrap_or(false) {
        let resolved = fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
        return ProbeRoot {
            probe_dir: Some(resolved.clone()),
            exact_on_disk_dir: Some(resolved),
        };
    }

    let probe_dir = resolve_existing_probe_dir(dir)
        .map(|ancestor| fs::canonicalize(&ancestor).unwrap_or(ancestor));
    ProbeRoot {
        probe_dir,
        exact_on_disk_dir: None,
    }
}

/// Resolves `parent_dir.join(segment)` only when it is an existing, non-symlink directory
/// contained within `canonical_root`.
fn resolve_contained_child_dir(
    canonical_root: Option<&Path>,
    parent_dir: &Path,
    segment: &str,
) -> Option<PathBuf> {
    let child = parent_dir.join(segment);
    let meta = fs::symlink_metadata(&child).ok()?;
    if meta.file_type().is_symlink() || !meta.file_type().is_dir() {
        return None;
    }
    if let Some(root) = canonical_root {
        let canonical_child = fs::canonicalize(&child).ok()?;
        if !canonical_child.starts_with(root) {
            return None;
        }
    }
    Some(child)
}

/// Creates a private empty temporary probe directory inside `parent` with RAII cleanup.
fn create_temp_probe_dir(parent: &Path) -> std::io::Result<ProbeDirGuard> {
    let pid = std::process::id();
    for _ in 0..MAX_TEMP_CASE_PROBE_ATTEMPTS {
        let seq = CASE_PROBE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let candidate = parent.join(format!(".ia-get-fs-probe-{pid}-{seq}"));
        match fs::create_dir(&candidate) {
            Ok(()) => return Ok(ProbeDirGuard(candidate)),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "exhausted temporary probe directory attempts",
    ))
}

/// Returns `true` if `name` is a Windows reserved DOS device name that must not be opened during
/// filesystem probing.
#[cfg(windows)]
fn is_windows_reserved_probe_name(name: &str) -> bool {
    let base = name.split('.').next().unwrap_or(name);
    let trimmed = base.trim_end_matches([' ', '.']);
    matches!(
        trimmed.to_ascii_uppercase().as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

/// Selects indices in `names` that could collide under non-exact filesystem lookup rules.
///
/// Returns an empty vector when all component names are pure ASCII and have pairwise distinct
/// ASCII-lowercased forms (preserving a zero-I/O fast path).
fn select_probe_candidates(names: &[String]) -> Vec<usize> {
    if names.len() < 2 {
        return Vec::new();
    }

    if names.iter().any(|name| !name.is_ascii()) {
        return (0..names.len()).collect();
    }

    let mut counts: HashMap<String, usize> = HashMap::with_capacity(names.len());
    let keys: Vec<String> = names
        .iter()
        .map(|name| {
            let key = name.trim_end_matches(['.', ' ']).to_ascii_lowercase();
            *counts.entry(key.clone()).or_insert(0) += 1;
            key
        })
        .collect();

    keys.into_iter()
        .enumerate()
        .filter_map(|(idx, key)| (counts.get(&key).copied().unwrap_or(0) > 1).then_some(idx))
        .collect()
}

/// Maximum bytes read from a prefixed probe file token (`"<pid>:<seq>:<idx>"`).
const MAX_PROBE_TOKEN_BYTES: u64 = 128;

/// Returns `true` when `name` is a single normal path component safe to probe on the host OS.
fn is_probable_component_name(name: &str) -> bool {
    let mut comps = Path::new(name).components();
    if !matches!(
        (comps.next(), comps.next()),
        (Some(Component::Normal(_)), None)
    ) {
        return false;
    }

    #[cfg(windows)]
    if is_windows_reserved_probe_name(name) {
        return false;
    }

    true
}

/// Returns `true` when `name` is pure ASCII without trailing dots or spaces, so ASCII case
/// sensitivity alone completely determines filename equivalence on standard filesystems.
fn is_plain_ascii_probe_name(name: &str) -> bool {
    name.is_ascii() && !name.ends_with(['.', ' '])
}

/// Builds a fail-closed filesystem error message when filename distinctness cannot be verified.
fn unverified_probe_error(probe_dir: Option<&Path>) -> String {
    match probe_dir {
        Some(dir) => format!(
            "Destination path collision check could not verify whether candidate paths are distinct in '{}'",
            dir.display()
        ),
        None => "Destination path collision check could not verify whether candidate paths are distinct in the destination directory".to_string(),
    }
}

/// Outcome of probing candidate names inside a private temporary subdirectory.
#[derive(Debug, PartialEq, Eq)]
enum TempDirProbeOutcome {
    /// Successfully determined equivalence groups for all candidates.
    Resolved(Vec<usize>),
    /// Could not create the temporary subdirectory inside `probe_dir` (for example, directory
    /// permissions allow file creation but deny `mkdir`, or the directory is read-only).
    TempDirUnavailable,
    /// Candidate names could not be probed even inside a temporary directory (for example,
    /// overlong or host-invalid filenames).
    Inconclusive,
}

/// Outcome of probing candidate names directly in `probe_dir` using uniquely prefixed files.
#[derive(Debug, PartialEq, Eq)]
enum PrefixedProbeOutcome {
    /// Successfully determined equivalence groups for all candidates.
    Resolved(Vec<usize>),
    /// `probe_dir` denied file creation because it is unwritable or read-only.
    DirUnwritable,
    /// Candidate names could not be probed conclusively.
    Inconclusive,
}

/// Probes actual filename equivalence inside a private temporary directory created in `probe_dir`.
fn probe_names_in_dir(
    probe_dir: &Path,
    names: &[String],
    candidates: &[usize],
) -> TempDirProbeOutcome {
    if !candidates
        .iter()
        .all(|&idx| is_probable_component_name(&names[idx]))
    {
        return TempDirProbeOutcome::Inconclusive;
    }

    let Ok(temp_dir) = create_temp_probe_dir(probe_dir) else {
        return TempDirProbeOutcome::TempDirUnavailable;
    };
    let mut groups: Vec<usize> = (0..names.len()).collect();

    for &idx in candidates {
        let name = &names[idx];
        let probe_path = temp_dir.0.join(name);
        let mut create_opts = fs::OpenOptions::new();
        create_opts.write(true).create_new(true);
        configure_no_follow(&mut create_opts);

        match create_opts.open(&probe_path) {
            Ok(mut file) => {
                let write_res = file.write_all(idx.to_string().as_bytes());
                drop(file);
                if write_res.is_err() {
                    return TempDirProbeOutcome::Inconclusive;
                }
                groups[idx] = idx;
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                let mut read_opts = fs::OpenOptions::new();
                read_opts.read(true);
                configure_no_follow(&mut read_opts);
                let Ok(mut file) = read_opts.open(&probe_path) else {
                    return TempDirProbeOutcome::Inconclusive;
                };
                if !file.metadata().ok().map(|m| m.is_file()).unwrap_or(false) {
                    drop(file);
                    return TempDirProbeOutcome::Inconclusive;
                }
                let mut tag = String::new();
                let read_res = file.read_to_string(&mut tag);
                drop(file);
                if read_res.is_err() {
                    return TempDirProbeOutcome::Inconclusive;
                }
                let Ok(earlier_idx) = tag.trim().parse::<usize>() else {
                    return TempDirProbeOutcome::Inconclusive;
                };
                if earlier_idx >= idx || earlier_idx >= names.len() {
                    return TempDirProbeOutcome::Inconclusive;
                }
                groups[idx] = groups[earlier_idx];
            }
            Err(_) => return TempDirProbeOutcome::Inconclusive,
        }
    }

    drop(temp_dir);
    TempDirProbeOutcome::Resolved(groups)
}

/// Probes actual filename equivalence directly inside `probe_dir` using uniquely prefixed
/// temporary files (`.ia-get-fs-probe-<pid>-<seq>-<name>`) with RAII cleanup.
fn probe_prefixed_names_in_dir(
    probe_dir: &Path,
    names: &[String],
    candidates: &[usize],
) -> PrefixedProbeOutcome {
    probe_prefixed_names_in_dir_with_seq(probe_dir, names, candidates, || {
        CASE_PROBE_COUNTER.fetch_add(1, Ordering::Relaxed)
    })
}

/// Internal implementation of [`probe_prefixed_names_in_dir`] accepting a sequence generator for
/// deterministic collision-avoidance unit tests.
fn probe_prefixed_names_in_dir_with_seq<F>(
    probe_dir: &Path,
    names: &[String],
    candidates: &[usize],
    mut next_seq: F,
) -> PrefixedProbeOutcome
where
    F: FnMut() -> u64,
{
    if !candidates
        .iter()
        .all(|&idx| is_probable_component_name(&names[idx]))
    {
        return PrefixedProbeOutcome::Inconclusive;
    }

    let pid = std::process::id();

    for _ in 0..MAX_TEMP_CASE_PROBE_ATTEMPTS {
        let seq = next_seq();
        let prefix = format!(".ia-get-fs-probe-{pid}-{seq}-");
        let token_prefix = format!("{pid}:{seq}:");
        let mut created = ProbeFilesGuard::default();
        let mut groups: Vec<usize> = (0..names.len()).collect();
        let mut retry_with_new_seq = false;

        for &idx in candidates {
            let name = &names[idx];
            let probe_path = probe_dir.join(format!("{prefix}{name}"));
            let mut create_opts = fs::OpenOptions::new();
            create_opts.write(true).create_new(true);
            configure_no_follow(&mut create_opts);

            match create_opts.open(&probe_path) {
                Ok(mut file) => {
                    created.push(probe_path);
                    let write_res = file.write_all(format!("{token_prefix}{idx}").as_bytes());
                    drop(file);
                    if write_res.is_err() {
                        return PrefixedProbeOutcome::Inconclusive;
                    }
                    groups[idx] = idx;
                }
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                    let mut read_opts = fs::OpenOptions::new();
                    read_opts.read(true);
                    configure_no_follow(&mut read_opts);
                    let Ok(mut file) = read_opts.open(&probe_path) else {
                        retry_with_new_seq = true;
                        break;
                    };
                    if !file.metadata().ok().map(|m| m.is_file()).unwrap_or(false) {
                        drop(file);
                        retry_with_new_seq = true;
                        break;
                    }
                    let mut bytes = Vec::new();
                    let read_res = (&mut file)
                        .take(MAX_PROBE_TOKEN_BYTES)
                        .read_to_end(&mut bytes);
                    drop(file);
                    if read_res.is_err() || bytes.len() as u64 == MAX_PROBE_TOKEN_BYTES {
                        retry_with_new_seq = true;
                        break;
                    }
                    let Some(idx_str) = std::str::from_utf8(&bytes)
                        .ok()
                        .and_then(|tag| tag.trim().strip_prefix(&token_prefix))
                    else {
                        retry_with_new_seq = true;
                        break;
                    };
                    let Ok(earlier_idx) = idx_str.parse::<usize>() else {
                        return PrefixedProbeOutcome::Inconclusive;
                    };
                    if earlier_idx >= idx || earlier_idx >= names.len() {
                        return PrefixedProbeOutcome::Inconclusive;
                    }
                    groups[idx] = groups[earlier_idx];
                }
                Err(err)
                    if matches!(
                        err.kind(),
                        std::io::ErrorKind::PermissionDenied
                            | std::io::ErrorKind::ReadOnlyFilesystem
                    ) =>
                {
                    return PrefixedProbeOutcome::DirUnwritable;
                }
                Err(_) => return PrefixedProbeOutcome::Inconclusive,
            }
        }

        drop(created);
        if !retry_with_new_seq {
            return PrefixedProbeOutcome::Resolved(groups);
        }
    }

    PrefixedProbeOutcome::Inconclusive
}

/// Fail-closed fallback filename equivalence check when live probing cannot run.
///
/// Uses read-only existing-child probing only when it can definitively decide the candidates:
/// - If read-only probing confirms `CaseInsensitive` and case-folding already detects a collision,
///   returns those equivalence groups (while keeping `unverified_error` set whenever non-ASCII or
///   unprobable names remain).
/// - If `allow_read_only_distinct` is `true`, all candidate names are pure ASCII without trailing
///   dots or spaces, and read-only probing confirms `CaseSensitive`, returns distinct groups.
/// - Otherwise fails closed with [`IaGetError::FileSystem`].
fn fallback_dir_equivalence(
    probe_dir: Option<&Path>,
    names: &[String],
    candidates: &[usize],
    allow_read_only_distinct: bool,
) -> Result<DirEquivalenceResult> {
    let all_plain_ascii = candidates
        .iter()
        .all(|&idx| is_plain_ascii_probe_name(&names[idx]));
    let read_only_sensitivity = probe_dir.and_then(probe_existing_child_case_sensitivity);

    match read_only_sensitivity {
        Some(PathCaseSensitivity::CaseInsensitive) => {
            let mut seen: HashMap<String, usize> = HashMap::with_capacity(candidates.len());
            let mut groups: Vec<usize> = (0..names.len()).collect();
            let mut has_equivalence = false;

            for &idx in candidates {
                let folded = names[idx].to_lowercase();
                let group = *seen.entry(folded).or_insert(idx);
                if group != idx {
                    has_equivalence = true;
                }
                groups[idx] = group;
            }

            if allow_read_only_distinct && all_plain_ascii {
                return Ok(DirEquivalenceResult {
                    groups,
                    reason_suffix: " on a case-insensitive filesystem",
                    unverified_error: None,
                });
            }

            if has_equivalence {
                return Ok(DirEquivalenceResult {
                    groups,
                    reason_suffix: " on a case-insensitive filesystem",
                    unverified_error: Some(unverified_probe_error(probe_dir)),
                });
            }
        }
        Some(PathCaseSensitivity::CaseSensitive) if allow_read_only_distinct && all_plain_ascii => {
            return Ok(DirEquivalenceResult {
                groups: (0..names.len()).collect(),
                reason_suffix: "",
                unverified_error: None,
            });
        }
        _ => {}
    }

    Err(IaGetError::FileSystem(unverified_probe_error(probe_dir)))
}

impl DirectoryLookup for OsDirectoryLookup {
    fn resolve_root(&self, dir: &Path) -> ProbeRoot {
        resolve_probe_root(dir)
    }

    fn resolve_child_dir(
        &self,
        canonical_root: Option<&Path>,
        parent_on_disk: &Path,
        segment: &str,
    ) -> Option<PathBuf> {
        resolve_contained_child_dir(canonical_root, parent_on_disk, segment)
    }

    fn resolve_equivalences(
        &self,
        probe_dir: Option<&Path>,
        names: &[String],
        candidates: &[usize],
    ) -> Result<DirEquivalenceResult> {
        let mut allow_read_only_distinct = false;

        if let Some(dir) = probe_dir {
            let try_prefixed = if self.allow_temp_subdir_probe {
                match probe_names_in_dir(dir, names, candidates) {
                    TempDirProbeOutcome::Resolved(groups) => {
                        return Ok(DirEquivalenceResult {
                            groups,
                            reason_suffix:
                                " under the destination directory's filesystem lookup rules",
                            unverified_error: None,
                        });
                    }
                    TempDirProbeOutcome::TempDirUnavailable => true,
                    TempDirProbeOutcome::Inconclusive => false,
                }
            } else {
                true
            };

            if try_prefixed {
                match probe_prefixed_names_in_dir(dir, names, candidates) {
                    PrefixedProbeOutcome::Resolved(groups) => {
                        return Ok(DirEquivalenceResult {
                            groups,
                            reason_suffix:
                                " under the destination directory's filesystem lookup rules",
                            unverified_error: None,
                        });
                    }
                    PrefixedProbeOutcome::DirUnwritable => {
                        allow_read_only_distinct = true;
                    }
                    PrefixedProbeOutcome::Inconclusive => {}
                }
            }
        }

        fallback_dir_equivalence(probe_dir, names, candidates, allow_read_only_distinct)
    }
}

/// Distinguishes whether a planned path placement represents a final file or a `.part` staging
/// file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlannedPathKind {
    Final,
    Staging,
}

/// Role played by a planned path item at a specific directory depth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CollisionRole {
    Final,
    Staging,
    Subdir,
}

/// Normalised planned path placement (either final destination or `.part` staging file).
#[derive(Debug)]
struct PlannedPathItem<'a> {
    entry_idx: usize,
    entry_label: &'a str,
    kind: PlannedPathKind,
    segments: Vec<String>,
    display_path: String,
}

/// Per-equivalence-group occupancy state while checking a single directory level.
#[derive(Default)]
struct GroupOccupancy {
    final_item: Option<usize>,
    staging_item: Option<usize>,
    subdir_first_item: Option<usize>,
    subdir_items: Vec<usize>,
    subdir_name_ids: Vec<usize>,
}

/// Work item representing a single planned directory level to validate.
struct DirLevelWork {
    depth: usize,
    item_indices: Vec<usize>,
    probe_dir: Option<PathBuf>,
    exact_on_disk_dir: Option<PathBuf>,
    inherited_reason_suffix: Option<&'static str>,
}

/// Extracts strictly normal UTF-8 path segments from a normalised relative path.
fn normal_path_segments(path: &Path) -> Option<Vec<String>> {
    let mut segments = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(segment) => segments.push(segment.to_str()?.to_string()),
            _ => return None,
        }
    }
    if segments.is_empty() {
        None
    } else {
        Some(segments)
    }
}

/// Selects the appropriate collision reason suffix depending on whether the conflict occurred on
/// exact identical segments or via non-exact lookup equivalence at this level or an ancestor.
fn collision_reason_suffix(
    first: &PlannedPathItem<'_>,
    second: &PlannedPathItem<'_>,
    depth: usize,
    level_reason_suffix: &'static str,
    inherited_reason_suffix: Option<&'static str>,
) -> &'static str {
    if first.segments[depth] != second.segments[depth] {
        level_reason_suffix
    } else if first.segments[..depth] != second.segments[..depth] {
        inherited_reason_suffix.unwrap_or(level_reason_suffix)
    } else {
        ""
    }
}

/// Builds a [`IaGetError::PathCollision`] for two conflicting items at `depth`.
fn build_level_collision_error(
    first: &PlannedPathItem<'_>,
    second: &PlannedPathItem<'_>,
    depth: usize,
    first_role: CollisionRole,
    second_role: CollisionRole,
    level_reason_suffix: &'static str,
    inherited_reason_suffix: Option<&'static str>,
) -> IaGetError {
    let suffix = collision_reason_suffix(
        first,
        second,
        depth,
        level_reason_suffix,
        inherited_reason_suffix,
    );
    let local_path = match second_role {
        CollisionRole::Final | CollisionRole::Staging => second.display_path.clone(),
        CollisionRole::Subdir => second.segments[..=depth].join("/"),
    };
    let reason = match (first_role, second_role) {
        (CollisionRole::Final, CollisionRole::Final) => {
            format!("both entries map to the same destination path{suffix}")
        }
        (CollisionRole::Staging, CollisionRole::Final) => {
            format!("destination path collides with staging '.part' path{suffix}")
        }
        (CollisionRole::Final, CollisionRole::Staging) => {
            format!("staging '.part' path collides with destination path{suffix}")
        }
        (CollisionRole::Staging, CollisionRole::Staging) => {
            format!("both entries map to the same staging '.part' path{suffix}")
        }
        (CollisionRole::Subdir, CollisionRole::Final) => {
            format!("destination path collides with directory path{suffix}")
        }
        (CollisionRole::Final, CollisionRole::Subdir) => {
            format!("directory path collides with destination path{suffix}")
        }
        (CollisionRole::Subdir, CollisionRole::Staging) => {
            format!("staging '.part' path collides with directory path{suffix}")
        }
        (CollisionRole::Staging, CollisionRole::Subdir) => {
            format!("directory path collides with staging '.part' path{suffix}")
        }
        (CollisionRole::Subdir, CollisionRole::Subdir) => {
            unreachable!("matching subdirectories merge instead of colliding")
        }
    };

    IaGetError::PathCollision {
        local_path,
        first_entry: first.entry_label.to_string(),
        second_entry: second.entry_label.to_string(),
        reason,
    }
}

/// Checks all items at a single directory level against equivalence `groups` and returns the
/// per-group occupancy (including merged subdirectory children) on success.
fn check_level_groups(
    all_items: &[PlannedPathItem<'_>],
    item_indices: &[usize],
    item_name_ids: &[usize],
    depth: usize,
    groups: &[usize],
    level_reason_suffix: &'static str,
    inherited_reason_suffix: Option<&'static str>,
) -> Result<Vec<GroupOccupancy>> {
    let mut occupancy: Vec<GroupOccupancy> = (0..groups.len())
        .map(|_| GroupOccupancy::default())
        .collect();

    for (pos, &item_idx) in item_indices.iter().enumerate() {
        let name_id = item_name_ids[pos];
        let group_id = groups[name_id];
        let item = &all_items[item_idx];
        let occ = &mut occupancy[group_id];

        if depth + 1 < item.segments.len() {
            if let Some(first_idx) = occ.final_item {
                if all_items[first_idx].entry_idx != item.entry_idx {
                    return Err(build_level_collision_error(
                        &all_items[first_idx],
                        item,
                        depth,
                        CollisionRole::Final,
                        CollisionRole::Subdir,
                        level_reason_suffix,
                        inherited_reason_suffix,
                    ));
                }
            }
            if let Some(first_idx) = occ.staging_item {
                if all_items[first_idx].entry_idx != item.entry_idx {
                    return Err(build_level_collision_error(
                        &all_items[first_idx],
                        item,
                        depth,
                        CollisionRole::Staging,
                        CollisionRole::Subdir,
                        level_reason_suffix,
                        inherited_reason_suffix,
                    ));
                }
            }
            if occ.subdir_first_item.is_none() {
                occ.subdir_first_item = Some(item_idx);
            }
            occ.subdir_items.push(item_idx);
            if !occ.subdir_name_ids.contains(&name_id) {
                occ.subdir_name_ids.push(name_id);
            }
        } else {
            let role = match item.kind {
                PlannedPathKind::Final => CollisionRole::Final,
                PlannedPathKind::Staging => CollisionRole::Staging,
            };

            if let Some(first_idx) = occ.final_item {
                if all_items[first_idx].entry_idx != item.entry_idx {
                    return Err(build_level_collision_error(
                        &all_items[first_idx],
                        item,
                        depth,
                        CollisionRole::Final,
                        role,
                        level_reason_suffix,
                        inherited_reason_suffix,
                    ));
                }
            }
            if let Some(first_idx) = occ.staging_item {
                if all_items[first_idx].entry_idx != item.entry_idx {
                    return Err(build_level_collision_error(
                        &all_items[first_idx],
                        item,
                        depth,
                        CollisionRole::Staging,
                        role,
                        level_reason_suffix,
                        inherited_reason_suffix,
                    ));
                }
            }
            if let Some(first_idx) = occ.subdir_first_item {
                if all_items[first_idx].entry_idx != item.entry_idx {
                    return Err(build_level_collision_error(
                        &all_items[first_idx],
                        item,
                        depth,
                        CollisionRole::Subdir,
                        role,
                        level_reason_suffix,
                        inherited_reason_suffix,
                    ));
                }
            }

            match item.kind {
                PlannedPathKind::Final => {
                    if occ.final_item.is_none() {
                        occ.final_item = Some(item_idx);
                    }
                }
                PlannedPathKind::Staging => {
                    if occ.staging_item.is_none() {
                        occ.staging_item = Some(item_idx);
                    }
                }
            }
        }
    }

    Ok(occupancy)
}

/// Validates planned `(entry_label, local_path)` pairs using per-parent-directory filesystem
/// lookup rules supplied by `lookup`.
fn validate_download_paths_with_lookup(
    dir: &Path,
    entries: &[(&str, &str)],
    lookup: &impl DirectoryLookup,
) -> Result<()> {
    // Fast exact pass catches byte-identical final and `.part` collisions with zero I/O.
    validate_collision_keys(entries, Path::to_path_buf, "")?;

    if entries.len() < 2 {
        return Ok(());
    }

    let mut all_items: Vec<PlannedPathItem<'_>> = Vec::with_capacity(entries.len() * 2);
    for (entry_idx, &(entry_label, local_path)) in entries.iter().enumerate() {
        let clean_final = collision_path_key(Path::new(local_path));
        if let Some(segments) = normal_path_segments(&clean_final) {
            all_items.push(PlannedPathItem {
                entry_idx,
                entry_label,
                kind: PlannedPathKind::Final,
                segments,
                display_path: format_collision_path(&clean_final),
            });
        }

        let part_path = part_file_path(local_path);
        let clean_staging = collision_path_key(&part_path);
        if let Some(segments) = normal_path_segments(&clean_staging) {
            all_items.push(PlannedPathItem {
                entry_idx,
                entry_label,
                kind: PlannedPathKind::Staging,
                segments,
                display_path: format_collision_path(&clean_staging),
            });
        }
    }

    if all_items.len() < 2 {
        return Ok(());
    }

    let root = lookup.resolve_root(dir);
    let canonical_root = root.exact_on_disk_dir.clone();

    let mut work_stack = vec![DirLevelWork {
        depth: 0,
        item_indices: (0..all_items.len()).collect(),
        probe_dir: root.probe_dir,
        exact_on_disk_dir: root.exact_on_disk_dir,
        inherited_reason_suffix: None,
    }];

    while let Some(level) = work_stack.pop() {
        let (Some(&first_item_idx), Some(&last_item_idx)) =
            (level.item_indices.first(), level.item_indices.last())
        else {
            continue;
        };
        if all_items[first_item_idx].entry_idx == all_items[last_item_idx].entry_idx {
            continue;
        }

        let mut name_to_id: HashMap<&str, usize> = HashMap::new();
        let mut names: Vec<String> = Vec::new();
        let mut item_name_ids: Vec<usize> = Vec::with_capacity(level.item_indices.len());

        for &item_idx in &level.item_indices {
            let seg = all_items[item_idx].segments[level.depth].as_str();
            let name_id = match name_to_id.get(seg) {
                Some(&id) => id,
                None => {
                    let id = names.len();
                    name_to_id.insert(seg, id);
                    names.push(seg.to_string());
                    id
                }
            };
            item_name_ids.push(name_id);
        }

        // First validate exact component groups (catches exact file-vs-directory and merged-parent
        // exact leaf collisions without filesystem probing).
        let exact_groups: Vec<usize> = (0..names.len()).collect();
        let mut occupancy = check_level_groups(
            &all_items,
            &level.item_indices,
            &item_name_ids,
            level.depth,
            &exact_groups,
            "",
            level.inherited_reason_suffix,
        )?;
        let mut level_reason_suffix = "";

        let candidates = select_probe_candidates(&names);
        if !candidates.is_empty() {
            let eq =
                lookup.resolve_equivalences(level.probe_dir.as_deref(), &names, &candidates)?;
            if eq
                .groups
                .iter()
                .enumerate()
                .any(|(idx, &group)| group != idx)
            {
                occupancy = check_level_groups(
                    &all_items,
                    &level.item_indices,
                    &item_name_ids,
                    level.depth,
                    &eq.groups,
                    eq.reason_suffix,
                    level.inherited_reason_suffix,
                )?;
                level_reason_suffix = eq.reason_suffix;
            }
            if let Some(err_msg) = eq.unverified_error {
                return Err(IaGetError::FileSystem(err_msg));
            }
        }

        let mut next_levels = Vec::new();
        for occ in occupancy {
            let (Some(&first_child), Some(&last_child)) =
                (occ.subdir_items.first(), occ.subdir_items.last())
            else {
                continue;
            };
            if all_items[first_child].entry_idx == all_items[last_child].entry_idx {
                continue;
            }

            let child_exact_on_disk = level.exact_on_disk_dir.as_deref().and_then(|parent_dir| {
                occ.subdir_name_ids.iter().find_map(|&name_id| {
                    lookup.resolve_child_dir(canonical_root.as_deref(), parent_dir, &names[name_id])
                })
            });
            let child_probe_dir = child_exact_on_disk
                .clone()
                .or_else(|| level.probe_dir.clone());
            let child_inherited_reason_suffix = if occ.subdir_name_ids.len() > 1 {
                Some(level_reason_suffix)
            } else {
                level.inherited_reason_suffix
            };

            next_levels.push(DirLevelWork {
                depth: level.depth + 1,
                item_indices: occ.subdir_items,
                probe_dir: child_probe_dir,
                exact_on_disk_dir: child_exact_on_disk,
                inherited_reason_suffix: child_inherited_reason_suffix,
            });
        }

        // Push in reverse order so child directories are visited in first-appearance order.
        for next in next_levels.into_iter().rev() {
            work_stack.push(next);
        }
    }

    Ok(())
}

/// Validates that no two planned entries collide on either a final destination path,
/// a `.part` staging path, or a planned directory path according to each parent directory's
/// actual filesystem lookup rules.
///
/// Probes only existing parent directories (or the nearest existing ancestor when directories do
/// not exist yet, without creating missing directories or following symlinks inside `dir`) and
/// avoids filesystem I/O when all names at a directory level are pure ASCII without case
/// conflicts.
pub fn validate_download_paths_for_dir<'a, I>(dir: &Path, entries: I) -> Result<()>
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    let entries: Vec<(&'a str, &'a str)> = entries.into_iter().collect();
    validate_download_paths_with_lookup(dir, &entries, &OsDirectoryLookup::PRODUCTION)
}

/// Validates that no two planned entries collide on either a final destination path
/// or a `.part` staging path using exact (case-sensitive) matching.
///
/// Each item is `(entry_label, local_path)`, where `entry_label` identifies the source
/// archive entry for error reporting and `local_path` is the local destination path.
///
/// Callers targeting a specific destination directory should use
/// [`validate_download_paths_for_dir`] so case-insensitive filesystems also reject
/// case-only collisions.
pub fn validate_download_paths<'a, I>(entries: I) -> Result<()>
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    validate_download_paths_with_case_sensitivity(entries, PathCaseSensitivity::CaseSensitive)
}

/// Returns the current byte length of a `.part` file inside `root`, or `0` if it does not exist.
fn existing_part_size(root: &Path, rel_part_path: &Path) -> Result<u64> {
    let canonical_root = resolve_destination_root(root)?;
    let part_path = validate_contained_path(&canonical_root, rel_part_path, true)?;
    match fs::symlink_metadata(&part_path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(IaGetError::FileSystem(format!(
                    "Refusing symlinked partial file '{}'",
                    part_path.display()
                )));
            }
            if !metadata.file_type().is_file() {
                return Err(IaGetError::FileSystem(format!(
                    "Refusing non-regular partial file '{}'",
                    part_path.display()
                )));
            }
            Ok(metadata.len())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(e) => Err(e.into()),
    }
}

/// Atomically promote a verified `.part` file to its final destination path inside `root`.
fn promote_part_file(root: &Path, rel_part_path: &Path, rel_final_path: &Path) -> Result<()> {
    let canonical_root = resolve_destination_root(root)?;
    let part_path = validate_contained_path(&canonical_root, rel_part_path, false)?;
    let final_path = validate_contained_path(&canonical_root, rel_final_path, true)?;
    fs::rename(&part_path, &final_path)?;
    Ok(())
}

/// Calculates the MD5 hash of a verified regular file inside `root`,
/// optionally displaying a progress bar for large files.
pub(crate) fn calculate_md5_with_progress(
    root: &Path,
    rel_path: &Path,
    running: &Arc<AtomicBool>,
    show_progress: bool,
) -> Result<String> {
    let file = open_regular_file_for_read(root, rel_path)?;
    let file_size = file.metadata()?.len();
    let is_large_file = show_progress && file_size > LARGE_FILE_THRESHOLD;

    let mut reader = BufReader::with_capacity(BUFFER_SIZE, file);
    let mut context = md5::Context::new();
    let mut buffer = [0; BUFFER_SIZE];

    let pb = if is_large_file {
        Some(create_progress_bar(
            file_size,
            &format!("{} {}    ", "╰╼".cyan().dimmed(), "Verifying".white()),
            Some("blue/blue"),
            false,
        ))
    } else {
        None
    };

    let mut bytes_processed: u64 = 0;

    loop {
        let bytes_read = reader.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }

        if !running.load(Ordering::SeqCst) {
            if let Some(ref progress_bar) = pb {
                progress_bar.finish_and_clear();
            }
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "Hash calculation interrupted by signal",
            )
            .into());
        }

        context.consume(&buffer[..bytes_read]);

        if let Some(ref progress_bar) = pb {
            bytes_processed += bytes_read as u64;
            progress_bar.set_position(bytes_processed);
        }
    }

    if let Some(progress_bar) = pb.as_ref() {
        progress_bar.finish_and_clear();
    }

    let hash = context.finalize();
    Ok(format!("{:x}", hash))
}

/// Calculates the MD5 hash of a verified regular file inside `root`.
pub(crate) fn calculate_md5(
    root: &Path,
    rel_path: &Path,
    running: &Arc<AtomicBool>,
) -> Result<String> {
    calculate_md5_with_progress(root, rel_path, running, true)
}

/// Result of inspecting an existing destination file before starting a download.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExistingFileStatus {
    Missing,
    CompleteMd5Verified,
    CompleteNoMd5,
    NeedsRedownload,
}

/// Check if an existing file inside `root` has the correct hash.
fn check_existing_file(
    root: &Path,
    file_path: &str,
    expected_md5: Option<&str>,
    running: &Arc<AtomicBool>,
) -> Result<ExistingFileStatus> {
    let canonical_root = resolve_destination_root(root)?;
    let rel_path = Path::new(file_path);
    let target_path = validate_contained_path(&canonical_root, rel_path, true)?;

    match fs::symlink_metadata(&target_path) {
        Ok(meta) => {
            if meta.file_type().is_symlink() {
                return Err(IaGetError::FileSystem(format!(
                    "Refusing symlink target '{}'",
                    target_path.display()
                )));
            }
            if !meta.file_type().is_file() {
                return Err(IaGetError::FileSystem(format!(
                    "Refusing non-regular file target '{}'",
                    target_path.display()
                )));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ExistingFileStatus::Missing)
        }
        Err(e) => return Err(e.into()),
    }

    let Some(expected) = expected_md5 else {
        return Ok(ExistingFileStatus::CompleteNoMd5);
    };

    let local_md5 = match calculate_md5(root, rel_path, running) {
        Ok(hash) => hash,
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("interrupted by signal")
                || msg.contains("Refusing")
                || msg.contains("escapes destination root")
            {
                return Err(e);
            }
            println!(
                "{} {} to calculate MD5 hash: {}",
                "╰╼".cyan().dimmed(),
                "Failed".red().bold(),
                e
            );
            return Ok(ExistingFileStatus::NeedsRedownload);
        }
    };

    if local_md5 == expected {
        Ok(ExistingFileStatus::CompleteMd5Verified)
    } else {
        Ok(ExistingFileStatus::NeedsRedownload)
    }
}

/// Ensure parent directories exist inside `root` for a relative destination path.
fn ensure_parent_directories(root: &Path, rel_path: &Path) -> Result<PathBuf> {
    let canonical_root = resolve_destination_root(root)?;
    let clean_rel = validate_relative_path(rel_path)?;

    let mut current = canonical_root.clone();
    if let Some(parent) = clean_rel.parent() {
        for component in parent.components() {
            let Component::Normal(segment) = component else {
                return Err(IaGetError::FileSystem(format!(
                    "Refusing unsafe parent path '{}'",
                    rel_path.display()
                )));
            };
            current.push(segment);

            match fs::symlink_metadata(&current) {
                Ok(meta) => {
                    if meta.file_type().is_symlink() {
                        return Err(IaGetError::FileSystem(format!(
                            "Refusing symlinked directory '{}'",
                            current.display()
                        )));
                    }
                    if !meta.file_type().is_dir() {
                        return Err(IaGetError::FileSystem(format!(
                            "Cannot create directory '{}': existing path is not a directory",
                            current.display()
                        )));
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    match fs::create_dir(&current) {
                        Ok(()) => {}
                        Err(create_err)
                            if create_err.kind() == std::io::ErrorKind::AlreadyExists =>
                        {
                            let meta = fs::symlink_metadata(&current)?;
                            if meta.file_type().is_symlink() {
                                return Err(IaGetError::FileSystem(format!(
                                    "Refusing symlinked directory '{}'",
                                    current.display()
                                )));
                            }
                            if !meta.file_type().is_dir() {
                                return Err(IaGetError::FileSystem(format!(
                                    "Cannot create directory '{}': existing path is not a directory",
                                    current.display()
                                )));
                            }
                        }
                        Err(create_err) => return Err(create_err.into()),
                    }
                }
                Err(e) => return Err(e.into()),
            }

            let canonical_current = fs::canonicalize(&current)?;
            if !canonical_current.starts_with(&canonical_root) {
                return Err(IaGetError::FileSystem(format!(
                    "Directory '{}' escapes destination root '{}'",
                    current.display(),
                    canonical_root.display()
                )));
            }
        }
    }

    Ok(current)
}

/// Prepare a `.part` file inside `root` for download by opening it safely for truncation or append.
fn prepare_file_for_download(root: &Path, rel_part_path: &Path, truncate: bool) -> Result<File> {
    let canonical_root = resolve_destination_root(root)?;
    let part_path = validate_contained_path(&canonical_root, rel_part_path, true)?;

    let mut options = fs::OpenOptions::new();
    options.write(true).create(true);
    configure_no_follow(&mut options);

    let mut file = options.open(&part_path)?;
    if !file.metadata()?.file_type().is_file() {
        return Err(IaGetError::FileSystem(format!(
            "Refusing non-regular partial file '{}'",
            part_path.display()
        )));
    }

    if truncate {
        file.set_len(0)?;
        file.seek(SeekFrom::Start(0))?;
    } else {
        file.seek(SeekFrom::End(0))?;
    }

    Ok(file)
}

/// Download file content into a `.part` file with progress reporting and automatic retry on failure
#[allow(clippy::too_many_arguments)]
async fn download_file_content(
    client: &Client,
    root: &Path,
    url: &str,
    part_path: &Path,
    signal: &ShutdownSignal,
    cookie_source: Option<&CookieSource>,
    expected_md5: Option<&str>,
    options: &DownloadOptions,
    logger: &VerboseLogger,
) -> Result<(u64, bool)> {
    let policy = &options.retry_policy;
    let initial_url = Url::parse(url).map_err(|e| {
        IaGetError::Network(format!(
            "Invalid download URL '{}': {}",
            redact_proxy_value(url),
            e
        ))
    })?;
    let safe_initial_url = sanitize_url_for_log(&initial_url);
    let mut retry_count: u32 = 0;
    let mut has_reset_range = false;

    loop {
        if !signal.is_running() {
            return Err(interrupted_error());
        }

        // Re-check `.part` file size at start of each attempt (in case of retry)
        let current_file_size = existing_part_size(root, part_path)?;
        let range_header = if current_file_size > 0 {
            Some(
                HeaderValue::from_str(&format!("bytes={}-", current_file_size)).map_err(|e| {
                    IaGetError::Network(format!("Invalid range header value: {}", e))
                })?,
            )
        } else {
            None
        };

        let send_outcome =
            send_following_redirects(&initial_url, signal, MAX_REDIRECTS, logger, |target_url| {
                let mut headers = HeaderMap::new();
                if let Some(cookie_value) = cookie_source
                    .map(|source| source.header_value_for_url(target_url))
                    .transpose()?
                    .flatten()
                {
                    headers.insert(reqwest::header::COOKIE, cookie_value);
                }
                if let Some(range_header) = range_header.as_ref() {
                    headers.insert(reqwest::header::RANGE, range_header.clone());
                }

                let mut request = client.get(target_url.clone());
                if !headers.is_empty() {
                    request = request.headers(headers);
                }
                Ok(request)
            })
            .await?;

        let mut response = match send_outcome {
            RedirectSendOutcome::Response(resp) => resp,
            RedirectSendOutcome::TransportError(e) => {
                // Request failed before we even got a response
                if !is_retryable_reqwest_error(&e) {
                    return Err(e.without_url().into());
                }

                let safe_err = e.without_url();
                if retry_count >= policy.max_retries {
                    logger.log(
                        "retry",
                        format_args!(
                            "exhausted ({}/{}) for {}: {}",
                            retry_count, policy.max_retries, safe_initial_url, safe_err
                        ),
                    );
                    println!(
                        "{} {}      {} Maximum retries ({}) exceeded",
                        "├╼".cyan().dimmed(),
                        "Failed".red().bold(),
                        "✘".red().bold(),
                        policy.max_retries
                    );
                    return Err(safe_err.into());
                }

                retry_count += 1;
                let (delay, delay_source) =
                    compute_retry_delay_with_source(retry_count, None, SystemTime::now(), policy);
                logger.log(
                    "retry",
                    format_args!(
                        "attempt {}/{} for {} after {}; waiting {:.2}s ({})",
                        retry_count,
                        policy.max_retries,
                        safe_initial_url,
                        safe_err,
                        delay.as_secs_f64(),
                        delay_source
                    ),
                );
                println!(
                    "{} {}      {} Connection error (attempt {}/{}): {}",
                    "├╼".cyan().dimmed(),
                    "Retry".yellow().bold(),
                    "⟳".yellow().bold(),
                    retry_count,
                    policy.max_retries,
                    safe_err
                );
                println!(
                    "{} {}      Waiting {:.1}s before retry...",
                    "├╼".cyan().dimmed(),
                    "Wait".white(),
                    delay.as_secs_f64()
                );

                sleep_interruptible(delay, signal).await?;

                continue; // Retry from the top of the loop
            }
        };

        let status = response.status();
        if is_retryable_status(status) {
            let retry_after = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            drop(response);

            if retry_count < policy.max_retries {
                retry_count += 1;
                let (delay, delay_source) = compute_retry_delay_with_source(
                    retry_count,
                    retry_after.as_deref(),
                    SystemTime::now(),
                    policy,
                );
                logger.log(
                    "retry",
                    format_args!(
                        "attempt {}/{} for {} after HTTP {}; waiting {:.2}s ({})",
                        retry_count,
                        policy.max_retries,
                        safe_initial_url,
                        status,
                        delay.as_secs_f64(),
                        delay_source
                    ),
                );
                println!(
                    "{} {}      {} HTTP {} (attempt {}/{})",
                    "├╼".cyan().dimmed(),
                    "Retry".yellow().bold(),
                    "⟳".yellow().bold(),
                    status,
                    retry_count,
                    policy.max_retries
                );
                println!(
                    "{} {}      Waiting {:.1}s before retry...",
                    "├╼".cyan().dimmed(),
                    "Wait".white(),
                    delay.as_secs_f64()
                );

                sleep_interruptible(delay, signal).await?;
                continue;
            }

            logger.log(
                "retry",
                format_args!(
                    "exhausted ({}/{}) for {} after HTTP {}",
                    retry_count, policy.max_retries, safe_initial_url, status
                ),
            );
            println!(
                "{} {}      {} Maximum retries ({}) exceeded",
                "├╼".cyan().dimmed(),
                "Failed".red().bold(),
                "✘".red().bold(),
                policy.max_retries
            );
            return Err(IaGetError::Network(format!("HTTP error {}", status)));
        }

        let content_range_header = response
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|value| value.to_str().ok());
        let content_length = response.content_length();

        let action = classify_download_response(
            status,
            content_range_header,
            content_length,
            current_file_size,
            expected_md5.is_some(),
        )?;

        let (mut file, start_offset, total_expected_size) = match action {
            ResponseAction::WriteFromStart {
                total_expected_size,
            } => (
                prepare_file_for_download(root, part_path, true)?,
                0,
                total_expected_size,
            ),
            ResponseAction::AppendFromOffset {
                total_expected_size,
            } => (
                prepare_file_for_download(root, part_path, false)?,
                current_file_size,
                total_expected_size,
            ),
            ResponseAction::VerifyCompletePart => {
                logger.log(
                    "download",
                    format_args!(
                        "HTTP {status} for '{}': verifying existing complete partial file ({current_file_size} bytes)",
                        part_path.display()
                    ),
                );
                if expected_md5.is_some() {
                    if !signal.is_running() {
                        return Err(interrupted_error());
                    }
                    let is_valid = verify_downloaded_file(
                        root,
                        part_path,
                        expected_md5,
                        signal.running_flag(),
                    )?;
                    if !signal.is_running() {
                        return Err(interrupted_error());
                    }
                    if is_valid {
                        logger.log(
                            "download",
                            format_args!(
                                "verified complete partial file '{}' ({current_file_size} bytes)",
                                part_path.display()
                            ),
                        );
                        return Ok((current_file_size, true));
                    }
                    if has_reset_range {
                        logger.log(
                            "download",
                            format_args!(
                                "complete partial file '{}' failed MD5 verification after range reset",
                                part_path.display()
                            ),
                        );
                        return Err(IaGetError::Integrity(format!(
                            "MD5 mismatch for {}",
                            part_path.display()
                        )));
                    }
                    logger.log(
                        "download",
                        format_args!(
                            "complete partial file '{}' failed MD5 verification; discarding partial file and refetching without Range",
                            part_path.display()
                        ),
                    );
                    has_reset_range = true;
                    remove_part_file_safely(root, part_path)?;
                    continue;
                }
                if !signal.is_running() {
                    return Err(interrupted_error());
                }
                logger.log(
                    "download",
                    format_args!(
                        "accepting complete partial file '{}' ({current_file_size} bytes; no MD5 in metadata)",
                        part_path.display()
                    ),
                );
                return Ok((current_file_size, false));
            }
            ResponseAction::DiscardAndRefetchUnranged => {
                if has_reset_range {
                    logger.log(
                        "download",
                        format_args!(
                            "HTTP {status} for '{}': repeated invalid range response after reset",
                            part_path.display()
                        ),
                    );
                    return Err(IaGetError::Network(format!(
                        "HTTP error {}: repeated invalid range response after reset",
                        status
                    )));
                }
                logger.log(
                    "download",
                    format_args!(
                        "HTTP {status} for '{}': discarding partial file ({current_file_size} bytes) and refetching without Range",
                        part_path.display()
                    ),
                );
                has_reset_range = true;
                remove_part_file_safely(root, part_path)?;
                continue;
            }
        };

        let download_action = if start_offset > 0 {
            format!("{} {}     ", "╰╼".cyan().dimmed(), "Resuming".white())
        } else {
            format!("{} {}  ", "╰╼".cyan().dimmed(), "Downloading".white())
        };

        let pb = create_progress_bar(
            total_expected_size,
            &download_action,
            Some("green/green"),
            true,
        );

        // Set initial progress to starting offset for resumed downloads
        pb.set_position(start_offset);

        let start_time = Instant::now();
        let mut rate_limiter = options.limit_rate.map(RateLimiter::new);
        let mut total_bytes: u64 = start_offset;
        let mut downloaded_bytes: u64 = 0;

        // Attempt the download
        let download_result: Result<()> = async {
            while let Some(chunk_result) = await_interruptible(response.chunk(), signal)
                .await?
                .transpose()
            {
                if !signal.is_running() {
                    pb.finish_and_clear();
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::Interrupted,
                        "Download interrupted during file transfer",
                    )
                    .into());
                }

                let chunk = chunk_result.map_err(reqwest::Error::without_url)?;
                file.write_all(&chunk)?;
                let chunk_len = chunk.len() as u64;
                downloaded_bytes = downloaded_bytes.saturating_add(chunk_len);
                total_bytes = total_bytes.saturating_add(chunk_len);
                pb.set_position(total_bytes);

                if let Some(limiter) = rate_limiter.as_mut() {
                    limiter.throttle(chunk_len, signal).await?;
                }
            }
            Ok(())
        }
        .await;

        match download_result {
            Ok(_) => {
                // Ensure data is written to disk and close handle before returning
                file.flush()?;
                drop(file);

                let elapsed = start_time.elapsed();
                let elapsed_secs = elapsed.as_secs_f64();
                let transfer_rate_val = if elapsed_secs > 0.0 {
                    downloaded_bytes as f64 / elapsed_secs
                } else {
                    0.0
                };

                let (rate, unit) = format_transfer_rate(transfer_rate_val);

                pb.finish_and_clear();
                println!(
                    "{} {}   {} {} in {} ({:.2} {}/s)",
                    "├╼".cyan().dimmed(),
                    "Downloaded".white(),
                    "↓".green().bold(),
                    format_size(downloaded_bytes).bold(),
                    format_duration(elapsed).bold(),
                    rate,
                    unit
                );

                return Ok((total_bytes, false));
            }
            Err(e) => {
                pb.finish_and_clear();

                // Fail immediately on user interruption or local filesystem errors
                if !signal.is_running() || !matches!(e, IaGetError::Network(_)) {
                    let _ = file.flush();
                    drop(file);
                    return Err(e);
                }

                if retry_count >= policy.max_retries {
                    let _ = file.flush();
                    drop(file);
                    logger.log(
                        "retry",
                        format_args!(
                            "exhausted ({}/{}) for {} mid-stream: {}",
                            retry_count, policy.max_retries, safe_initial_url, e
                        ),
                    );
                    println!(
                        "{} {}      {} Maximum retries ({}) exceeded",
                        "├╼".cyan().dimmed(),
                        "Failed".red().bold(),
                        "✘".red().bold(),
                        policy.max_retries
                    );
                    return Err(e);
                }

                retry_count += 1;
                let (delay, delay_source) =
                    compute_retry_delay_with_source(retry_count, None, SystemTime::now(), policy);
                logger.log(
                    "retry",
                    format_args!(
                        "attempt {}/{} for {} after mid-stream error ({}); waiting {:.2}s ({})",
                        retry_count,
                        policy.max_retries,
                        safe_initial_url,
                        e,
                        delay.as_secs_f64(),
                        delay_source
                    ),
                );
                println!(
                    "{} {}      {} Download error (attempt {}/{}): {}",
                    "├╼".cyan().dimmed(),
                    "Retry".yellow().bold(),
                    "⟳".yellow().bold(),
                    retry_count,
                    policy.max_retries,
                    e
                );
                println!(
                    "{} {}      Waiting {:.1}s before retry...",
                    "├╼".cyan().dimmed(),
                    "Wait".white(),
                    delay.as_secs_f64()
                );

                // Ensure file is flushed and closed before sleeping and starting next attempt
                file.flush()?;
                drop(file);

                sleep_interruptible(delay, signal).await?;
            }
        }
    }
}

/// Verify a downloaded file's hash against an expected value inside `root`.
fn verify_downloaded_file(
    root: &Path,
    rel_path: &Path,
    expected_md5: Option<&str>,
    running: &Arc<AtomicBool>,
) -> Result<bool> {
    let Some(expected_md5_str) = expected_md5 else {
        println!(
            "{} {}",
            "-".dimmed(),
            "No MD5 hash provided for verification.".dimmed()
        );
        return Ok(true); // No hash to check against, consider it verified
    };
    let local_md5 = calculate_md5(root, rel_path, running)?;
    if local_md5 == expected_md5_str {
        println!(
            "{} {}         {} {}",
            "╰╼".cyan().dimmed(),
            "Hash".white(),
            "✔".green().bold(),
            format!("({})", local_md5).dimmed()
        );
        Ok(true)
    } else {
        println!(
            "{} {}         {} ({}) Expected ({})",
            "╰╼".cyan().dimmed(),
            "Hash".white(),
            "✘".red().bold(),
            local_md5.red(),
            expected_md5_str.dimmed()
        );
        Ok(false)
    }
}

/// Outcome of processing a single file in a download session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileOutcome {
    Downloaded,
    Skipped,
}

/// Details of a single file that failed during a batch download.
#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct FailedFile {
    /// Local destination path of the file that failed.
    pub file_path: String,
    /// Error message explaining why the file failed.
    pub error: String,
}

/// Outcome summary of a batch download session.
#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct BatchSummary {
    /// Number of files downloaded and verified successfully.
    pub ok: usize,
    /// Number of existing files skipped because they were already complete.
    pub skipped: usize,
    /// Files that failed to download or failed verification.
    pub failed: Vec<FailedFile>,
    /// Whether the batch stopped early due to a user interrupt signal.
    pub interrupted: bool,
}

/// Formats a plain-text summary of a batch download session, including any failed files.
pub fn format_batch_summary(summary: &BatchSummary) -> String {
    let mut text = format!(
        "Summary: {} ok, {} skipped, {} failed",
        summary.ok,
        summary.skipped,
        summary.failed.len()
    );
    if summary.interrupted {
        text.push_str(", interrupted");
    }

    if !summary.failed.is_empty() {
        text.push_str("\nFailed files:");
        for failed in &summary.failed {
            text.push_str(&format!("\n  - {}: {}", failed.file_path, failed.error));
        }
    }

    text
}

/// Prints a coloured end-of-batch summary to stdout.
fn print_batch_summary(summary: &BatchSummary) {
    let status_icon = if summary.interrupted || !summary.failed.is_empty() {
        "✘".red().bold()
    } else {
        "✔".green().bold()
    };

    let failed_count = if summary.failed.is_empty() {
        summary.failed.len().to_string().bold()
    } else {
        summary.failed.len().to_string().red().bold()
    };

    let interrupted_suffix = if summary.interrupted {
        format!(", {}", "interrupted".yellow().bold())
    } else {
        String::new()
    };

    println!(
        "\n{} {} {} ok, {} skipped, {} failed{}",
        status_icon,
        "Summary:".bold(),
        summary.ok.to_string().green().bold(),
        summary.skipped.to_string().cyan().bold(),
        failed_count,
        interrupted_suffix
    );

    if !summary.failed.is_empty() {
        println!("{}", "Failed files:".red().bold());
        for failed in &summary.failed {
            println!(
                "  {} {}: {}",
                "✘".red().bold(),
                failed.file_path.bold(),
                failed.error
            );
        }
    }
}

/// Download a single file inside `root` via `.part` staging, MD5 verification, and atomic promotion
/// using explicit `DownloadOptions`.
#[allow(clippy::too_many_arguments)]
async fn download_single_file_with_options(
    client: &Client,
    root: &Path,
    url: &str,
    file_path: &str,
    expected_md5: Option<&str>,
    signal: &ShutdownSignal,
    cookie_source: Option<&CookieSource>,
    options: &DownloadOptions,
    logger: &VerboseLogger,
) -> Result<FileOutcome> {
    if !signal.is_running() {
        return Err(interrupted_error());
    }

    match check_existing_file(root, file_path, expected_md5, signal.running_flag())? {
        ExistingFileStatus::Missing => {}
        ExistingFileStatus::CompleteMd5Verified => {
            logger.log(
                "download",
                format_args!("skipped '{file_path}': already exists; MD5 verified"),
            );
            println!(
                "{} {}   {}",
                "╰╼".cyan().dimmed(),
                "Downloaded".white(),
                "✔".green().bold()
            );
            return Ok(FileOutcome::Skipped);
        }
        ExistingFileStatus::CompleteNoMd5 => {
            logger.log(
                "download",
                format_args!("skipped '{file_path}': already exists; no MD5 in metadata"),
            );
            println!(
                "{} {}   {}",
                "╰╼".cyan().dimmed(),
                "Downloaded".white(),
                "✔".green().bold()
            );
            return Ok(FileOutcome::Skipped);
        }
        ExistingFileStatus::NeedsRedownload => {
            logger.log(
                "download",
                format_args!("redownloading '{file_path}': existing file hash mismatch"),
            );
            println!(
                "{} {}      {}",
                "├╼".cyan().dimmed(),
                "Partial".white(),
                "▲".yellow().bold()
            );
        }
    }

    if !signal.is_running() {
        return Err(interrupted_error());
    }

    ensure_parent_directories(root, Path::new(file_path))?;

    let part_path = part_file_path(file_path);
    let (_, already_verified) = download_file_content(
        client,
        root,
        url,
        &part_path,
        signal,
        cookie_source,
        expected_md5,
        options,
        logger,
    )
    .await?;

    if !signal.is_running() {
        return Err(interrupted_error());
    }

    if already_verified
        || verify_downloaded_file(root, &part_path, expected_md5, signal.running_flag())?
    {
        if !signal.is_running() {
            return Err(interrupted_error());
        }
        promote_part_file(root, &part_path, Path::new(file_path))?;
        Ok(FileOutcome::Downloaded)
    } else {
        Err(IaGetError::Integrity(format!(
            "MD5 mismatch for {}",
            file_path
        )))
    }
}

/// Download a single file inside `root` via `.part` staging, MD5 verification, and atomic promotion
/// using an explicit `RetryPolicy`.
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
async fn download_single_file_with_policy(
    client: &Client,
    root: &Path,
    url: &str,
    file_path: &str,
    expected_md5: Option<&str>,
    signal: &ShutdownSignal,
    cookie_source: Option<&CookieSource>,
    policy: &RetryPolicy,
) -> Result<FileOutcome> {
    let options = DownloadOptions {
        retry_policy: *policy,
        limit_rate: None,
        verbose: false,
    };
    download_single_file_with_options(
        client,
        root,
        url,
        file_path,
        expected_md5,
        signal,
        cookie_source,
        &options,
        &VerboseLogger::disabled(),
    )
    .await
}

/// Download a single file inside `root` via `.part` staging, MD5 verification, and atomic promotion.
#[cfg(test)]
async fn download_single_file(
    client: &Client,
    root: &Path,
    url: &str,
    file_path: &str,
    expected_md5: Option<&str>,
    signal: &ShutdownSignal,
    cookie_source: Option<&CookieSource>,
) -> Result<FileOutcome> {
    download_single_file_with_policy(
        client,
        root,
        url,
        file_path,
        expected_md5,
        signal,
        cookie_source,
        &RetryPolicy::default(),
    )
    .await
}

/// Downloads multiple files into destination `root` with explicit [`DownloadOptions`],
/// shutdown signal, and [`VerboseLogger`].
///
/// Validates and prepares `root` before downloading, continues past per-file failures
/// after printing a warning, stops promptly on shutdown signal interruption, prints an
/// end-of-batch summary, and returns the collected [`BatchSummary`].
#[allow(clippy::too_many_arguments)]
pub async fn download_files_in_dir_with_logger<I>(
    client: &Client,
    root: &Path,
    files: I,
    total_files: usize,
    cookie_source: Option<&CookieSource>,
    options: &DownloadOptions,
    signal: &ShutdownSignal,
    logger: &VerboseLogger,
) -> Result<BatchSummary>
where
    I: IntoIterator<Item = (String, String, Option<String>)>, // (url, filename, md5)
{
    let files: Vec<(String, String, Option<String>)> = files.into_iter().collect();
    validate_download_paths_for_dir(
        root,
        files
            .iter()
            .map(|(_, file_path, _)| (file_path.as_str(), file_path.as_str())),
    )?;

    let canonical_root = prepare_output_dir(root)?;
    let mut summary = BatchSummary::default();

    for (index, (url, file_path, expected_md5)) in files.into_iter().enumerate() {
        if !signal.is_running() {
            summary.interrupted = true;
            logger.log(
                "download",
                format_args!(
                    "interrupted before file {}/{total_files} ('{file_path}')",
                    index + 1
                ),
            );
            println!(
                "\n{} Download interrupted. Run the command again to resume remaining files.",
                "✘".red().bold()
            );
            break;
        }

        println!(" ");
        println!(
            "{}  {}     {}",
            "▣".bright_cyan().bold(),
            "Filename".white(),
            file_path.bold()
        );
        println!(
            "{} {}        {} {} of {}",
            "├╼".cyan().dimmed(),
            "Count".white(),
            "#".blue().bold(),
            (index + 1).to_string().bold(),
            total_files.to_string().bold()
        );

        match download_single_file_with_options(
            client,
            &canonical_root,
            &url,
            &file_path,
            expected_md5.as_deref(),
            signal,
            cookie_source,
            options,
            logger,
        )
        .await
        {
            Ok(FileOutcome::Downloaded) => {
                summary.ok += 1;
                logger.log(
                    "download",
                    format_args!("completed '{file_path}' ({}/{total_files})", index + 1),
                );
            }
            Ok(FileOutcome::Skipped) => summary.skipped += 1,
            Err(err) => {
                if !signal.is_running() {
                    summary.interrupted = true;
                    logger.log(
                        "download",
                        format_args!(
                            "interrupted while downloading '{file_path}' ({}/{total_files})",
                            index + 1
                        ),
                    );
                    println!(
                        "\n{} Download interrupted. Run the command again to resume remaining files.",
                        "✘".red().bold()
                    );
                    break;
                }

                logger.log("download", format_args!("failed '{file_path}': {err}"));
                println!(
                    "{} {}      {} {}: {}",
                    "╰╼".cyan().dimmed(),
                    "Warning".yellow().bold(),
                    "⚠".yellow().bold(),
                    file_path.bold(),
                    err
                );
                summary.failed.push(FailedFile {
                    file_path,
                    error: err.to_string(),
                });
            }
        }
    }

    logger.log(
        "summary",
        format_args!(
            "ok={} skipped={} failed={} interrupted={}",
            summary.ok,
            summary.skipped,
            summary.failed.len(),
            summary.interrupted
        ),
    );
    print_batch_summary(&summary);
    Ok(summary)
}

/// Downloads multiple files into destination `root` with explicit [`DownloadOptions`] and
/// shutdown signal.
///
/// Validates and prepares `root` before downloading, continues past per-file failures
/// after printing a warning, stops promptly on shutdown signal interruption, prints an
/// end-of-batch summary, and returns the collected [`BatchSummary`].
pub async fn download_files_in_dir_with_options<I>(
    client: &Client,
    root: &Path,
    files: I,
    total_files: usize,
    cookie_source: Option<&CookieSource>,
    options: &DownloadOptions,
    signal: &ShutdownSignal,
) -> Result<BatchSummary>
where
    I: IntoIterator<Item = (String, String, Option<String>)>, // (url, filename, md5)
{
    let logger = VerboseLogger::from_enabled(options.verbose);
    download_files_in_dir_with_logger(
        client,
        root,
        files,
        total_files,
        cookie_source,
        options,
        signal,
        &logger,
    )
    .await
}

/// Downloads multiple files into destination `root` with an explicit retry policy and shutdown signal.
///
/// Validates and prepares `root` before downloading, continues past per-file failures
/// after printing a warning, stops promptly on shutdown signal interruption, prints an
/// end-of-batch summary, and returns the collected [`BatchSummary`].
pub async fn download_files_in_dir<I>(
    client: &Client,
    root: &Path,
    files: I,
    total_files: usize,
    cookie_source: Option<&CookieSource>,
    policy: &RetryPolicy,
    signal: &ShutdownSignal,
) -> Result<BatchSummary>
where
    I: IntoIterator<Item = (String, String, Option<String>)>, // (url, filename, md5)
{
    let options = DownloadOptions {
        retry_policy: *policy,
        limit_rate: None,
        verbose: false,
    };
    download_files_in_dir_with_options(
        client,
        root,
        files,
        total_files,
        cookie_source,
        &options,
        signal,
    )
    .await
}

/// Downloads multiple files with an explicit retry policy and shutdown signal.
///
/// Continues past per-file failures after printing a warning, stops promptly on
/// shutdown signal interruption, prints an end-of-batch summary, and returns the
/// collected [`BatchSummary`].
pub async fn download_files_with_policy_and_signal<I>(
    client: &Client,
    files: I,
    total_files: usize,
    cookie_source: Option<&CookieSource>,
    policy: &RetryPolicy,
    signal: &ShutdownSignal,
) -> Result<BatchSummary>
where
    I: IntoIterator<Item = (String, String, Option<String>)>, // (url, filename, md5)
{
    let options = DownloadOptions {
        retry_policy: *policy,
        limit_rate: None,
        verbose: false,
    };
    download_files_in_dir_with_options(
        client,
        Path::new("."),
        files,
        total_files,
        cookie_source,
        &options,
        signal,
    )
    .await
}

/// Download multiple files with shared signal handling.
///
/// Prepares `output_dir` (defaulting to the current working directory when `None`),
/// sets up signal handling once for the entire download session, continues past
/// individual file failures, prints an end-of-batch summary, and returns the
/// collected [`BatchSummary`].
pub async fn download_files<I>(
    client: &Client,
    output_dir: Option<&Path>,
    files: I,
    total_files: usize,
    cookie_source: Option<&CookieSource>,
) -> Result<BatchSummary>
where
    I: IntoIterator<Item = (String, String, Option<String>)>, // (url, filename, md5)
{
    let root = output_dir.unwrap_or_else(|| Path::new("."));
    let signal = install_signal_handler();
    download_files_in_dir_with_options(
        client,
        root,
        files,
        total_files,
        cookie_source,
        &DownloadOptions::default(),
        &signal,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_content_range_valid_bytes() {
        assert_eq!(
            parse_content_range("bytes 0-99/1000"),
            Some(ContentRange::Bytes {
                start: 0,
                end: 99,
                complete_length: Some(1000),
            })
        );
        assert_eq!(
            parse_content_range("bytes 0-0/1"),
            Some(ContentRange::Bytes {
                start: 0,
                end: 0,
                complete_length: Some(1),
            })
        );
        assert_eq!(
            parse_content_range("bytes 100-199/*"),
            Some(ContentRange::Bytes {
                start: 100,
                end: 199,
                complete_length: None,
            })
        );
        assert_eq!(
            parse_content_range("BYTES 50-50/51"),
            Some(ContentRange::Bytes {
                start: 50,
                end: 50,
                complete_length: Some(51),
            })
        );
        assert_eq!(
            parse_content_range("Bytes 10-20/*"),
            Some(ContentRange::Bytes {
                start: 10,
                end: 20,
                complete_length: None,
            })
        );
        assert_eq!(
            parse_content_range("  bytes 0-9/10  "),
            Some(ContentRange::Bytes {
                start: 0,
                end: 9,
                complete_length: Some(10),
            })
        );
        assert_eq!(
            parse_content_range(&format!("bytes 0-{}/{}", u64::MAX - 1, u64::MAX)),
            Some(ContentRange::Bytes {
                start: 0,
                end: u64::MAX - 1,
                complete_length: Some(u64::MAX),
            })
        );
        assert_eq!(
            parse_content_range(&format!("bytes {}-{}/*", u64::MAX, u64::MAX)),
            Some(ContentRange::Bytes {
                start: u64::MAX,
                end: u64::MAX,
                complete_length: None,
            })
        );
    }

    #[test]
    fn test_parse_content_range_valid_unsatisfied() {
        assert_eq!(
            parse_content_range("bytes */1234"),
            Some(ContentRange::Unsatisfied {
                complete_length: 1234,
            })
        );
        assert_eq!(
            parse_content_range("bytes */0"),
            Some(ContentRange::Unsatisfied { complete_length: 0 })
        );
        assert_eq!(
            parse_content_range("ByTeS */42"),
            Some(ContentRange::Unsatisfied {
                complete_length: 42,
            })
        );
        assert_eq!(
            parse_content_range(&format!("  bytes */{}  ", u64::MAX)),
            Some(ContentRange::Unsatisfied {
                complete_length: u64::MAX,
            })
        );
    }

    #[test]
    fn test_parse_content_range_rejects_invalid_headers() {
        let invalid_cases = [
            "",
            "bytes",
            "bytes ",
            "bytes */*",
            "items 0-9/10",
            "octets */10",
            "bytes 10-5/20",
            "bytes 10-5/*",
            "bytes 0-10/10",
            "bytes 0-11/10",
            "bytes 0-0/0",
            "bytes -1-9/10",
            "bytes 0--9/10",
            "bytes 0-9/-10",
            "bytes */-1",
            "bytes +0-9/10",
            "bytes 0-+9/10",
            "bytes 0-9/+10",
            "bytes a-9/10",
            "bytes 0-b/10",
            "bytes 0-9/c",
            "bytes */abc",
            "bytes 0-18446744073709551616/*",
            "bytes 18446744073709551616-18446744073709551616/*",
            "bytes 0-1/18446744073709551616",
            "bytes */18446744073709551616",
            "bytes 0-9/10 extra",
            "bytes 0-9/* extra",
            "bytes */10 extra",
            "bytes 0-9/10/20",
            "bytes 0-9-10/20",
            "bytes  0-9/10",
            "bytes 0-9",
            "bytes /10",
        ];

        for header in invalid_cases {
            assert_eq!(
                parse_content_range(header),
                None,
                "expected None for header: {header:?}"
            );
        }
    }

    #[test]
    fn test_compute_progress_total() {
        assert_eq!(compute_progress_total(0, Some(500), None), 500);
        assert_eq!(compute_progress_total(100, Some(400), Some(1000)), 1000);
        assert_eq!(compute_progress_total(100, Some(400), None), 500);
        assert_eq!(compute_progress_total(100, None, None), 100);
        assert_eq!(compute_progress_total(0, None, None), 0);
        assert_eq!(compute_progress_total(100, Some(u64::MAX), None), u64::MAX);
        assert_eq!(compute_progress_total(u64::MAX, Some(1), None), u64::MAX);
        assert_eq!(
            compute_progress_total(u64::MAX, Some(u64::MAX), None),
            u64::MAX
        );
        assert_eq!(compute_progress_total(100, Some(u64::MAX), Some(500)), 500);
    }

    #[test]
    fn test_classify_download_response_ok() {
        assert_eq!(
            classify_download_response(StatusCode::OK, None, Some(1024), 0, true).unwrap(),
            ResponseAction::WriteFromStart {
                total_expected_size: 1024,
            }
        );
        assert_eq!(
            classify_download_response(StatusCode::OK, None, Some(1024), 256, true).unwrap(),
            ResponseAction::WriteFromStart {
                total_expected_size: 1024,
            }
        );
        assert_eq!(
            classify_download_response(StatusCode::OK, None, None, 256, false).unwrap(),
            ResponseAction::WriteFromStart {
                total_expected_size: 0,
            }
        );
    }

    #[test]
    fn test_classify_download_response_partial_content() {
        let err = classify_download_response(
            StatusCode::PARTIAL_CONTENT,
            Some("bytes 0-99/200"),
            Some(100),
            0,
            true,
        )
        .unwrap_err();
        assert!(matches!(err, IaGetError::Network(_)));

        assert_eq!(
            classify_download_response(
                StatusCode::PARTIAL_CONTENT,
                Some("bytes 100-199/500"),
                Some(100),
                100,
                true,
            )
            .unwrap(),
            ResponseAction::AppendFromOffset {
                total_expected_size: 500,
            }
        );

        assert_eq!(
            classify_download_response(
                StatusCode::PARTIAL_CONTENT,
                Some("bytes 100-199/*"),
                Some(100),
                100,
                false,
            )
            .unwrap(),
            ResponseAction::AppendFromOffset {
                total_expected_size: 200,
            }
        );

        assert_eq!(
            classify_download_response(
                StatusCode::PARTIAL_CONTENT,
                Some("bytes 100-199/*"),
                Some(u64::MAX),
                100,
                false,
            )
            .unwrap(),
            ResponseAction::AppendFromOffset {
                total_expected_size: u64::MAX,
            }
        );

        assert_eq!(
            classify_download_response(
                StatusCode::PARTIAL_CONTENT,
                Some("bytes 50-199/500"),
                Some(150),
                100,
                true,
            )
            .unwrap(),
            ResponseAction::DiscardAndRefetchUnranged
        );

        assert_eq!(
            classify_download_response(StatusCode::PARTIAL_CONTENT, None, Some(100), 100, true)
                .unwrap(),
            ResponseAction::DiscardAndRefetchUnranged
        );

        assert_eq!(
            classify_download_response(
                StatusCode::PARTIAL_CONTENT,
                Some("bytes */100"),
                Some(0),
                100,
                true,
            )
            .unwrap(),
            ResponseAction::DiscardAndRefetchUnranged
        );

        assert_eq!(
            classify_download_response(
                StatusCode::PARTIAL_CONTENT,
                Some("bytes 150-199/500"),
                Some(50),
                100,
                true,
            )
            .unwrap(),
            ResponseAction::DiscardAndRefetchUnranged
        );

        assert_eq!(
            classify_download_response(
                StatusCode::PARTIAL_CONTENT,
                Some("not-a-valid-range"),
                Some(100),
                100,
                true,
            )
            .unwrap(),
            ResponseAction::DiscardAndRefetchUnranged
        );
    }

    #[test]
    fn test_classify_download_response_range_not_satisfiable() {
        let err = classify_download_response(
            StatusCode::RANGE_NOT_SATISFIABLE,
            Some("bytes */0"),
            Some(0),
            0,
            true,
        )
        .unwrap_err();
        assert!(matches!(err, IaGetError::Network(_)));

        assert_eq!(
            classify_download_response(
                StatusCode::RANGE_NOT_SATISFIABLE,
                Some("bytes */500"),
                None,
                500,
                false,
            )
            .unwrap(),
            ResponseAction::VerifyCompletePart
        );

        assert_eq!(
            classify_download_response(
                StatusCode::RANGE_NOT_SATISFIABLE,
                Some("bytes */400"),
                None,
                500,
                true,
            )
            .unwrap(),
            ResponseAction::DiscardAndRefetchUnranged
        );

        assert_eq!(
            classify_download_response(
                StatusCode::RANGE_NOT_SATISFIABLE,
                Some("bytes */600"),
                None,
                500,
                true,
            )
            .unwrap(),
            ResponseAction::DiscardAndRefetchUnranged
        );

        assert_eq!(
            classify_download_response(
                StatusCode::RANGE_NOT_SATISFIABLE,
                Some("bytes 0-99/500"),
                None,
                500,
                true,
            )
            .unwrap(),
            ResponseAction::DiscardAndRefetchUnranged
        );

        assert_eq!(
            classify_download_response(StatusCode::RANGE_NOT_SATISFIABLE, None, None, 500, true)
                .unwrap(),
            ResponseAction::VerifyCompletePart
        );

        assert_eq!(
            classify_download_response(StatusCode::RANGE_NOT_SATISFIABLE, None, None, 500, false)
                .unwrap(),
            ResponseAction::DiscardAndRefetchUnranged
        );

        assert_eq!(
            classify_download_response(
                StatusCode::RANGE_NOT_SATISFIABLE,
                Some("invalid"),
                None,
                500,
                true,
            )
            .unwrap(),
            ResponseAction::VerifyCompletePart
        );

        assert_eq!(
            classify_download_response(
                StatusCode::RANGE_NOT_SATISFIABLE,
                Some("invalid"),
                None,
                500,
                false,
            )
            .unwrap(),
            ResponseAction::DiscardAndRefetchUnranged
        );
    }

    #[test]
    fn test_classify_download_response_other_statuses() {
        for status in [
            StatusCode::NOT_FOUND,
            StatusCode::FORBIDDEN,
            StatusCode::INTERNAL_SERVER_ERROR,
            StatusCode::FOUND,
        ] {
            for offset in [0, 256] {
                let err =
                    classify_download_response(status, None, Some(128), offset, true).unwrap_err();
                assert!(matches!(
                    err,
                    IaGetError::Network(ref message) if message == &format!("HTTP error {}", status)
                ));
            }
        }
    }

    #[test]
    fn test_part_file_path() {
        assert_eq!(part_file_path("file.zip"), PathBuf::from("file.zip.part"));
        assert_eq!(
            part_file_path("archive.tar.gz"),
            PathBuf::from("archive.tar.gz.part")
        );
        assert_eq!(part_file_path("README"), PathBuf::from("README.part"));
        assert_eq!(
            part_file_path("dir.v1/sub.dir/data.2026.tar.xz"),
            PathBuf::from("dir.v1/sub.dir/data.2026.tar.xz.part")
        );
        assert_eq!(part_file_path(".hidden"), PathBuf::from(".hidden.part"));
        assert_eq!(
            part_file_path("file name (1).bin"),
            PathBuf::from("file name (1).bin.part")
        );
    }

    use std::sync::atomic::AtomicU64;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::Mutex;

    static TEMP_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TempTestDir {
        path: PathBuf,
    }

    impl TempTestDir {
        fn new(prefix: &str) -> Self {
            let id = TEMP_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "ia-get-test-{}-{}-{}",
                prefix,
                std::process::id(),
                id
            ));
            fs::create_dir_all(&path).expect("create temp test dir");
            Self { path }
        }

        fn join(&self, name: &str) -> PathBuf {
            self.path.join(name)
        }

        fn part_path(&self, name: &str) -> PathBuf {
            self.path.join(part_file_path(name))
        }
    }

    impl Drop for TempTestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn md5_hex(data: &[u8]) -> String {
        format!("{:x}", md5::compute(data))
    }

    #[derive(Debug, Clone)]
    struct CapturedRequest {
        request_line: String,
        headers: Vec<(String, String)>,
    }

    impl CapturedRequest {
        fn header(&self, name: &str) -> Option<&str> {
            self.headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.as_str())
        }
    }

    enum MockResponse {
        Static {
            status_line: &'static str,
            headers: Vec<(String, String)>,
            body: Vec<u8>,
        },
        MidStreamInterrupt {
            status_line: &'static str,
            headers: Vec<(String, String)>,
            first_chunk: Vec<u8>,
            second_chunk: Option<Vec<u8>>,
            watch_path: PathBuf,
            expected_part_len: u64,
            signal: ShutdownSignal,
        },
    }

    impl MockResponse {
        fn ok(body: &[u8]) -> Self {
            Self::Static {
                status_line: "200 OK",
                headers: vec![("Content-Length".to_string(), body.len().to_string())],
                body: body.to_vec(),
            }
        }

        fn with_headers(
            status_line: &'static str,
            headers: Vec<(&str, String)>,
            body: &[u8],
        ) -> Self {
            Self::Static {
                status_line,
                headers: headers
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), v))
                    .collect(),
                body: body.to_vec(),
            }
        }
    }

    struct MockServer {
        host: &'static str,
        addr: std::net::SocketAddr,
        requests: Arc<Mutex<Vec<CapturedRequest>>>,
        handle: Option<tokio::task::JoinHandle<()>>,
    }

    impl MockServer {
        async fn start(responses: Vec<MockResponse>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind mock server");
            Self::start_with_listener(listener, "127.0.0.1", responses)
        }

        async fn start_on_localhost(responses: Vec<MockResponse>) -> Self {
            let listener = TcpListener::bind(("localhost", 0))
                .await
                .expect("bind localhost mock server");
            Self::start_with_listener(listener, "localhost", responses)
        }

        fn start_with_listener(
            listener: TcpListener,
            host: &'static str,
            responses: Vec<MockResponse>,
        ) -> Self {
            let addr = listener.local_addr().expect("mock server local_addr");
            let requests = Arc::new(Mutex::new(Vec::new()));
            let requests_clone = Arc::clone(&requests);

            let handle = tokio::spawn(async move {
                for response in responses {
                    let Ok((mut socket, _)) = listener.accept().await else {
                        break;
                    };
                    let _ = socket.set_nodelay(true);
                    if let Some(req) = read_http_request(&mut socket).await {
                        requests_clone.lock().await.push(req);
                    }
                    write_mock_response(&mut socket, response).await;
                }
            });

            Self {
                host,
                addr,
                requests,
                handle: Some(handle),
            }
        }

        fn url(&self, path: &str) -> String {
            format!("http://{}:{}{}", self.host, self.addr.port(), path)
        }

        fn requests_handle(&self) -> Arc<Mutex<Vec<CapturedRequest>>> {
            Arc::clone(&self.requests)
        }

        async fn finish(mut self) -> Vec<CapturedRequest> {
            if let Some(handle) = self.handle.take() {
                let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;
            }
            self.requests.lock().await.clone()
        }

        async fn abort_and_collect(mut self) -> Vec<CapturedRequest> {
            if let Some(handle) = self.handle.take() {
                handle.abort();
                let _ = handle.await;
            }
            self.requests.lock().await.clone()
        }
    }

    impl Drop for MockServer {
        fn drop(&mut self) {
            if let Some(handle) = self.handle.take() {
                handle.abort();
            }
        }
    }

    async fn read_http_request(socket: &mut TcpStream) -> Option<CapturedRequest> {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 1024];
        loop {
            let n = socket.read(&mut chunk).await.ok()?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }

        let header_end = buf
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .unwrap_or(buf.len());
        let header_text = String::from_utf8_lossy(&buf[..header_end]);
        let mut lines = header_text.split("\r\n");
        let request_line = lines.next()?.to_string();
        let mut headers = Vec::new();
        for line in lines {
            if let Some((name, value)) = line.split_once(':') {
                headers.push((name.trim().to_string(), value.trim().to_string()));
            }
        }

        Some(CapturedRequest {
            request_line,
            headers,
        })
    }

    fn format_response_head(status_line: &str, headers: &[(String, String)]) -> Vec<u8> {
        let mut head = format!("HTTP/1.1 {}\r\n", status_line);
        let mut has_connection = false;
        for (name, value) in headers {
            if name.eq_ignore_ascii_case("connection") {
                has_connection = true;
            }
            head.push_str(name);
            head.push_str(": ");
            head.push_str(value);
            head.push_str("\r\n");
        }
        if !has_connection {
            head.push_str("Connection: close\r\n");
        }
        head.push_str("\r\n");
        head.into_bytes()
    }

    async fn write_mock_response(socket: &mut TcpStream, response: MockResponse) {
        match response {
            MockResponse::Static {
                status_line,
                headers,
                body,
            } => {
                let mut wire = format_response_head(status_line, &headers);
                wire.extend_from_slice(&body);
                let _ = socket.write_all(&wire).await;
                let _ = socket.flush().await;
                let _ = socket.shutdown().await;
            }
            MockResponse::MidStreamInterrupt {
                status_line,
                headers,
                first_chunk,
                second_chunk,
                watch_path,
                expected_part_len,
                signal,
            } => {
                let mut wire = format_response_head(status_line, &headers);
                wire.extend_from_slice(&first_chunk);
                let _ = socket.write_all(&wire).await;
                let _ = socket.flush().await;

                let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
                while tokio::time::Instant::now() < deadline {
                    if fs::metadata(&watch_path)
                        .map(|m| m.len() >= expected_part_len)
                        .unwrap_or(false)
                    {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }

                signal.trigger();

                if let Some(chunk) = second_chunk {
                    let _ = socket.write_all(&chunk).await;
                    let _ = socket.flush().await;
                }
                let _ = socket.shutdown().await;
            }
        }
    }

    #[tokio::test]
    async fn test_download_fresh_http_200_verifies_md5_and_promotes_part() {
        let payload = b"fresh archive download payload 1234567890";
        let expected_md5 = md5_hex(payload);
        let server = MockServer::start(vec![MockResponse::ok(payload)]).await;
        let dir = TempTestDir::new("fresh-200");
        let rel_path = "nested/item.bin";
        let final_path = dir.join(rel_path);
        let part_path = dir.part_path(rel_path);
        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();

        download_single_file(
            &client,
            &dir.path,
            &server.url("/item.bin"),
            rel_path,
            Some(&expected_md5),
            &signal,
            None,
        )
        .await
        .expect("fresh 200 download should succeed");

        let requests = server.finish().await;
        assert_eq!(requests.len(), 1);
        assert!(requests[0].request_line.starts_with("GET /item.bin "));
        assert_eq!(requests[0].header("range"), None);
        assert_eq!(fs::read(&final_path).unwrap(), payload);
        assert!(!part_path.exists(), ".part file should be promoted away");
    }

    #[tokio::test]
    async fn test_download_interrupted_mid_stream_leaves_part_without_final_file() {
        let first_chunk = b"0123456789";
        let second_chunk = b"abcdefghij";
        let total_len = first_chunk.len() + second_chunk.len();
        let dir = TempTestDir::new("interrupt");
        let rel_path = "interrupted.bin";
        let final_path = dir.join(rel_path);
        let part_path = dir.part_path(rel_path);
        let signal = ShutdownSignal::new();

        let server = MockServer::start(vec![MockResponse::MidStreamInterrupt {
            status_line: "200 OK",
            headers: vec![("Content-Length".to_string(), total_len.to_string())],
            first_chunk: first_chunk.to_vec(),
            second_chunk: Some(second_chunk.to_vec()),
            watch_path: part_path.clone(),
            expected_part_len: first_chunk.len() as u64,
            signal: signal.clone(),
        }])
        .await;

        let client = http_client_builder().build().unwrap();
        let err = download_single_file(
            &client,
            &dir.path,
            &server.url("/interrupted.bin"),
            rel_path,
            None,
            &signal,
            None,
        )
        .await
        .unwrap_err();

        let requests = server.finish().await;
        assert_eq!(requests.len(), 1);
        assert!(matches!(err, IaGetError::FileSystem(_)));
        assert!(
            !final_path.exists(),
            "interrupted download must not create final file"
        );
        assert_eq!(
            fs::read(&part_path).unwrap(),
            first_chunk,
            ".part file should retain bytes received before interrupt"
        );
    }

    #[tokio::test]
    async fn test_download_resume_http_206_appends_and_promotes() {
        let prefix = b"hello ";
        let suffix = b"resumed archive world!";
        let mut full = Vec::new();
        full.extend_from_slice(prefix);
        full.extend_from_slice(suffix);
        let expected_md5 = md5_hex(&full);

        let dir = TempTestDir::new("resume-206");
        let rel_path = "resumed.bin";
        let final_path = dir.join(rel_path);
        let part_path = dir.part_path(rel_path);
        fs::write(&part_path, prefix).unwrap();

        let content_range = format!("bytes {}-{}/{}", prefix.len(), full.len() - 1, full.len());
        let server = MockServer::start(vec![MockResponse::with_headers(
            "206 Partial Content",
            vec![
                ("Content-Range", content_range),
                ("Content-Length", suffix.len().to_string()),
            ],
            suffix,
        )])
        .await;

        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();
        download_single_file(
            &client,
            &dir.path,
            &server.url("/resumed.bin"),
            rel_path,
            Some(&expected_md5),
            &signal,
            None,
        )
        .await
        .expect("resumed 206 download should succeed");

        let requests = server.finish().await;
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].header("range"),
            Some(format!("bytes={}-", prefix.len()).as_str())
        );
        assert_eq!(fs::read(&final_path).unwrap(), full);
        assert!(!part_path.exists());
    }

    #[tokio::test]
    async fn test_download_http_200_to_ranged_request_restarts_without_duplication() {
        let stale_prefix = b"stale-prefix-bytes";
        let full_payload = b"complete fresh payload from byte zero";
        let expected_md5 = md5_hex(full_payload);

        let dir = TempTestDir::new("ranged-200");
        let rel_path = "restarted.bin";
        let final_path = dir.join(rel_path);
        let part_path = dir.part_path(rel_path);
        fs::write(&part_path, stale_prefix).unwrap();

        let server = MockServer::start(vec![MockResponse::ok(full_payload)]).await;
        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();

        download_single_file(
            &client,
            &dir.path,
            &server.url("/restarted.bin"),
            rel_path,
            Some(&expected_md5),
            &signal,
            None,
        )
        .await
        .expect("HTTP 200 to ranged request should restart from byte 0");

        let requests = server.finish().await;
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].header("range"),
            Some(format!("bytes={}-", stale_prefix.len()).as_str())
        );
        assert_eq!(fs::read(&final_path).unwrap(), full_payload);
        assert!(!part_path.exists());
    }

    #[tokio::test]
    async fn test_download_http_206_mismatched_content_range_discards_and_refetches_unranged() {
        let stale_prefix = b"stale-01";
        let full_payload = b"0123456789abcdefghij";
        let expected_md5 = md5_hex(full_payload);

        let dir = TempTestDir::new("mismatched-206");
        let rel_path = "mismatched.bin";
        let final_path = dir.join(rel_path);
        let part_path = dir.part_path(rel_path);
        fs::write(&part_path, stale_prefix).unwrap();

        let server = MockServer::start(vec![
            MockResponse::with_headers(
                "206 Partial Content",
                vec![
                    (
                        "Content-Range",
                        format!("bytes 0-{}/{}", full_payload.len() - 1, full_payload.len()),
                    ),
                    ("Content-Length", full_payload.len().to_string()),
                ],
                full_payload,
            ),
            MockResponse::ok(full_payload),
        ])
        .await;

        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();
        download_single_file(
            &client,
            &dir.path,
            &server.url("/mismatched.bin"),
            rel_path,
            Some(&expected_md5),
            &signal,
            None,
        )
        .await
        .expect("mismatched Content-Range 206 should discard and refetch cleanly");

        let requests = server.finish().await;
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[0].header("range"),
            Some(format!("bytes={}-", stale_prefix.len()).as_str())
        );
        assert_eq!(
            requests[1].header("range"),
            None,
            "refetch after mismatched 206 must be unranged"
        );
        assert_eq!(fs::read(&final_path).unwrap(), full_payload);
        assert!(!part_path.exists());
    }

    #[tokio::test]
    async fn test_download_http_206_missing_and_malformed_content_range_refetches_unranged() {
        let full_payload = b"clean recovery payload after bad 206";
        let expected_md5 = md5_hex(full_payload);
        let stale_prefix = b"partial";

        for (label, bad_headers) in [
            ("missing", vec![("Content-Length", "4".to_string())]),
            (
                "malformed",
                vec![
                    ("Content-Range", "bytes not-valid".to_string()),
                    ("Content-Length", "4".to_string()),
                ],
            ),
        ] {
            let dir = TempTestDir::new(label);
            let rel_path = "bad206.bin";
            let final_path = dir.join(rel_path);
            let part_path = dir.part_path(rel_path);
            fs::write(&part_path, stale_prefix).unwrap();

            let server = MockServer::start(vec![
                MockResponse::with_headers("206 Partial Content", bad_headers, b"junk"),
                MockResponse::ok(full_payload),
            ])
            .await;

            let client = http_client_builder().build().unwrap();
            let signal = ShutdownSignal::new();
            download_single_file(
                &client,
                &dir.path,
                &server.url("/bad206.bin"),
                rel_path,
                Some(&expected_md5),
                &signal,
                None,
            )
            .await
            .expect("missing or malformed Content-Range 206 should recover via unranged 200");

            let requests = server.finish().await;
            assert_eq!(requests.len(), 2, "case {label}");
            assert_eq!(
                requests[0].header("range"),
                Some(format!("bytes={}-", stale_prefix.len()).as_str()),
                "case {label}"
            );
            assert_eq!(requests[1].header("range"), None, "case {label}");
            assert_eq!(fs::read(&final_path).unwrap(), full_payload, "case {label}");
            assert!(!part_path.exists(), "case {label}");
        }
    }

    #[tokio::test]
    async fn test_download_http_206_repeated_on_unranged_retry_fails_with_network_error() {
        let stale_prefix = b"partial";
        let dir = TempTestDir::new("repeated-206");
        let rel_path = "repeated206.bin";
        let final_path = dir.join(rel_path);
        let part_path = dir.part_path(rel_path);
        fs::write(&part_path, stale_prefix).unwrap();

        let server = MockServer::start(vec![
            MockResponse::with_headers(
                "206 Partial Content",
                vec![("Content-Length", "4".to_string())],
                b"junk",
            ),
            MockResponse::with_headers(
                "206 Partial Content",
                vec![
                    ("Content-Range", "bytes 0-3/10".to_string()),
                    ("Content-Length", "4".to_string()),
                ],
                b"junk",
            ),
        ])
        .await;

        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();
        let err = download_single_file(
            &client,
            &dir.path,
            &server.url("/repeated206.bin"),
            rel_path,
            None,
            &signal,
            None,
        )
        .await
        .unwrap_err();

        let requests = server.finish().await;
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[0].header("range"),
            Some(format!("bytes={}-", stale_prefix.len()).as_str())
        );
        assert_eq!(requests[1].header("range"), None);
        assert!(matches!(err, IaGetError::Network(_)));
        assert!(!final_path.exists());
        assert!(!part_path.exists());
    }

    #[tokio::test]
    async fn test_download_http_416_matching_complete_part_verifies_in_place() {
        let full_payload = b"already complete part payload";
        let expected_md5 = md5_hex(full_payload);

        let dir = TempTestDir::new("416-complete");
        let rel_path = "complete.bin";
        let final_path = dir.join(rel_path);
        let part_path = dir.part_path(rel_path);
        fs::write(&part_path, full_payload).unwrap();

        let server = MockServer::start(vec![MockResponse::with_headers(
            "416 Range Not Satisfiable",
            vec![
                ("Content-Range", format!("bytes */{}", full_payload.len())),
                ("Content-Length", "0".to_string()),
            ],
            b"",
        )])
        .await;

        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();
        download_single_file(
            &client,
            &dir.path,
            &server.url("/complete.bin"),
            rel_path,
            Some(&expected_md5),
            &signal,
            None,
        )
        .await
        .expect("416 with matching complete length should verify and promote .part");

        let requests = server.finish().await;
        assert_eq!(requests.len(), 1, "should not re-download complete .part");
        assert_eq!(
            requests[0].header("range"),
            Some(format!("bytes={}-", full_payload.len()).as_str())
        );
        assert_eq!(fs::read(&final_path).unwrap(), full_payload);
        assert!(!part_path.exists());
    }

    #[tokio::test]
    async fn test_download_http_416_oversized_part_discards_and_refetches_unranged() {
        let full_payload = b"exact server bytes";
        let expected_md5 = md5_hex(full_payload);
        let oversized_part = b"exact server bytes PLUS extra stale trailing data";

        let dir = TempTestDir::new("416-oversized");
        let rel_path = "oversized.bin";
        let final_path = dir.join(rel_path);
        let part_path = dir.part_path(rel_path);
        fs::write(&part_path, oversized_part).unwrap();

        let server = MockServer::start(vec![
            MockResponse::with_headers(
                "416 Range Not Satisfiable",
                vec![
                    ("Content-Range", format!("bytes */{}", full_payload.len())),
                    ("Content-Length", "0".to_string()),
                ],
                b"",
            ),
            MockResponse::ok(full_payload),
        ])
        .await;

        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();
        download_single_file(
            &client,
            &dir.path,
            &server.url("/oversized.bin"),
            rel_path,
            Some(&expected_md5),
            &signal,
            None,
        )
        .await
        .expect("416 with smaller complete length should discard oversized .part and refetch");

        let requests = server.finish().await;
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[0].header("range"),
            Some(format!("bytes={}-", oversized_part.len()).as_str())
        );
        assert_eq!(requests[1].header("range"), None);
        assert_eq!(fs::read(&final_path).unwrap(), full_payload);
        assert!(!part_path.exists());
    }

    #[tokio::test]
    async fn test_download_http_416_corrupt_equal_length_part_discards_and_refetches_unranged() {
        let full_payload = b"valid complete archive payload!!";
        let corrupt_part = b"corrupt equal length part data!!";
        assert_eq!(corrupt_part.len(), full_payload.len());
        assert_ne!(corrupt_part, full_payload);
        let expected_md5 = md5_hex(full_payload);

        let dir = TempTestDir::new("416-corrupt-equal-len");
        let rel_path = "corrupt-complete.bin";
        let final_path = dir.join(rel_path);
        let part_path = dir.part_path(rel_path);
        fs::write(&part_path, corrupt_part).unwrap();

        let server = MockServer::start(vec![
            MockResponse::with_headers(
                "416 Range Not Satisfiable",
                vec![
                    ("Content-Range", format!("bytes */{}", full_payload.len())),
                    ("Content-Length", "0".to_string()),
                ],
                b"",
            ),
            MockResponse::ok(full_payload),
        ])
        .await;

        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();
        download_single_file(
            &client,
            &dir.path,
            &server.url("/corrupt-complete.bin"),
            rel_path,
            Some(&expected_md5),
            &signal,
            None,
        )
        .await
        .expect("416 on corrupt equal-length .part should discard and refetch unranged");

        let requests = server.finish().await;
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[0].header("range"),
            Some(format!("bytes={}-", corrupt_part.len()).as_str())
        );
        assert_eq!(
            requests[1].header("range"),
            None,
            "retry after corrupt equal-length 416 must be unranged"
        );
        assert_eq!(fs::read(&final_path).unwrap(), full_payload);
        assert!(!part_path.exists());
    }

    #[tokio::test]
    async fn test_download_error_responses_never_write_body_to_part_or_final_file() {
        let error_cases: [(&str, &str, &[u8], usize); 3] = [
            (
                "404",
                "404 Not Found",
                b"<html><body><h1>404 Not Found</h1></body></html>",
                1,
            ),
            (
                "500",
                "500 Internal Server Error",
                b"<html><body>Internal Server Error</body></html>",
                3,
            ),
            ("403", "403 Forbidden", b"Access Denied", 1),
        ];

        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();
        let fast_policy = RetryPolicy {
            max_retries: 2,
            initial_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(1),
            max_retry_after: Duration::from_millis(1),
        };

        for (label, status_line, body, expected_requests) in error_cases {
            // Case 1: no prior .part file
            let dir_fresh = TempTestDir::new(&format!("err-fresh-{label}"));
            let rel_path = "target.bin";
            let final_path_fresh = dir_fresh.join(rel_path);
            let part_path_fresh = dir_fresh.part_path(rel_path);

            let fresh_responses = (0..expected_requests)
                .map(|_| {
                    MockResponse::with_headers(
                        status_line,
                        vec![
                            ("Content-Type", "text/html".to_string()),
                            ("Content-Length", body.len().to_string()),
                        ],
                        body,
                    )
                })
                .collect();
            let server_fresh = MockServer::start(fresh_responses).await;

            let err = download_single_file_with_policy(
                &client,
                &dir_fresh.path,
                &server_fresh.url("/target.bin"),
                rel_path,
                None,
                &signal,
                None,
                &fast_policy,
            )
            .await
            .unwrap_err();

            let requests = server_fresh.finish().await;
            assert_eq!(requests.len(), expected_requests, "fresh {label}");
            assert!(matches!(err, IaGetError::Network(_)), "fresh {label}");
            assert!(
                !part_path_fresh.exists(),
                "fresh {label} must not create .part"
            );
            assert!(
                !final_path_fresh.exists(),
                "fresh {label} must not create final file"
            );

            // Case 2: pre-existing .part prefix must stay untouched
            let existing_prefix = b"preserved-partial-prefix";
            let dir_resume = TempTestDir::new(&format!("err-resume-{label}"));
            let final_path_resume = dir_resume.join(rel_path);
            let part_path_resume = dir_resume.part_path(rel_path);
            fs::write(&part_path_resume, existing_prefix).unwrap();

            let resume_responses = (0..expected_requests)
                .map(|_| {
                    MockResponse::with_headers(
                        status_line,
                        vec![
                            ("Content-Type", "text/html".to_string()),
                            ("Content-Length", body.len().to_string()),
                        ],
                        body,
                    )
                })
                .collect();
            let server_resume = MockServer::start(resume_responses).await;

            let err = download_single_file_with_policy(
                &client,
                &dir_resume.path,
                &server_resume.url("/target.bin"),
                rel_path,
                None,
                &signal,
                None,
                &fast_policy,
            )
            .await
            .unwrap_err();

            let requests = server_resume.finish().await;
            assert_eq!(requests.len(), expected_requests, "resume {label}");
            for request in &requests {
                assert_eq!(
                    request.header("range"),
                    Some(format!("bytes={}-", existing_prefix.len()).as_str()),
                    "resume {label}"
                );
            }
            assert!(matches!(err, IaGetError::Network(_)), "resume {label}");
            assert_eq!(
                fs::read(&part_path_resume).unwrap(),
                existing_prefix,
                "resume {label} must not append or overwrite .part with error body"
            );
            assert!(
                !final_path_resume.exists(),
                "resume {label} must not create final file"
            );
        }
    }

    #[tokio::test]
    async fn test_download_md5_mismatch_returns_integrity_error_and_does_not_promote() {
        let payload = b"corrupted payload bytes";
        let wrong_md5 = "00000000000000000000000000000000";
        let server = MockServer::start(vec![MockResponse::ok(payload)]).await;

        let dir = TempTestDir::new("md5-mismatch");
        let rel_path = "corrupt.bin";
        let final_path = dir.join(rel_path);
        let part_path = dir.part_path(rel_path);
        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();

        let err = download_single_file(
            &client,
            &dir.path,
            &server.url("/corrupt.bin"),
            rel_path,
            Some(wrong_md5),
            &signal,
            None,
        )
        .await
        .unwrap_err();

        let requests = server.finish().await;
        assert_eq!(requests.len(), 1);
        assert!(matches!(err, IaGetError::Integrity(_)));
        assert!(
            !final_path.exists(),
            "MD5 mismatch must never promote .part to final path"
        );
        assert_eq!(fs::read(&part_path).unwrap(), payload);
    }

    #[tokio::test]
    async fn test_download_http_206_huge_content_length_does_not_overflow_or_panic() {
        let prefix = b"hello";
        let chunk = b"world";
        let dir = TempTestDir::new("u64-max-cl");
        let rel_path = "huge-cl.bin";
        let final_path = dir.join(rel_path);
        let part_path = dir.part_path(rel_path);
        fs::write(&part_path, prefix).unwrap();

        // hyper reserves u64::MAX and u64::MAX - 1 as DecodedLength sentinels, so
        // u64::MAX - 2 is the largest Content-Length it passes to reqwest; with a
        // 5-byte prefix it still overflows current_offset + content_length without
        // saturating arithmetic. (u64::MAX itself is also covered by unit tests.)
        let huge_content_length = u64::MAX - 2;
        assert!(prefix.len() as u64 > u64::MAX - huge_content_length);

        let signal = ShutdownSignal::new();
        let server = MockServer::start(vec![MockResponse::MidStreamInterrupt {
            status_line: "206 Partial Content",
            headers: vec![
                ("Content-Range".to_string(), "bytes 5-9/*".to_string()),
                (
                    "Content-Length".to_string(),
                    huge_content_length.to_string(),
                ),
            ],
            first_chunk: chunk.to_vec(),
            second_chunk: None,
            watch_path: part_path.clone(),
            expected_part_len: (prefix.len() + chunk.len()) as u64,
            signal: signal.clone(),
        }])
        .await;

        let client = http_client_builder().build().unwrap();
        let result = download_single_file(
            &client,
            &dir.path,
            &server.url("/huge-cl.bin"),
            rel_path,
            None,
            &signal,
            None,
        )
        .await;

        let requests = server.finish().await;
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].header("range"),
            Some(format!("bytes={}-", prefix.len()).as_str())
        );
        assert!(
            result.is_err(),
            "truncated stream with u64::MAX Content-Length should return an error without panicking"
        );
        assert!(!final_path.exists());
        assert_eq!(fs::read(&part_path).unwrap(), b"helloworld");
    }

    fn utc_time(year: i64, month: u8, day: u8, hour: u8, minute: u8, second: u8) -> SystemTime {
        let secs = days_from_civil(year, month, day) * 86_400
            + i64::from(hour) * 3600
            + i64::from(minute) * 60
            + i64::from(second);
        if secs >= 0 {
            UNIX_EPOCH + Duration::from_secs(secs as u64)
        } else {
            UNIX_EPOCH - Duration::from_secs(secs.unsigned_abs())
        }
    }

    #[test]
    fn test_parse_retry_after_delta_seconds() {
        let now = utc_time(2026, 5, 1, 12, 0, 0);
        let cap = Duration::from_secs(60);

        assert_eq!(parse_retry_after("0", now, cap), Some(Duration::ZERO));
        assert_eq!(parse_retry_after("00", now, cap), Some(Duration::ZERO));
        assert_eq!(
            parse_retry_after("5", now, cap),
            Some(Duration::from_secs(5))
        );
        assert_eq!(
            parse_retry_after("  42  ", now, cap),
            Some(Duration::from_secs(42))
        );
        assert_eq!(parse_retry_after("60", now, cap), Some(cap));
        assert_eq!(parse_retry_after("120", now, cap), Some(cap));
        assert_eq!(
            parse_retry_after("999999999999999999999999999999999999", now, cap),
            Some(cap)
        );
        assert_eq!(
            parse_retry_after("10", now, Duration::ZERO),
            Some(Duration::ZERO)
        );
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
    fn test_parse_retry_after_http_dates_and_leap_years() {
        let cap = Duration::from_secs(60);

        // IMF-fixdate: future, equal, past, far-future clamp
        let base_1994 = utc_time(1994, 11, 6, 8, 49, 0);
        assert_eq!(
            parse_retry_after("Sun, 06 Nov 1994 08:49:37 GMT", base_1994, cap),
            Some(Duration::from_secs(37))
        );
        assert_eq!(
            parse_retry_after(
                "Sun, 06 Nov 1994 08:49:37 GMT",
                utc_time(1994, 11, 6, 8, 49, 37),
                cap,
            ),
            Some(Duration::ZERO)
        );
        assert_eq!(
            parse_retry_after(
                "Sun, 06 Nov 1994 08:49:37 GMT",
                utc_time(1994, 11, 6, 9, 0, 0),
                cap,
            ),
            Some(Duration::ZERO)
        );
        assert_eq!(
            parse_retry_after("Sun, 06 Nov 1994 09:49:37 GMT", base_1994, cap),
            Some(cap)
        );

        // Subsecond precision on `now`
        let base_subsec = base_1994 + Duration::from_millis(250);
        assert_eq!(
            parse_retry_after("Sun, 06 Nov 1994 08:49:05 GMT", base_subsec, cap),
            Some(Duration::from_millis(4_750))
        );

        // Valid leap years (divisible by 400 and divisible by 4 not 100)
        assert_eq!(
            parse_retry_after(
                "Tue, 29 Feb 2000 00:00:30 GMT",
                utc_time(2000, 2, 29, 0, 0, 0),
                cap,
            ),
            Some(Duration::from_secs(30))
        );
        assert_eq!(
            parse_retry_after(
                "Thu, 29 Feb 2024 00:00:15 GMT",
                utc_time(2024, 2, 29, 0, 0, 0),
                cap,
            ),
            Some(Duration::from_secs(15))
        );

        // RFC 850: current century, past century pivot, and century wrap
        assert_eq!(
            parse_retry_after("Sunday, 06-Nov-94 08:49:37 GMT", base_1994, cap),
            Some(Duration::from_secs(37))
        );
        assert_eq!(
            parse_retry_after(
                "Sunday, 06-Nov-94 08:49:37 GMT",
                utc_time(2026, 1, 1, 0, 0, 0),
                cap,
            ),
            Some(Duration::ZERO)
        );
        assert_eq!(
            parse_retry_after(
                "Saturday, 01-Jan-00 00:00:30 GMT",
                utc_time(1999, 12, 31, 23, 59, 50),
                cap,
            ),
            Some(Duration::from_secs(40))
        );

        // asctime(): single-digit padded day and two-digit day
        assert_eq!(
            parse_retry_after("Sun Nov  6 08:49:37 1994", base_1994, cap),
            Some(Duration::from_secs(37))
        );
        assert_eq!(
            parse_retry_after(
                "Wed Nov 16 08:49:37 1994",
                utc_time(1994, 11, 16, 8, 49, 17),
                cap,
            ),
            Some(Duration::from_secs(20))
        );
        assert_eq!(
            parse_retry_after("Wed Nov 16 08:49:37 1994", base_1994, cap),
            Some(cap)
        );
    }

    #[test]
    fn test_compute_backoff_delay_with_sample_bounds_and_saturation() {
        let policy = RetryPolicy {
            max_retries: 5,
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(10),
            max_retry_after: Duration::from_secs(60),
        };

        // Attempt 1: base = 1s -> [500ms, 1000ms]
        assert_eq!(
            compute_backoff_delay_with_sample(1, &policy, 0),
            Duration::from_millis(500)
        );
        let mid = compute_backoff_delay_with_sample(1, &policy, u64::MAX / 2);
        assert!(
            mid >= Duration::from_millis(749) && mid <= Duration::from_millis(751),
            "expected ~750ms midpoint, got {:?}",
            mid
        );
        assert_eq!(
            compute_backoff_delay_with_sample(1, &policy, u64::MAX),
            Duration::from_secs(1)
        );

        // Doubling across attempts 2, 3, 4 (bases 2s, 4s, 8s) and cap at 10s on attempt 5
        assert_eq!(
            compute_backoff_delay_with_sample(2, &policy, 0),
            Duration::from_secs(1)
        );
        assert_eq!(
            compute_backoff_delay_with_sample(2, &policy, u64::MAX),
            Duration::from_secs(2)
        );
        assert_eq!(
            compute_backoff_delay_with_sample(3, &policy, 0),
            Duration::from_secs(2)
        );
        assert_eq!(
            compute_backoff_delay_with_sample(3, &policy, u64::MAX),
            Duration::from_secs(4)
        );
        assert_eq!(
            compute_backoff_delay_with_sample(4, &policy, 0),
            Duration::from_secs(4)
        );
        assert_eq!(
            compute_backoff_delay_with_sample(4, &policy, u64::MAX),
            Duration::from_secs(8)
        );
        assert_eq!(
            compute_backoff_delay_with_sample(5, &policy, 0),
            Duration::from_secs(5)
        );
        assert_eq!(
            compute_backoff_delay_with_sample(5, &policy, u64::MAX),
            Duration::from_secs(10)
        );

        // Large attempts must saturate cleanly without overflow
        for attempt in [64, 128, 1000, u32::MAX] {
            assert_eq!(
                compute_backoff_delay_with_sample(attempt, &policy, 0),
                Duration::from_secs(5)
            );
            assert_eq!(
                compute_backoff_delay_with_sample(attempt, &policy, u64::MAX),
                Duration::from_secs(10)
            );
        }

        // Attempt 0 saturates exponent to 0
        assert_eq!(
            compute_backoff_delay_with_sample(0, &policy, 0),
            Duration::from_millis(500)
        );

        // Zero delays
        let zero_policy = RetryPolicy {
            initial_delay: Duration::ZERO,
            ..policy
        };
        assert_eq!(
            compute_backoff_delay_with_sample(1, &zero_policy, u64::MAX),
            Duration::ZERO
        );
    }

    #[test]
    fn test_compute_retry_delay_prefers_valid_retry_after_and_falls_back() {
        let now = utc_time(2026, 5, 1, 12, 0, 0);
        let policy = RetryPolicy {
            max_retries: 3,
            initial_delay: Duration::from_secs(2),
            max_delay: Duration::from_secs(16),
            max_retry_after: Duration::from_secs(30),
        };

        assert_eq!(
            compute_retry_delay(1, Some("12"), now, &policy),
            Duration::from_secs(12)
        );
        assert_eq!(
            compute_retry_delay(1, Some("120"), now, &policy),
            Duration::from_secs(30)
        );

        let fallback_none = compute_retry_delay(1, None, now, &policy);
        assert!(
            fallback_none >= Duration::from_secs(1) && fallback_none <= Duration::from_secs(2),
            "expected fallback in [1s, 2s], got {:?}",
            fallback_none
        );

        let fallback_invalid = compute_retry_delay(2, Some("not-a-date"), now, &policy);
        assert!(
            fallback_invalid >= Duration::from_secs(2)
                && fallback_invalid <= Duration::from_secs(4),
            "expected fallback in [2s, 4s], got {:?}",
            fallback_invalid
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
            .get(format!("http://{}/", closed_addr))
            .send()
            .await
            .unwrap_err();
        assert!(is_retryable_reqwest_error(&connect_err));
    }

    #[tokio::test]
    async fn test_sleep_interruptible_already_triggered_returns_immediately() {
        let signal = ShutdownSignal::new();
        signal.trigger();
        assert!(!signal.is_running());

        let start = std::time::Instant::now();
        let err = sleep_interruptible(Duration::from_secs(60), &signal)
            .await
            .unwrap_err();
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "already-triggered signal should return immediately"
        );
        assert!(matches!(err, IaGetError::FileSystem(_)));
        assert!(err.to_string().contains("Download interrupted by user"));
    }

    #[tokio::test]
    async fn test_sleep_interruptible_cancels_long_wait_immediately() {
        let signal = ShutdownSignal::new();
        let trigger_signal = signal.clone();

        let trigger_task = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            trigger_signal.trigger();
        });

        let start = std::time::Instant::now();
        let err = sleep_interruptible(Duration::from_secs(60), &signal)
            .await
            .unwrap_err();
        trigger_task.await.unwrap();

        assert!(
            start.elapsed() < Duration::from_secs(2),
            "sleep_interruptible should wake promptly on trigger, took {:?}",
            start.elapsed()
        );
        assert!(err.to_string().contains("Download interrupted by user"));
    }

    #[tokio::test]
    async fn test_sleep_interruptible_completes_when_running() {
        let signal = ShutdownSignal::new();
        assert!(signal.is_running());
        assert!(signal.running_flag().load(Ordering::SeqCst));

        sleep_interruptible(Duration::ZERO, &signal).await.unwrap();
        sleep_interruptible(Duration::from_millis(10), &signal)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_send_and_fetch_text_with_retry_behavior() {
        let client = http_client_builder().build().unwrap();
        let policy = RetryPolicy {
            max_retries: 3,
            initial_delay: Duration::from_millis(5),
            max_delay: Duration::from_millis(20),
            max_retry_after: Duration::from_secs(5),
        };

        // Transient 503 + 429 (with Retry-After: 0) then 200 succeeds
        let server = MockServer::start(vec![
            MockResponse::with_headers("503 Service Unavailable", vec![], b"busy"),
            MockResponse::with_headers(
                "429 Too Many Requests",
                vec![("Retry-After", "0".to_string())],
                b"slow down",
            ),
            MockResponse::ok(b"<files/>"),
        ])
        .await;

        let signal = ShutdownSignal::new();
        let mut events = Vec::new();
        let meta_url = Url::parse(&server.url("/meta.xml")).unwrap();
        let body = fetch_text_with_retry(
            &meta_url,
            &policy,
            &signal,
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
        assert_eq!(events[0].0, 1);
        assert_eq!(events[0].1, 3);
        assert!(events[0].3.contains("503"));
        assert_eq!(events[1].0, 2);
        assert_eq!(events[1].2, Duration::ZERO);
        assert!(events[1].3.contains("429"));

        // Permanent 404 fails immediately without retrying
        let server_404 = MockServer::start(vec![MockResponse::with_headers(
            "404 Not Found",
            vec![],
            b"missing",
        )])
        .await;
        let mut retry_calls = 0;
        let missing_url = Url::parse(&server_404.url("/missing")).unwrap();
        let err = send_with_retry(
            &missing_url,
            &policy,
            &signal,
            |target_url| Ok(client.get(target_url.clone())),
            |_| retry_calls += 1,
        )
        .await
        .unwrap_err();
        let requests_404 = server_404.finish().await;
        assert_eq!(requests_404.len(), 1);
        assert_eq!(retry_calls, 0);
        assert!(matches!(err, IaGetError::Network(_)));

        // Exhausting max_retries on persistent 503 returns error
        let small_policy = RetryPolicy {
            max_retries: 2,
            ..policy
        };
        let server_503 = MockServer::start(vec![
            MockResponse::with_headers("503 Service Unavailable", vec![], b"1"),
            MockResponse::with_headers("503 Service Unavailable", vec![], b"2"),
            MockResponse::with_headers("503 Service Unavailable", vec![], b"3"),
        ])
        .await;
        let mut attempts = Vec::new();
        let down_url = Url::parse(&server_503.url("/down")).unwrap();
        let err = send_with_retry(
            &down_url,
            &small_policy,
            &signal,
            |target_url| Ok(client.get(target_url.clone())),
            |event| attempts.push(event.attempt),
        )
        .await
        .unwrap_err();
        let requests_503 = server_503.finish().await;
        assert_eq!(requests_503.len(), 3);
        assert_eq!(attempts, vec![1, 2]);
        assert!(matches!(err, IaGetError::Network(_)));

        // Triggering ShutdownSignal during retry sleep aborts immediately
        let server_cancel = MockServer::start(vec![MockResponse::with_headers(
            "429 Too Many Requests",
            vec![("Retry-After", "60".to_string())],
            b"rate limited",
        )])
        .await;
        let cancel_signal = ShutdownSignal::new();
        let trigger_signal = cancel_signal.clone();
        let start = std::time::Instant::now();
        let rate_limited_url = Url::parse(&server_cancel.url("/rate-limited")).unwrap();
        let err = send_with_retry(
            &rate_limited_url,
            &RetryPolicy {
                max_retries: 2,
                max_retry_after: Duration::from_secs(60),
                ..policy
            },
            &cancel_signal,
            |target_url| Ok(client.get(target_url.clone())),
            |_| trigger_signal.trigger(),
        )
        .await
        .unwrap_err();
        let requests_cancel = server_cancel.finish().await;
        assert_eq!(requests_cancel.len(), 1);
        assert!(start.elapsed() < Duration::from_secs(2));
        assert!(err.to_string().contains("Download interrupted by user"));
    }

    fn format_imf_fixdate(unix_secs: i64) -> String {
        const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
        const MONTHS: [&str; 12] = [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ];

        let days = unix_secs.div_euclid(86_400);
        let sec_of_day = unix_secs.rem_euclid(86_400);
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = mp + if mp < 10 { 3 } else { -9 };
        let year = y + i64::from(month <= 2);

        let weekday = WEEKDAYS[(days + 4).rem_euclid(7) as usize];
        let month_name = MONTHS[(month - 1) as usize];
        let hour = sec_of_day / 3600;
        let minute = (sec_of_day % 3600) / 60;
        let second = sec_of_day % 60;

        format!("{weekday}, {day:02} {month_name} {year:04} {hour:02}:{minute:02}:{second:02} GMT")
    }

    fn future_imf_fixdate_header() -> String {
        let since_epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after UNIX_EPOCH");
        let offset_secs = if since_epoch.subsec_millis() >= 800 {
            2
        } else {
            1
        };
        format_imf_fixdate(since_epoch.as_secs() as i64 + offset_secs)
    }

    #[tokio::test]
    async fn test_download_429_retry_after_zero_retries_immediately_and_promotes() {
        let payload = b"retry-after zero payload bytes";
        let expected_md5 = md5_hex(payload);
        let server = MockServer::start(vec![
            MockResponse::with_headers(
                "429 Too Many Requests",
                vec![
                    ("Retry-After", "0".to_string()),
                    ("Content-Length", "9".to_string()),
                ],
                b"slow down",
            ),
            MockResponse::ok(payload),
        ])
        .await;

        let dir = TempTestDir::new("429-retry-after-0");
        let rel_path = "item.bin";
        let final_path = dir.join(rel_path);
        let part_path = dir.part_path(rel_path);
        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();
        let policy = RetryPolicy {
            max_retries: 2,
            initial_delay: Duration::from_secs(10),
            max_delay: Duration::from_secs(10),
            max_retry_after: Duration::from_secs(5),
        };

        let start = std::time::Instant::now();
        download_single_file_with_policy(
            &client,
            &dir.path,
            &server.url("/item.bin"),
            rel_path,
            Some(&expected_md5),
            &signal,
            None,
            &policy,
        )
        .await
        .expect("429 with Retry-After: 0 should retry and promote");
        let elapsed = start.elapsed();

        let requests = server.finish().await;
        assert_eq!(requests.len(), 2);
        assert!(
            elapsed < Duration::from_secs(1),
            "Retry-After: 0 should retry promptly instead of falling back to 10s backoff, took {:?}",
            elapsed
        );
        assert_eq!(fs::read(&final_path).unwrap(), payload);
        assert!(!part_path.exists());
    }

    #[tokio::test]
    async fn test_download_429_retry_after_delta_seconds_honours_delay_and_promotes() {
        let payload = b"retry-after delta-seconds payload";
        let expected_md5 = md5_hex(payload);
        let server = MockServer::start(vec![
            MockResponse::with_headers(
                "429 Too Many Requests",
                vec![
                    ("Retry-After", "1".to_string()),
                    ("Content-Length", "9".to_string()),
                ],
                b"slow down",
            ),
            MockResponse::ok(payload),
        ])
        .await;

        let dir = TempTestDir::new("429-retry-after-1");
        let rel_path = "item.bin";
        let final_path = dir.join(rel_path);
        let part_path = dir.part_path(rel_path);
        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();
        let policy = RetryPolicy {
            max_retries: 2,
            initial_delay: Duration::from_millis(5),
            max_delay: Duration::from_millis(5),
            max_retry_after: Duration::from_secs(5),
        };

        let start = std::time::Instant::now();
        download_single_file_with_policy(
            &client,
            &dir.path,
            &server.url("/item.bin"),
            rel_path,
            Some(&expected_md5),
            &signal,
            None,
            &policy,
        )
        .await
        .expect("429 with Retry-After: 1 should wait, retry, and promote");
        let elapsed = start.elapsed();

        let requests = server.finish().await;
        assert_eq!(requests.len(), 2);
        assert!(
            elapsed >= Duration::from_millis(900) && elapsed < Duration::from_secs(4),
            "Retry-After: 1 should wait roughly 1s rather than 5ms backoff, took {:?}",
            elapsed
        );
        assert_eq!(fs::read(&final_path).unwrap(), payload);
        assert!(!part_path.exists());
    }

    #[tokio::test]
    async fn test_download_503_retry_after_imf_fixdate_retries_and_promotes() {
        let payload = b"retry-after imf-fixdate payload";
        let expected_md5 = md5_hex(payload);
        let retry_after_date = future_imf_fixdate_header();
        let server = MockServer::start(vec![
            MockResponse::with_headers(
                "503 Service Unavailable",
                vec![
                    ("Retry-After", retry_after_date),
                    ("Content-Length", "4".to_string()),
                ],
                b"busy",
            ),
            MockResponse::ok(payload),
        ])
        .await;

        let dir = TempTestDir::new("503-retry-after-date");
        let rel_path = "item.bin";
        let final_path = dir.join(rel_path);
        let part_path = dir.part_path(rel_path);
        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();
        let policy = RetryPolicy {
            max_retries: 2,
            initial_delay: Duration::from_secs(10),
            max_delay: Duration::from_secs(10),
            max_retry_after: Duration::from_secs(5),
        };

        let start = std::time::Instant::now();
        download_single_file_with_policy(
            &client,
            &dir.path,
            &server.url("/item.bin"),
            rel_path,
            Some(&expected_md5),
            &signal,
            None,
            &policy,
        )
        .await
        .expect("503 with future IMF-fixdate Retry-After should retry and promote");
        let elapsed = start.elapsed();

        let requests = server.finish().await;
        assert_eq!(requests.len(), 2);
        assert!(
            elapsed >= Duration::from_millis(100) && elapsed < Duration::from_secs(3),
            "IMF-fixdate Retry-After should wait for future timestamp without falling back to 10s backoff, took {:?}",
            elapsed
        );
        assert_eq!(fs::read(&final_path).unwrap(), payload);
        assert!(!part_path.exists());
    }

    #[tokio::test]
    async fn test_download_429_excessive_retry_after_clamps_to_max_retry_after() {
        let payload = b"clamped retry-after payload";
        let expected_md5 = md5_hex(payload);
        let server = MockServer::start(vec![
            MockResponse::with_headers(
                "429 Too Many Requests",
                vec![
                    ("Retry-After", "999999".to_string()),
                    ("Content-Length", "12".to_string()),
                ],
                b"rate limited",
            ),
            MockResponse::ok(payload),
        ])
        .await;

        let dir = TempTestDir::new("429-retry-after-clamp");
        let rel_path = "item.bin";
        let final_path = dir.join(rel_path);
        let part_path = dir.part_path(rel_path);
        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();
        let policy = RetryPolicy {
            max_retries: 2,
            initial_delay: Duration::from_secs(30),
            max_delay: Duration::from_secs(30),
            max_retry_after: Duration::from_millis(50),
        };

        let start = std::time::Instant::now();
        download_single_file_with_policy(
            &client,
            &dir.path,
            &server.url("/item.bin"),
            rel_path,
            Some(&expected_md5),
            &signal,
            None,
            &policy,
        )
        .await
        .expect("excessive Retry-After should clamp to max_retry_after and succeed");
        let elapsed = start.elapsed();

        let requests = server.finish().await;
        assert_eq!(requests.len(), 2);
        assert!(
            elapsed >= Duration::from_millis(40) && elapsed < Duration::from_secs(1),
            "Retry-After: 999999 should clamp to ~50ms max_retry_after, took {:?}",
            elapsed
        );
        assert_eq!(fs::read(&final_path).unwrap(), payload);
        assert!(!part_path.exists());
    }

    #[tokio::test]
    async fn test_download_408_and_502_retry_and_recover_on_200() {
        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();
        let fast_policy = RetryPolicy {
            max_retries: 2,
            initial_delay: Duration::from_millis(5),
            max_delay: Duration::from_millis(10),
            max_retry_after: Duration::from_millis(50),
        };

        for (label, status_line) in [("408", "408 Request Timeout"), ("502", "502 Bad Gateway")] {
            let payload = format!("recovered payload after {label}");
            let expected_md5 = md5_hex(payload.as_bytes());
            let server = MockServer::start(vec![
                MockResponse::with_headers(
                    status_line,
                    vec![("Content-Length", "9".to_string())],
                    b"transient",
                ),
                MockResponse::ok(payload.as_bytes()),
            ])
            .await;

            let dir = TempTestDir::new(&format!("recover-{label}"));
            let rel_path = "item.bin";
            let final_path = dir.join(rel_path);
            let part_path = dir.part_path(rel_path);

            download_single_file_with_policy(
                &client,
                &dir.path,
                &server.url("/item.bin"),
                rel_path,
                Some(&expected_md5),
                &signal,
                None,
                &fast_policy,
            )
            .await
            .unwrap_or_else(|err| panic!("expected HTTP {label} to recover on 200, got: {err}"));

            let requests = server.finish().await;
            assert_eq!(requests.len(), 2, "case {label}");
            assert_eq!(
                fs::read(&final_path).unwrap(),
                payload.as_bytes(),
                "case {label}"
            );
            assert!(!part_path.exists(), "case {label}");
        }
    }

    #[tokio::test]
    async fn test_download_persistent_503_stops_after_one_plus_max_retries() {
        let fast_policy = RetryPolicy {
            max_retries: 2,
            initial_delay: Duration::from_millis(5),
            max_delay: Duration::from_millis(10),
            max_retry_after: Duration::from_millis(50),
        };
        let expected_requests = 1 + fast_policy.max_retries as usize;

        let mut responses: Vec<MockResponse> = (0..expected_requests)
            .map(|_| {
                MockResponse::with_headers(
                    "503 Service Unavailable",
                    vec![("Content-Length", "4".to_string())],
                    b"busy",
                )
            })
            .collect();
        responses.push(MockResponse::ok(b"should never be reached"));

        let server = MockServer::start(responses).await;
        let dir = TempTestDir::new("persistent-503");
        let rel_path = "item.bin";
        let final_path = dir.join(rel_path);
        let part_path = dir.part_path(rel_path);
        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();

        let err = download_single_file_with_policy(
            &client,
            &dir.path,
            &server.url("/item.bin"),
            rel_path,
            None,
            &signal,
            None,
            &fast_policy,
        )
        .await
        .unwrap_err();

        let requests = server.abort_and_collect().await;
        assert_eq!(requests.len(), expected_requests);
        assert!(matches!(err, IaGetError::Network(_)));
        assert!(err.to_string().contains("503"));
        assert!(!part_path.exists());
        assert!(!final_path.exists());
    }

    #[tokio::test]
    async fn test_download_403_and_404_fail_fast_without_retrying() {
        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();
        let slow_policy = RetryPolicy {
            max_retries: 3,
            initial_delay: Duration::from_secs(10),
            max_delay: Duration::from_secs(10),
            max_retry_after: Duration::from_secs(10),
        };

        for (code, status_line) in [("403", "403 Forbidden"), ("404", "404 Not Found")] {
            let server = MockServer::start(vec![
                MockResponse::with_headers(
                    status_line,
                    vec![("Content-Length", "4".to_string())],
                    b"nope",
                ),
                MockResponse::ok(b"should never be fetched on permanent 4xx"),
            ])
            .await;

            let dir = TempTestDir::new(&format!("fast-fail-{code}"));
            let rel_path = "item.bin";
            let final_path = dir.join(rel_path);
            let part_path = dir.part_path(rel_path);

            let start = std::time::Instant::now();
            let err = download_single_file_with_policy(
                &client,
                &dir.path,
                &server.url("/item.bin"),
                rel_path,
                None,
                &signal,
                None,
                &slow_policy,
            )
            .await
            .unwrap_err();
            let elapsed = start.elapsed();

            let requests = server.abort_and_collect().await;
            assert_eq!(requests.len(), 1, "HTTP {code} must not retry");
            assert!(
                elapsed < Duration::from_secs(1),
                "HTTP {code} should fail immediately, took {:?}",
                elapsed
            );
            assert!(matches!(err, IaGetError::Network(_)), "case {code}");
            assert!(err.to_string().contains(code), "case {code}: {err}");
            assert!(!part_path.exists(), "case {code}");
            assert!(!final_path.exists(), "case {code}");
        }
    }

    #[tokio::test]
    async fn test_download_ctrl_c_during_retry_wait_exits_promptly() {
        let server = MockServer::start(vec![
            MockResponse::with_headers(
                "429 Too Many Requests",
                vec![
                    ("Retry-After", "30".to_string()),
                    ("Content-Length", "12".to_string()),
                ],
                b"rate limited",
            ),
            MockResponse::ok(b"should never be downloaded"),
        ])
        .await;

        let dir = TempTestDir::new("ctrl-c-retry-wait");
        let rel_path = "item.bin";
        let final_path = dir.join(rel_path);
        let part_path = dir.part_path(rel_path);
        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();
        let policy = RetryPolicy {
            max_retries: 3,
            initial_delay: Duration::from_secs(10),
            max_delay: Duration::from_secs(30),
            max_retry_after: Duration::from_secs(60),
        };

        let requests_handle = server.requests_handle();
        let trigger_signal = signal.clone();
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
        let err = download_single_file_with_policy(
            &client,
            &dir.path,
            &server.url("/item.bin"),
            rel_path,
            None,
            &signal,
            None,
            &policy,
        )
        .await
        .unwrap_err();
        let elapsed = start.elapsed();
        trigger_task.await.unwrap();

        let requests = server.abort_and_collect().await;
        assert!(
            elapsed < Duration::from_secs(1),
            "Ctrl+C during 30s Retry-After wait should exit promptly, took {:?}",
            elapsed
        );
        assert_eq!(requests.len(), 1);
        assert!(matches!(err, IaGetError::FileSystem(_)));
        assert!(
            err.to_string().contains("Download interrupted by user"),
            "unexpected error: {err}"
        );
        assert!(!part_path.exists());
        assert!(!final_path.exists());
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

    #[tokio::test]
    async fn test_batch_continues_after_http_404_and_downloads_remaining_files() {
        let payload2 = b"second file archive payload";
        let payload3 = b"third file archive payload!";
        let md5_2 = md5_hex(payload2);
        let md5_3 = md5_hex(payload3);

        let server = MockServer::start(vec![
            MockResponse::with_headers(
                "404 Not Found",
                vec![("Content-Length", "9".to_string())],
                b"not found",
            ),
            MockResponse::ok(payload2),
            MockResponse::ok(payload3),
        ])
        .await;

        let dir = TempTestDir::new("batch-404-continue");
        let file1 = "file1.bin".to_string();
        let file2 = "file2.bin".to_string();
        let file3 = "nested/file3.bin".to_string();

        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();
        let policy = RetryPolicy {
            max_retries: 2,
            initial_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(2),
            max_retry_after: Duration::from_millis(5),
        };

        let files = vec![
            (server.url("/file1.bin"), file1.clone(), None),
            (server.url("/file2.bin"), file2.clone(), Some(md5_2)),
            (server.url("/file3.bin"), file3.clone(), Some(md5_3)),
        ];

        let summary = download_files_in_dir(&client, &dir.path, files, 3, None, &policy, &signal)
            .await
            .unwrap();

        let requests = server.finish().await;
        assert_eq!(requests.len(), 3);
        assert!(requests[0].request_line.starts_with("GET /file1.bin "));
        assert!(requests[1].request_line.starts_with("GET /file2.bin "));
        assert!(requests[2].request_line.starts_with("GET /file3.bin "));

        assert_eq!(summary.ok, 2);
        assert_eq!(summary.skipped, 0);
        assert_eq!(summary.failed.len(), 1);
        assert!(!summary.interrupted);
        assert_eq!(summary.failed[0].file_path, file1);
        assert!(
            summary.failed[0].error.contains("404"),
            "expected 404 in failure error, got: {}",
            summary.failed[0].error
        );

        assert!(!dir.join(&file1).exists());
        assert!(!dir.part_path(&file1).exists());
        assert_eq!(fs::read(dir.join(&file2)).unwrap(), payload2);
        assert!(!dir.part_path(&file2).exists());
        assert_eq!(fs::read(dir.join(&file3)).unwrap(), payload3);
        assert!(!dir.part_path(&file3).exists());

        let formatted = format_batch_summary(&summary);
        assert!(formatted.contains("2 ok"), "summary was: {formatted}");
        assert!(formatted.contains("1 failed"), "summary was: {formatted}");
        assert!(formatted.contains(&file1), "summary was: {formatted}");
    }

    #[tokio::test]
    async fn test_batch_continues_after_exhausted_retries() {
        let payload2 = b"file two downloads after file one exhausts retries";
        let md5_2 = md5_hex(payload2);

        let server = MockServer::start(vec![
            MockResponse::with_headers(
                "503 Service Unavailable",
                vec![("Content-Length", "4".to_string())],
                b"busy",
            ),
            MockResponse::with_headers(
                "503 Service Unavailable",
                vec![("Content-Length", "4".to_string())],
                b"busy",
            ),
            MockResponse::ok(payload2),
        ])
        .await;

        let dir = TempTestDir::new("batch-exhausted-retries");
        let file1 = "file1.bin".to_string();
        let file2 = "file2.bin".to_string();

        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();
        let fast_policy = RetryPolicy {
            max_retries: 1,
            initial_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(1),
            max_retry_after: Duration::from_millis(1),
        };

        let files = vec![
            (server.url("/file1.bin"), file1.clone(), None),
            (server.url("/file2.bin"), file2.clone(), Some(md5_2)),
        ];

        let summary =
            download_files_in_dir(&client, &dir.path, files, 2, None, &fast_policy, &signal)
                .await
                .unwrap();

        let requests = server.finish().await;
        assert_eq!(requests.len(), 3);
        assert!(requests[0].request_line.starts_with("GET /file1.bin "));
        assert!(requests[1].request_line.starts_with("GET /file1.bin "));
        assert!(requests[2].request_line.starts_with("GET /file2.bin "));

        assert_eq!(summary.ok, 1);
        assert_eq!(summary.skipped, 0);
        assert_eq!(summary.failed.len(), 1);
        assert!(!summary.interrupted);
        assert_eq!(summary.failed[0].file_path, file1);
        assert!(
            summary.failed[0].error.contains("503"),
            "expected 503 in failure error, got: {}",
            summary.failed[0].error
        );

        assert!(!dir.join(&file1).exists());
        assert!(!dir.part_path(&file1).exists());
        assert_eq!(fs::read(dir.join(&file2)).unwrap(), payload2);
        assert!(!dir.part_path(&file2).exists());
    }

    #[tokio::test]
    async fn test_batch_md5_mismatch_counts_as_failed_and_continues() {
        let payload1 = b"corrupted first file payload";
        let payload2 = b"verified second file payload";
        let wrong_md5 = "00000000000000000000000000000000".to_string();
        let md5_2 = md5_hex(payload2);

        let server =
            MockServer::start(vec![MockResponse::ok(payload1), MockResponse::ok(payload2)]).await;

        let dir = TempTestDir::new("batch-md5-mismatch");
        let file1 = "corrupt.bin".to_string();
        let file2 = "good.bin".to_string();

        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();
        let policy = RetryPolicy {
            max_retries: 1,
            initial_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(1),
            max_retry_after: Duration::from_millis(1),
        };

        let files = vec![
            (server.url("/corrupt.bin"), file1.clone(), Some(wrong_md5)),
            (server.url("/good.bin"), file2.clone(), Some(md5_2)),
        ];

        let summary = download_files_in_dir(&client, &dir.path, files, 2, None, &policy, &signal)
            .await
            .unwrap();

        let requests = server.finish().await;
        assert_eq!(requests.len(), 2);

        assert_eq!(summary.ok, 1);
        assert_eq!(summary.skipped, 0);
        assert_eq!(summary.failed.len(), 1);
        assert!(!summary.interrupted);
        assert_eq!(summary.failed[0].file_path, file1);
        assert!(
            summary.failed[0].error.contains("MD5 mismatch"),
            "expected MD5 mismatch in failure error, got: {}",
            summary.failed[0].error
        );

        assert!(
            !dir.join(&file1).exists(),
            "MD5 mismatch must not promote file 1 to final path"
        );
        assert_eq!(fs::read(dir.part_path(&file1)).unwrap(), payload1);
        assert_eq!(fs::read(dir.join(&file2)).unwrap(), payload2);
        assert!(!dir.part_path(&file2).exists());
    }

    #[tokio::test]
    async fn test_batch_counts_skipped_existing_files_alongside_ok_and_failed() {
        let verified_payload = b"already downloaded and verified";
        let verified_md5 = md5_hex(verified_payload);
        let unhashed_payload = b"already present without md5";
        let fresh_payload = b"freshly downloaded file payload";
        let fresh_md5 = md5_hex(fresh_payload);

        let dir = TempTestDir::new("batch-skipped-mixed");
        let existing_verified = "existing_verified.bin".to_string();
        let existing_unhashed = "existing_unhashed.bin".to_string();
        let fresh_ok = "fresh_ok.bin".to_string();
        let missing_fail = "missing_fail.bin".to_string();

        fs::write(dir.join(&existing_verified), verified_payload).unwrap();
        fs::write(dir.join(&existing_unhashed), unhashed_payload).unwrap();

        let server = MockServer::start(vec![
            MockResponse::ok(fresh_payload),
            MockResponse::with_headers(
                "404 Not Found",
                vec![("Content-Length", "7".to_string())],
                b"missing",
            ),
        ])
        .await;

        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();
        let policy = RetryPolicy {
            max_retries: 1,
            initial_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(1),
            max_retry_after: Duration::from_millis(1),
        };

        let files = vec![
            (
                server.url("/existing_verified.bin"),
                existing_verified.clone(),
                Some(verified_md5),
            ),
            (
                server.url("/existing_unhashed.bin"),
                existing_unhashed.clone(),
                None,
            ),
            (
                server.url("/fresh_ok.bin"),
                fresh_ok.clone(),
                Some(fresh_md5),
            ),
            (server.url("/missing_fail.bin"), missing_fail.clone(), None),
        ];

        let summary = download_files_in_dir(&client, &dir.path, files, 4, None, &policy, &signal)
            .await
            .unwrap();

        let requests = server.finish().await;
        assert_eq!(requests.len(), 2);
        assert!(requests[0].request_line.starts_with("GET /fresh_ok.bin "));
        assert!(requests[1]
            .request_line
            .starts_with("GET /missing_fail.bin "));

        assert_eq!(summary.skipped, 2);
        assert_eq!(summary.ok, 1);
        assert_eq!(summary.failed.len(), 1);
        assert!(!summary.interrupted);
        assert_eq!(summary.failed[0].file_path, missing_fail);
        assert!(summary.failed[0].error.contains("404"));

        assert_eq!(
            fs::read(dir.join(&existing_verified)).unwrap(),
            verified_payload
        );
        assert_eq!(
            fs::read(dir.join(&existing_unhashed)).unwrap(),
            unhashed_payload
        );
        assert_eq!(fs::read(dir.join(&fresh_ok)).unwrap(), fresh_payload);
        assert!(!dir.part_path(&fresh_ok).exists());
        assert!(!dir.join(&missing_fail).exists());
    }

    #[tokio::test]
    async fn test_batch_ctrl_c_mid_stream_stops_cleanly_without_recording_failure() {
        let payload1 = b"completed first file";
        let md5_1 = md5_hex(payload1);
        let first_chunk = b"partial-before-ctrl-c";
        let second_chunk = b"-never-completed";
        let total_len = first_chunk.len() + second_chunk.len();

        let dir = TempTestDir::new("batch-ctrl-c-mid");
        let file1 = "file1.bin".to_string();
        let file2 = "file2.bin".to_string();
        let file3 = "file3.bin".to_string();
        let part2 = dir.part_path(&file2);

        let signal = ShutdownSignal::new();
        let server = MockServer::start(vec![
            MockResponse::ok(payload1),
            MockResponse::MidStreamInterrupt {
                status_line: "200 OK",
                headers: vec![("Content-Length".to_string(), total_len.to_string())],
                first_chunk: first_chunk.to_vec(),
                second_chunk: Some(second_chunk.to_vec()),
                watch_path: part2.clone(),
                expected_part_len: first_chunk.len() as u64,
                signal: signal.clone(),
            },
            MockResponse::ok(b"file3 must not be requested"),
        ])
        .await;

        let client = http_client_builder().build().unwrap();
        let policy = RetryPolicy {
            max_retries: 1,
            initial_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(1),
            max_retry_after: Duration::from_millis(1),
        };

        let files = vec![
            (server.url("/file1.bin"), file1.clone(), Some(md5_1)),
            (server.url("/file2.bin"), file2.clone(), None),
            (server.url("/file3.bin"), file3.clone(), None),
        ];

        let summary = download_files_in_dir(&client, &dir.path, files, 3, None, &policy, &signal)
            .await
            .unwrap();

        let requests = server.abort_and_collect().await;
        assert_eq!(requests.len(), 2);
        assert!(requests[0].request_line.starts_with("GET /file1.bin "));
        assert!(requests[1].request_line.starts_with("GET /file2.bin "));

        assert_eq!(summary.ok, 1);
        assert_eq!(summary.skipped, 0);
        assert!(
            summary.failed.is_empty(),
            "interrupted file must not be recorded as failed: {:?}",
            summary.failed
        );
        assert!(summary.interrupted);

        assert_eq!(fs::read(dir.join(&file1)).unwrap(), payload1);
        assert!(!dir.part_path(&file1).exists());
        assert!(!dir.join(&file2).exists());
        assert_eq!(fs::read(&part2).unwrap(), first_chunk);
        assert!(!dir.join(&file3).exists());
        assert!(!dir.part_path(&file3).exists());
    }

    #[tokio::test]
    async fn test_batch_ctrl_c_before_start_records_interrupted_with_zero_requests() {
        let server = MockServer::start(vec![
            MockResponse::ok(b"should not be requested 1"),
            MockResponse::ok(b"should not be requested 2"),
        ])
        .await;

        let dir = TempTestDir::new("batch-ctrl-c-before-start");
        let file1 = "file1.bin".to_string();
        let file2 = "file2.bin".to_string();

        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();
        signal.trigger();

        let policy = RetryPolicy {
            max_retries: 1,
            initial_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(1),
            max_retry_after: Duration::from_millis(1),
        };

        let files = vec![
            (server.url("/file1.bin"), file1.clone(), None),
            (server.url("/file2.bin"), file2.clone(), None),
        ];

        let summary = download_files_in_dir(&client, &dir.path, files, 2, None, &policy, &signal)
            .await
            .unwrap();

        let requests = server.abort_and_collect().await;
        assert!(
            requests.is_empty(),
            "expected 0 requests, got {:?}",
            requests
        );

        assert_eq!(summary.ok, 0);
        assert_eq!(summary.skipped, 0);
        assert!(summary.failed.is_empty());
        assert!(summary.interrupted);
        assert!(!dir.join(&file1).exists());
        assert!(!dir.part_path(&file1).exists());
        assert!(!dir.join(&file2).exists());
        assert!(!dir.part_path(&file2).exists());
    }

    #[test]
    fn test_format_batch_summary_outputs_counts_failed_files_and_interrupted_state() {
        let all_ok = BatchSummary {
            ok: 3,
            skipped: 0,
            failed: Vec::new(),
            interrupted: false,
        };
        assert_eq!(
            format_batch_summary(&all_ok),
            "Summary: 3 ok, 0 skipped, 0 failed"
        );

        let mixed = BatchSummary {
            ok: 2,
            skipped: 1,
            failed: vec![
                FailedFile {
                    file_path: "first.bin".to_string(),
                    error: "Network error: HTTP error 404 Not Found".to_string(),
                },
                FailedFile {
                    file_path: "second.bin".to_string(),
                    error: "Integrity check failed: MD5 mismatch for second.bin".to_string(),
                },
            ],
            interrupted: false,
        };
        assert_eq!(
            format_batch_summary(&mixed),
            concat!(
                "Summary: 2 ok, 1 skipped, 2 failed\n",
                "Failed files:\n",
                "  - first.bin: Network error: HTTP error 404 Not Found\n",
                "  - second.bin: Integrity check failed: MD5 mismatch for second.bin"
            )
        );

        let interrupted_clean = BatchSummary {
            ok: 1,
            skipped: 2,
            failed: Vec::new(),
            interrupted: true,
        };
        assert_eq!(
            format_batch_summary(&interrupted_clean),
            "Summary: 1 ok, 2 skipped, 0 failed, interrupted"
        );

        let interrupted_with_failure = BatchSummary {
            ok: 1,
            skipped: 0,
            failed: vec![FailedFile {
                file_path: "broken.bin".to_string(),
                error: "Network error: HTTP error 503 Service Unavailable".to_string(),
            }],
            interrupted: true,
        };
        assert_eq!(
            format_batch_summary(&interrupted_with_failure),
            concat!(
                "Summary: 1 ok, 0 skipped, 1 failed, interrupted\n",
                "Failed files:\n",
                "  - broken.bin: Network error: HTTP error 503 Service Unavailable"
            )
        );
    }

    #[test]
    fn test_validate_relative_path_rejects_traversal_absolute_empty_and_curdir() {
        for unsafe_path in [
            "",
            ".",
            "./item.bin",
            "../escape.bin",
            "sub/../../escape.bin",
            "/abs/path.bin",
        ] {
            let err = validate_relative_path(Path::new(unsafe_path)).unwrap_err();
            assert!(
                matches!(err, IaGetError::FileSystem(_)),
                "expected FileSystem error for {unsafe_path:?}, got {err:?}"
            );
        }

        assert_eq!(
            validate_relative_path(Path::new("item.bin")).unwrap(),
            PathBuf::from("item.bin")
        );
        assert_eq!(
            validate_relative_path(Path::new("nested/dir/item.bin")).unwrap(),
            PathBuf::from("nested/dir/item.bin")
        );
    }

    #[tokio::test]
    async fn test_download_rejects_traversal_and_unsafe_paths_with_zero_requests() {
        let dir = TempTestDir::new("unsafe-paths");
        let root = dir.join("root");
        fs::create_dir_all(&root).unwrap();

        let server = MockServer::start(vec![MockResponse::ok(b"should never be fetched")]).await;
        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();

        for unsafe_path in [
            "../escape.bin",
            "sub/../../escape.bin",
            "/abs/path.bin",
            "",
            ".",
        ] {
            let err = download_single_file(
                &client,
                &root,
                &server.url("/item.bin"),
                unsafe_path,
                None,
                &signal,
                None,
            )
            .await
            .unwrap_err();
            assert!(
                matches!(err, IaGetError::FileSystem(_)),
                "expected FileSystem error for {unsafe_path:?}, got {err:?}"
            );
        }

        let requests = server.abort_and_collect().await;
        assert!(
            requests.is_empty(),
            "expected zero HTTP requests for unsafe paths, got {requests:?}"
        );
        assert!(!dir.join("escape.bin").exists());
    }

    #[tokio::test]
    async fn test_download_rejects_directory_at_target_or_part_with_zero_requests() {
        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();

        // Directory at target (with and without expected_md5)
        for expected_md5 in [None, Some("d41d8cd98f00b204e9800998ecf8427e")] {
            let dir = TempTestDir::new("dir-at-target");
            fs::create_dir(dir.join("item.bin")).unwrap();
            let server = MockServer::start(vec![MockResponse::ok(b"payload")]).await;

            let err = download_single_file(
                &client,
                &dir.path,
                &server.url("/item.bin"),
                "item.bin",
                expected_md5,
                &signal,
                None,
            )
            .await
            .unwrap_err();

            let requests = server.abort_and_collect().await;
            assert!(matches!(err, IaGetError::FileSystem(_)));
            assert!(requests.is_empty());
            assert!(dir.join("item.bin").is_dir());
            assert!(!dir.part_path("item.bin").exists());
        }

        // Directory at .part
        let dir = TempTestDir::new("dir-at-part");
        fs::create_dir(dir.part_path("item.bin")).unwrap();
        let server = MockServer::start(vec![MockResponse::ok(b"payload")]).await;

        let err = download_single_file(
            &client,
            &dir.path,
            &server.url("/item.bin"),
            "item.bin",
            None,
            &signal,
            None,
        )
        .await
        .unwrap_err();

        let requests = server.abort_and_collect().await;
        assert!(matches!(err, IaGetError::FileSystem(_)));
        assert!(requests.is_empty());
        assert!(dir.part_path("item.bin").is_dir());
        assert!(!dir.join("item.bin").exists());
    }

    #[tokio::test]
    async fn test_download_rejects_regular_file_blocking_intermediate_directory() {
        let dir = TempTestDir::new("file-blocks-dir");
        let blocker = dir.join("blocker");
        fs::write(&blocker, b"existing regular file").unwrap();

        let server = MockServer::start(vec![MockResponse::ok(b"payload")]).await;
        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();

        let err = download_single_file(
            &client,
            &dir.path,
            &server.url("/item.bin"),
            "blocker/item.bin",
            None,
            &signal,
            None,
        )
        .await
        .unwrap_err();

        let requests = server.abort_and_collect().await;
        assert!(matches!(err, IaGetError::FileSystem(_)));
        assert!(requests.is_empty());
        assert_eq!(fs::read(&blocker).unwrap(), b"existing regular file");
    }

    #[cfg(unix)]
    fn create_fifo(path: &Path) {
        let status = std::process::Command::new("mkfifo")
            .arg(path)
            .status()
            .expect("run mkfifo");
        assert!(status.success(), "mkfifo failed for {}", path.display());
    }

    #[cfg(unix)]
    #[test]
    fn test_unix_open_nofollow_flag_rejects_symlink() {
        use std::os::unix::fs::{symlink, OpenOptionsExt};

        let Some(flag) = unix_open_nofollow_flag() else {
            return;
        };

        let dir = TempTestDir::new("nofollow-flag");
        let real_file = dir.join("real.bin");
        let link_file = dir.join("link.bin");
        fs::write(&real_file, b"hello").unwrap();
        symlink(&real_file, &link_file).unwrap();

        let _regular = fs::OpenOptions::new()
            .read(true)
            .custom_flags(flag)
            .open(&real_file)
            .expect("O_NOFOLLOW should allow opening regular files");

        let err = fs::OpenOptions::new()
            .read(true)
            .custom_flags(flag)
            .open(&link_file)
            .unwrap_err();
        assert!(
            err.raw_os_error().is_some(),
            "expected OS error (ELOOP) when opening symlink with O_NOFOLLOW, got {err:?}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_unix_symlinked_intermediate_dir_refused_with_zero_requests() {
        use std::os::unix::fs::symlink;

        let dir = TempTestDir::new("symlink-parent");
        let root = dir.join("root");
        let outside = dir.join("outside");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();

        let outside_file = outside.join("victim.bin");
        fs::write(&outside_file, b"original outside data").unwrap();
        symlink(&outside, root.join("sub")).unwrap();

        let server = MockServer::start(vec![MockResponse::ok(b"attacker payload")]).await;
        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();

        let err = download_single_file(
            &client,
            &root,
            &server.url("/victim.bin"),
            "sub/victim.bin",
            None,
            &signal,
            None,
        )
        .await
        .unwrap_err();

        let requests = server.abort_and_collect().await;
        assert!(matches!(err, IaGetError::FileSystem(_)));
        assert!(requests.is_empty(), "expected zero HTTP requests");
        assert_eq!(fs::read(&outside_file).unwrap(), b"original outside data");
        assert!(!outside.join("victim.bin.part").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_unix_live_and_dangling_symlinked_part_refused_with_zero_requests() {
        use std::os::unix::fs::symlink;

        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();

        // Live symlinked .part
        {
            let dir = TempTestDir::new("symlink-part-live");
            let root = dir.join("root");
            let outside = dir.join("outside");
            fs::create_dir_all(&root).unwrap();
            fs::create_dir_all(&outside).unwrap();

            let outside_victim = outside.join("victim.bin");
            fs::write(&outside_victim, b"do not overwrite").unwrap();
            symlink(&outside_victim, root.join("item.bin.part")).unwrap();

            let server = MockServer::start(vec![MockResponse::ok(b"new payload")]).await;
            let err = download_single_file(
                &client,
                &root,
                &server.url("/item.bin"),
                "item.bin",
                None,
                &signal,
                None,
            )
            .await
            .unwrap_err();

            let requests = server.abort_and_collect().await;
            assert!(matches!(err, IaGetError::FileSystem(_)));
            assert!(requests.is_empty());
            assert_eq!(fs::read(&outside_victim).unwrap(), b"do not overwrite");
            assert!(!root.join("item.bin").exists());
        }

        // Dangling symlinked .part
        {
            let dir = TempTestDir::new("symlink-part-dangling");
            let root = dir.join("root");
            let outside = dir.join("outside");
            fs::create_dir_all(&root).unwrap();
            fs::create_dir_all(&outside).unwrap();

            let dangling_target = outside.join("should-not-exist.bin");
            symlink(&dangling_target, root.join("item.bin.part")).unwrap();

            let server = MockServer::start(vec![MockResponse::ok(b"new payload")]).await;
            let err = download_single_file(
                &client,
                &root,
                &server.url("/item.bin"),
                "item.bin",
                None,
                &signal,
                None,
            )
            .await
            .unwrap_err();

            let requests = server.abort_and_collect().await;
            assert!(matches!(err, IaGetError::FileSystem(_)));
            assert!(requests.is_empty());
            assert!(!dangling_target.exists());
            assert!(!root.join("item.bin").exists());
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_unix_live_and_dangling_symlinked_target_refused_with_and_without_md5() {
        use std::os::unix::fs::symlink;

        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();

        for expected_md5 in [None, Some("d41d8cd98f00b204e9800998ecf8427e")] {
            // Live symlinked target
            let dir = TempTestDir::new("symlink-target-live");
            let root = dir.join("root");
            let outside = dir.join("outside");
            fs::create_dir_all(&root).unwrap();
            fs::create_dir_all(&outside).unwrap();

            let outside_victim = outside.join("victim.bin");
            fs::write(&outside_victim, b"protected target").unwrap();
            symlink(&outside_victim, root.join("item.bin")).unwrap();

            let server = MockServer::start(vec![MockResponse::ok(b"payload")]).await;
            let err = download_single_file(
                &client,
                &root,
                &server.url("/item.bin"),
                "item.bin",
                expected_md5,
                &signal,
                None,
            )
            .await
            .unwrap_err();

            let requests = server.abort_and_collect().await;
            assert!(matches!(err, IaGetError::FileSystem(_)));
            assert!(requests.is_empty());
            assert_eq!(fs::read(&outside_victim).unwrap(), b"protected target");

            // Dangling symlinked target
            let dir_dangling = TempTestDir::new("symlink-target-dangling");
            let root_d = dir_dangling.join("root");
            let outside_d = dir_dangling.join("outside");
            fs::create_dir_all(&root_d).unwrap();
            fs::create_dir_all(&outside_d).unwrap();

            let missing_target = outside_d.join("missing.bin");
            symlink(&missing_target, root_d.join("item.bin")).unwrap();

            let server_d = MockServer::start(vec![MockResponse::ok(b"payload")]).await;
            let err_d = download_single_file(
                &client,
                &root_d,
                &server_d.url("/item.bin"),
                "item.bin",
                expected_md5,
                &signal,
                None,
            )
            .await
            .unwrap_err();

            let requests_d = server_d.abort_and_collect().await;
            assert!(matches!(err_d, IaGetError::FileSystem(_)));
            assert!(requests_d.is_empty());
            assert!(!missing_target.exists());
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_unix_fifo_at_target_and_part_refused_without_hanging() {
        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();

        // FIFO at target (with and without expected_md5)
        for expected_md5 in [None, Some("d41d8cd98f00b204e9800998ecf8427e")] {
            let dir = TempTestDir::new("fifo-target");
            create_fifo(&dir.join("item.bin"));
            let server = MockServer::start(vec![MockResponse::ok(b"payload")]).await;

            let result = tokio::time::timeout(
                Duration::from_secs(5),
                download_single_file(
                    &client,
                    &dir.path,
                    &server.url("/item.bin"),
                    "item.bin",
                    expected_md5,
                    &signal,
                    None,
                ),
            )
            .await
            .expect("FIFO target check must not hang");

            let requests = server.abort_and_collect().await;
            assert!(matches!(result.unwrap_err(), IaGetError::FileSystem(_)));
            assert!(requests.is_empty());
        }

        // FIFO at .part
        let dir = TempTestDir::new("fifo-part");
        create_fifo(&dir.part_path("item.bin"));
        let server = MockServer::start(vec![MockResponse::ok(b"payload")]).await;

        let result = tokio::time::timeout(
            Duration::from_secs(5),
            download_single_file(
                &client,
                &dir.path,
                &server.url("/item.bin"),
                "item.bin",
                None,
                &signal,
                None,
            ),
        )
        .await
        .expect("FIFO .part check must not hang");

        let requests = server.abort_and_collect().await;
        assert!(matches!(result.unwrap_err(), IaGetError::FileSystem(_)));
        assert!(requests.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn test_unix_promote_part_file_refuses_symlink_fifo_and_directory_targets() {
        use std::os::unix::fs::symlink;

        // Live symlink target
        {
            let dir = TempTestDir::new("promote-symlink-live");
            let root = dir.join("root");
            let outside = dir.join("outside");
            fs::create_dir_all(&root).unwrap();
            fs::create_dir_all(&outside).unwrap();

            let outside_file = outside.join("victim.bin");
            fs::write(&outside_file, b"original").unwrap();
            fs::write(root.join("item.bin.part"), b"staged").unwrap();
            symlink(&outside_file, root.join("item.bin")).unwrap();

            let err = promote_part_file(&root, Path::new("item.bin.part"), Path::new("item.bin"))
                .unwrap_err();
            assert!(matches!(err, IaGetError::FileSystem(_)));
            assert_eq!(fs::read(&outside_file).unwrap(), b"original");
            assert_eq!(fs::read(root.join("item.bin.part")).unwrap(), b"staged");
        }

        // Dangling symlink target
        {
            let dir = TempTestDir::new("promote-symlink-dangling");
            let root = dir.join("root");
            let outside = dir.join("outside");
            fs::create_dir_all(&root).unwrap();
            fs::create_dir_all(&outside).unwrap();

            let dangling = outside.join("missing.bin");
            fs::write(root.join("item.bin.part"), b"staged").unwrap();
            symlink(&dangling, root.join("item.bin")).unwrap();

            let err = promote_part_file(&root, Path::new("item.bin.part"), Path::new("item.bin"))
                .unwrap_err();
            assert!(matches!(err, IaGetError::FileSystem(_)));
            assert!(!dangling.exists());
            assert_eq!(fs::read(root.join("item.bin.part")).unwrap(), b"staged");
        }

        // FIFO target
        {
            let dir = TempTestDir::new("promote-fifo");
            fs::write(dir.join("item.bin.part"), b"staged").unwrap();
            create_fifo(&dir.join("item.bin"));

            let err =
                promote_part_file(&dir.path, Path::new("item.bin.part"), Path::new("item.bin"))
                    .unwrap_err();
            assert!(matches!(err, IaGetError::FileSystem(_)));
            assert_eq!(fs::read(dir.join("item.bin.part")).unwrap(), b"staged");
        }

        // Directory target
        {
            let dir = TempTestDir::new("promote-dir");
            fs::write(dir.join("item.bin.part"), b"staged").unwrap();
            fs::create_dir(dir.join("item.bin")).unwrap();

            let err =
                promote_part_file(&dir.path, Path::new("item.bin.part"), Path::new("item.bin"))
                    .unwrap_err();
            assert!(matches!(err, IaGetError::FileSystem(_)));
            assert!(dir.join("item.bin").is_dir());
            assert_eq!(fs::read(dir.join("item.bin.part")).unwrap(), b"staged");
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_unix_symlinked_root_succeeds_while_symlinks_inside_root_fail() {
        use std::os::unix::fs::symlink;

        let dir = TempTestDir::new("symlinked-root");
        let real_root = dir.join("real_root");
        let link_root = dir.join("link_root");
        let outside = dir.join("outside");
        fs::create_dir_all(&real_root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        symlink(&real_root, &link_root).unwrap();

        let payload = b"downloaded through symlinked root";
        let expected_md5 = md5_hex(payload);
        let server = MockServer::start(vec![MockResponse::ok(payload)]).await;
        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();

        // Normal nested download inside symlinked root must succeed
        download_single_file(
            &client,
            &link_root,
            &server.url("/good.bin"),
            "nested/good.bin",
            Some(&expected_md5),
            &signal,
            None,
        )
        .await
        .expect("download into symlinked root should succeed");

        let requests = server.finish().await;
        assert_eq!(requests.len(), 1);
        assert_eq!(
            fs::read(real_root.join("nested/good.bin")).unwrap(),
            payload
        );

        // Symlink planted inside that same symlinked root must still be refused
        symlink(&outside, real_root.join("escape")).unwrap();
        let server_bad = MockServer::start(vec![MockResponse::ok(b"nope")]).await;
        let err = download_single_file(
            &client,
            &link_root,
            &server_bad.url("/bad.bin"),
            "escape/bad.bin",
            None,
            &signal,
            None,
        )
        .await
        .unwrap_err();

        let bad_requests = server_bad.abort_and_collect().await;
        assert!(matches!(err, IaGetError::FileSystem(_)));
        assert!(bad_requests.is_empty());
        assert!(!outside.join("bad.bin").exists());
    }

    #[tokio::test]
    async fn test_download_cross_host_redirect_strips_cookie_and_verifies_md5() {
        let payload = b"cross-host redirected archive payload with verified md5";
        let expected_md5 = md5_hex(payload);

        let target_server = MockServer::start_on_localhost(vec![MockResponse::ok(payload)]).await;
        let redirect_url = target_server.url("/final/item.bin");
        let origin_server = MockServer::start(vec![MockResponse::with_headers(
            "302 Found",
            vec![
                ("Location", redirect_url),
                ("Content-Length", "0".to_string()),
            ],
            b"",
        )])
        .await;

        let cookie_source =
            CookieSource::from_netscape_str("127.0.0.1\tFALSE\t/\tFALSE\t0\tsession\tsecret123\n");
        let dir = TempTestDir::new("redirect-strip-cookie");
        let rel_path = "item.bin";
        let final_path = dir.join(rel_path);
        let part_path = dir.part_path(rel_path);
        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();

        download_single_file(
            &client,
            &dir.path,
            &origin_server.url("/download/item.bin"),
            rel_path,
            Some(&expected_md5),
            &signal,
            Some(&cookie_source),
        )
        .await
        .expect("cross-host redirect download should succeed and pass MD5 check");

        let origin_requests = origin_server.finish().await;
        let target_requests = target_server.finish().await;
        assert_eq!(origin_requests.len(), 1);
        assert!(origin_requests[0]
            .request_line
            .starts_with("GET /download/item.bin "));
        assert_eq!(
            origin_requests[0].header("cookie"),
            Some("session=secret123")
        );

        assert_eq!(target_requests.len(), 1);
        assert!(target_requests[0]
            .request_line
            .starts_with("GET /final/item.bin "));
        assert_eq!(
            target_requests[0].header("cookie"),
            None,
            "redirect to non-matching host must strip Cookie header"
        );

        assert_eq!(fs::read(&final_path).unwrap(), payload);
        assert!(!part_path.exists());
    }

    #[tokio::test]
    async fn test_download_same_host_redirect_rescopes_and_orders_path_cookies() {
        let payload = b"same-host path-scoped redirect payload";
        let expected_md5 = md5_hex(payload);

        let server = MockServer::start(vec![
            MockResponse::with_headers(
                "302 Found",
                vec![
                    ("Location", "/download/public/item.bin".to_string()),
                    ("Content-Length", "0".to_string()),
                ],
                b"",
            ),
            MockResponse::ok(payload),
        ])
        .await;

        // Place shorter path before longer path in file order to verify RFC 6265 longest-path-first sorting.
        let cookie_source = CookieSource::from_netscape_str(concat!(
            "127.0.0.1\tFALSE\t/\tFALSE\t0\troot\t1\n",
            "127.0.0.1\tFALSE\t/download/private\tFALSE\t0\tprivate\t1\n",
        ));

        let dir = TempTestDir::new("redirect-path-scope");
        let rel_path = "item.bin";
        let final_path = dir.join(rel_path);
        let part_path = dir.part_path(rel_path);
        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();

        download_single_file(
            &client,
            &dir.path,
            &server.url("/download/private/item.bin"),
            rel_path,
            Some(&expected_md5),
            &signal,
            Some(&cookie_source),
        )
        .await
        .expect("same-host path-scoped redirect download should succeed");

        let requests = server.finish().await;
        assert_eq!(requests.len(), 2);
        assert!(requests[0]
            .request_line
            .starts_with("GET /download/private/item.bin "));
        assert_eq!(requests[0].header("cookie"), Some("private=1; root=1"));

        assert!(requests[1]
            .request_line
            .starts_with("GET /download/public/item.bin "));
        assert_eq!(requests[1].header("cookie"), Some("root=1"));

        assert_eq!(fs::read(&final_path).unwrap(), payload);
        assert!(!part_path.exists());
    }

    #[tokio::test]
    async fn test_download_resume_preserves_range_across_all_redirect_statuses() {
        let prefix = b"partial-prefix-";
        let suffix = b"resumed-payload-after-redirect";
        let mut full = Vec::new();
        full.extend_from_slice(prefix);
        full.extend_from_slice(suffix);
        let expected_md5 = md5_hex(&full);
        let expected_range = format!("bytes={}-", prefix.len());
        let content_range = format!("bytes {}-{}/{}", prefix.len(), full.len() - 1, full.len());

        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();

        for (code, status_line) in [
            ("301", "301 Moved Permanently"),
            ("302", "302 Found"),
            ("303", "303 See Other"),
            ("307", "307 Temporary Redirect"),
            ("308", "308 Permanent Redirect"),
        ] {
            let dir = TempTestDir::new(&format!("resume-redirect-{code}"));
            let rel_path = "resumed.bin";
            let final_path = dir.join(rel_path);
            let part_path = dir.part_path(rel_path);
            fs::write(&part_path, prefix).unwrap();

            let server = MockServer::start(vec![
                MockResponse::with_headers(
                    status_line,
                    vec![
                        ("Location", "/cdn/resumed.bin".to_string()),
                        ("Content-Length", "0".to_string()),
                    ],
                    b"",
                ),
                MockResponse::with_headers(
                    "206 Partial Content",
                    vec![
                        ("Content-Range", content_range.clone()),
                        ("Content-Length", suffix.len().to_string()),
                    ],
                    suffix,
                ),
            ])
            .await;

            download_single_file(
                &client,
                &dir.path,
                &server.url("/download/resumed.bin"),
                rel_path,
                Some(&expected_md5),
                &signal,
                None,
            )
            .await
            .unwrap_or_else(|err| panic!("resume across {code} redirect should succeed: {err}"));

            let requests = server.finish().await;
            assert_eq!(requests.len(), 2, "status {code}");
            assert!(
                requests[0]
                    .request_line
                    .starts_with("GET /download/resumed.bin "),
                "status {code}"
            );
            assert_eq!(
                requests[0].header("range"),
                Some(expected_range.as_str()),
                "status {code} hop 0 must send Range"
            );
            assert!(
                requests[1]
                    .request_line
                    .starts_with("GET /cdn/resumed.bin "),
                "status {code}"
            );
            assert_eq!(
                requests[1].header("range"),
                Some(expected_range.as_str()),
                "status {code} hop 1 must preserve Range"
            );
            assert_eq!(fs::read(&final_path).unwrap(), full, "status {code}");
            assert!(!part_path.exists(), "status {code}");
        }
    }

    #[tokio::test]
    async fn test_download_raw_vs_cookies_txt_disambiguation_end_to_end() {
        let payload = b"cookie source disambiguation payload";
        let expected_md5 = md5_hex(payload);
        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();

        // Raw cookie string: strictly scoped to HTTPS archive.org, so http://127.0.0.1 receives no Cookie.
        let raw_source = CookieSource::from_input("logged-in-user=yes; logged-in-sig=abc123")
            .expect("raw cookie string should parse");
        assert!(matches!(raw_source, CookieSource::Raw(_)));

        let dir_raw = TempTestDir::new("cookie-raw-e2e");
        let server_raw = MockServer::start(vec![MockResponse::ok(payload)]).await;
        download_single_file(
            &client,
            &dir_raw.path,
            &server_raw.url("/download/item.bin"),
            "item.bin",
            Some(&expected_md5),
            &signal,
            Some(&raw_source),
        )
        .await
        .expect("download with raw cookie source should succeed");

        let raw_requests = server_raw.finish().await;
        assert_eq!(raw_requests.len(), 1);
        assert_eq!(
            raw_requests[0].header("cookie"),
            None,
            "raw cookie must not be sent to plain HTTP 127.0.0.1"
        );
        assert_eq!(fs::read(dir_raw.join("item.bin")).unwrap(), payload);

        // Netscape cookies.txt file: attaches only matching host and path cookies.
        let dir_file = TempTestDir::new("cookie-file-e2e");
        let cookie_file = dir_file.join("cookies.txt");
        fs::write(
            &cookie_file,
            concat!(
                "# Netscape HTTP Cookie File\n",
                "127.0.0.1\tFALSE\t/download\tFALSE\t0\tsession\tfromfile\n",
                "127.0.0.1\tFALSE\t/other\tFALSE\t0\tother\tskip\n",
                "example.com\tFALSE\t/\tFALSE\t0\tforeign\tskip\n",
            ),
        )
        .unwrap();

        let file_source = CookieSource::from_input(cookie_file.to_str().unwrap())
            .expect("cookies.txt file should parse");
        assert!(matches!(file_source, CookieSource::NetscapeFile(_)));

        let server_file = MockServer::start(vec![MockResponse::ok(payload)]).await;
        download_single_file(
            &client,
            &dir_file.path,
            &server_file.url("/download/item.bin"),
            "item.bin",
            Some(&expected_md5),
            &signal,
            Some(&file_source),
        )
        .await
        .expect("download with cookies.txt file should succeed");

        let file_requests = server_file.finish().await;
        assert_eq!(file_requests.len(), 1);
        assert_eq!(file_requests[0].header("cookie"), Some("session=fromfile"));
        assert_eq!(fs::read(dir_file.join("item.bin")).unwrap(), payload);
    }

    #[tokio::test]
    async fn test_download_redirect_errors_fail_fast_without_retrying() {
        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();
        let slow_policy = RetryPolicy {
            max_retries: 3,
            initial_delay: Duration::from_secs(10),
            max_delay: Duration::from_secs(10),
            max_retry_after: Duration::from_secs(10),
        };

        let mut too_many_responses: Vec<MockResponse> = (0..=MAX_REDIRECTS)
            .map(|hop| {
                MockResponse::with_headers(
                    "302 Found",
                    vec![
                        ("Location", format!("/hop-{}", hop + 1)),
                        ("Content-Length", "0".to_string()),
                    ],
                    b"",
                )
            })
            .collect();
        too_many_responses.push(MockResponse::ok(b"should not retry"));

        let cases: Vec<(&str, &str, Vec<MockResponse>, usize, &str)> = vec![
            (
                "direct-loop",
                "/a",
                vec![
                    MockResponse::with_headers(
                        "302 Found",
                        vec![
                            ("Location", "/a".to_string()),
                            ("Content-Length", "0".to_string()),
                        ],
                        b"",
                    ),
                    MockResponse::ok(b"should not retry"),
                ],
                1,
                "Redirect loop",
            ),
            (
                "two-hop-loop",
                "/a",
                vec![
                    MockResponse::with_headers(
                        "302 Found",
                        vec![
                            ("Location", "/b".to_string()),
                            ("Content-Length", "0".to_string()),
                        ],
                        b"",
                    ),
                    MockResponse::with_headers(
                        "302 Found",
                        vec![
                            ("Location", "/a".to_string()),
                            ("Content-Length", "0".to_string()),
                        ],
                        b"",
                    ),
                    MockResponse::ok(b"should not retry"),
                ],
                2,
                "Redirect loop",
            ),
            (
                "fragment-only-loop",
                "/a",
                vec![
                    MockResponse::with_headers(
                        "302 Found",
                        vec![
                            ("Location", "/a#section".to_string()),
                            ("Content-Length", "0".to_string()),
                        ],
                        b"",
                    ),
                    MockResponse::ok(b"should not retry"),
                ],
                1,
                "Redirect loop",
            ),
            (
                "too-many-redirects",
                "/hop-0",
                too_many_responses,
                MAX_REDIRECTS + 1,
                "Too many redirects",
            ),
            (
                "missing-location",
                "/missing",
                vec![
                    MockResponse::with_headers(
                        "302 Found",
                        vec![("Content-Length", "0".to_string())],
                        b"",
                    ),
                    MockResponse::ok(b"should not retry"),
                ],
                1,
                "missing Location",
            ),
            (
                "empty-location",
                "/empty",
                vec![
                    MockResponse::with_headers(
                        "302 Found",
                        vec![
                            ("Location", "   ".to_string()),
                            ("Content-Length", "0".to_string()),
                        ],
                        b"",
                    ),
                    MockResponse::ok(b"should not retry"),
                ],
                1,
                "empty Location",
            ),
            (
                "non-http-scheme",
                "/scheme",
                vec![
                    MockResponse::with_headers(
                        "302 Found",
                        vec![
                            ("Location", "file:///etc/passwd".to_string()),
                            ("Content-Length", "0".to_string()),
                        ],
                        b"",
                    ),
                    MockResponse::ok(b"should not retry"),
                ],
                1,
                "Unsupported redirect scheme",
            ),
        ];

        for (label, initial_path, responses, expected_requests, expected_msg) in cases {
            let dir = TempTestDir::new(&format!("redirect-err-{label}"));
            let server = MockServer::start(responses).await;

            let start = std::time::Instant::now();
            let err = download_single_file_with_policy(
                &client,
                &dir.path,
                &server.url(initial_path),
                "item.bin",
                None,
                &signal,
                None,
                &slow_policy,
            )
            .await
            .unwrap_err();
            let elapsed = start.elapsed();

            let requests = server.abort_and_collect().await;
            assert_eq!(
                requests.len(),
                expected_requests,
                "case {label}: unexpected request count"
            );
            assert!(
                elapsed < Duration::from_secs(2),
                "case {label}: redirect error must fail fast without retrying, took {:?}",
                elapsed
            );
            assert!(
                matches!(err, IaGetError::Network(_)),
                "case {label}: expected Network error, got {err:?}"
            );
            assert!(
                err.to_string().contains(expected_msg),
                "case {label}: expected '{expected_msg}' in error '{err}'"
            );
            assert!(!dir.join("item.bin").exists(), "case {label}");
            assert!(!dir.part_path("item.bin").exists(), "case {label}");
        }
    }

    #[tokio::test]
    async fn test_download_retry_after_redirected_503_restarts_from_initial_hop() {
        let payload = b"payload recovered after redirected 503";
        let expected_md5 = md5_hex(payload);

        let server = MockServer::start(vec![
            MockResponse::with_headers(
                "302 Found",
                vec![
                    ("Location", "/cdn/item.bin".to_string()),
                    ("Content-Length", "0".to_string()),
                ],
                b"",
            ),
            MockResponse::with_headers(
                "503 Service Unavailable",
                vec![
                    ("Retry-After", "0".to_string()),
                    ("Content-Length", "4".to_string()),
                ],
                b"busy",
            ),
            MockResponse::with_headers(
                "302 Found",
                vec![
                    ("Location", "/cdn/item.bin".to_string()),
                    ("Content-Length", "0".to_string()),
                ],
                b"",
            ),
            MockResponse::ok(payload),
        ])
        .await;

        let dir = TempTestDir::new("redirect-retry-503");
        let rel_path = "item.bin";
        let final_path = dir.join(rel_path);
        let part_path = dir.part_path(rel_path);
        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();
        let policy = RetryPolicy {
            max_retries: 2,
            initial_delay: Duration::from_secs(10),
            max_delay: Duration::from_secs(10),
            max_retry_after: Duration::from_secs(5),
        };

        let start = std::time::Instant::now();
        download_single_file_with_policy(
            &client,
            &dir.path,
            &server.url("/download/item.bin"),
            rel_path,
            Some(&expected_md5),
            &signal,
            None,
            &policy,
        )
        .await
        .expect("retry after redirected 503 should restart from hop 0 and succeed");
        let elapsed = start.elapsed();

        let requests = server.finish().await;
        assert_eq!(requests.len(), 4);
        assert!(requests[0]
            .request_line
            .starts_with("GET /download/item.bin "));
        assert!(requests[1].request_line.starts_with("GET /cdn/item.bin "));
        assert!(requests[2]
            .request_line
            .starts_with("GET /download/item.bin "));
        assert!(requests[3].request_line.starts_with("GET /cdn/item.bin "));
        assert!(
            elapsed < Duration::from_secs(2),
            "Retry-After: 0 should retry immediately, took {:?}",
            elapsed
        );
        assert_eq!(fs::read(&final_path).unwrap(), payload);
        assert!(!part_path.exists());
    }

    #[tokio::test]
    async fn test_download_cookie_expiry_reevaluated_across_retry_wait() {
        let payload = b"payload delivered after short-lived cookie expired";
        let expected_md5 = md5_hex(payload);

        let server = MockServer::start(vec![
            MockResponse::with_headers(
                "503 Service Unavailable",
                vec![
                    ("Retry-After", "2".to_string()),
                    ("Content-Length", "4".to_string()),
                ],
                b"busy",
            ),
            MockResponse::ok(payload),
        ])
        .await;

        let dir = TempTestDir::new("cookie-expiry-retry");
        let rel_path = "item.bin";
        let final_path = dir.join(rel_path);
        let part_path = dir.part_path(rel_path);
        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();
        let policy = RetryPolicy {
            max_retries: 2,
            initial_delay: Duration::from_secs(10),
            max_delay: Duration::from_secs(10),
            max_retry_after: Duration::from_secs(5),
        };

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after UNIX_EPOCH")
            .as_secs();
        let cookie_source = CookieSource::from_netscape_str(&format!(
            "127.0.0.1\tFALSE\t/\tFALSE\t{}\tshort\ttemp\n\
             127.0.0.1\tFALSE\t/\tFALSE\t{}\tlong\tkeep\n",
            now + 2,
            now + 3600
        ));

        download_single_file_with_policy(
            &client,
            &dir.path,
            &server.url("/download/item.bin"),
            rel_path,
            Some(&expected_md5),
            &signal,
            Some(&cookie_source),
            &policy,
        )
        .await
        .expect("retry after 503 should succeed with re-evaluated cookies");

        let requests = server.finish().await;
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].header("cookie"), Some("short=temp; long=keep"));
        assert_eq!(
            requests[1].header("cookie"),
            Some("long=keep"),
            "expired short-lived cookie must be omitted on retry"
        );
        assert_eq!(fs::read(&final_path).unwrap(), payload);
        assert!(!part_path.exists());
    }

    #[test]
    fn test_resolve_redirect_target_rejects_https_to_http_downgrade() {
        let current = Url::parse("https://archive.org/download/item/a").unwrap();

        for target in [
            "http://archive.org/download/item/a",
            "http://mirror.example/a",
        ] {
            let err = resolve_redirect_target(&current, target).unwrap_err();
            assert!(
                matches!(err, IaGetError::Network(_)),
                "target {target}: expected Network error, got {err:?}"
            );
            assert!(
                err.to_string().contains("downgrade"),
                "target {target}: expected downgrade error message, got '{err}'"
            );
        }
    }

    #[test]
    fn test_resolve_redirect_target_allows_valid_transitions() {
        let cases = [
            (
                "https://archive.org/download/item/a",
                "https://mirror.example/download/item/a",
                "https://mirror.example/download/item/a",
            ),
            (
                "https://archive.org/download/item/a",
                "/cdn/item/a",
                "https://archive.org/cdn/item/a",
            ),
            (
                "https://archive.org/download/item/a",
                "b",
                "https://archive.org/download/item/b",
            ),
            (
                "http://archive.org/download/item/a",
                "http://mirror.example/a",
                "http://mirror.example/a",
            ),
            (
                "http://archive.org/download/item/a",
                "https://archive.org/download/item/a",
                "https://archive.org/download/item/a",
            ),
        ];

        for (current_str, location, expected) in cases {
            let current = Url::parse(current_str).unwrap();
            let resolved = resolve_redirect_target(&current, location)
                .unwrap_or_else(|err| panic!("{current_str} -> {location} failed: {err}"));
            assert_eq!(resolved.as_str(), expected);
        }
    }

    #[test]
    fn test_resolve_redirect_target_rejects_invalid_and_unsupported_locations() {
        let current = Url::parse("https://archive.org/download/item/a").unwrap();

        for (location, expected_msg) in [
            ("", "empty Location"),
            ("   ", "empty Location"),
            ("file:///etc/passwd", "Unsupported redirect scheme"),
            (
                "ftp://archive.org/download/item/a",
                "Unsupported redirect scheme",
            ),
            ("http://[::1", "Invalid redirect Location"),
        ] {
            let err = resolve_redirect_target(&current, location).unwrap_err();
            assert!(
                matches!(err, IaGetError::Network(_)),
                "location '{location}': expected Network error, got {err:?}"
            );
            assert!(
                err.to_string().contains(expected_msg),
                "location '{location}': expected '{expected_msg}' in '{err}'"
            );
        }
    }

    static CURRENT_DIR_LOCK: Mutex<()> = Mutex::const_new(());

    struct CurrentDirGuard {
        original: PathBuf,
    }

    impl CurrentDirGuard {
        fn enter(target: &Path) -> Self {
            let original = std::env::current_dir().expect("read current working directory");
            std::env::set_current_dir(target).expect("switch current working directory");
            Self { original }
        }
    }

    impl Drop for CurrentDirGuard {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(&self.original);
        }
    }

    #[tokio::test]
    async fn test_prepare_output_dir_dot_returns_canonical_current_working_dir() {
        let _cwd_lock = CURRENT_DIR_LOCK.lock().await;
        let dir = TempTestDir::new("prepare-dot");
        let _cwd_guard = CurrentDirGuard::enter(&dir.path);

        let canonical = prepare_output_dir(Path::new("."))
            .expect("prepare_output_dir('.') should resolve current working directory");
        assert_eq!(canonical, fs::canonicalize(&dir.path).unwrap());
    }

    #[tokio::test]
    async fn test_download_files_with_policy_and_signal_writes_to_default_working_dir() {
        let _cwd_lock = CURRENT_DIR_LOCK.lock().await;
        let dir = TempTestDir::new("default-cwd-download");
        let _cwd_guard = CurrentDirGuard::enter(&dir.path);

        let payload_top = b"default working directory top-level payload";
        let payload_nested = b"default working directory nested payload";
        let md5_top = md5_hex(payload_top);
        let md5_nested = md5_hex(payload_nested);

        let server = MockServer::start(vec![
            MockResponse::ok(payload_top),
            MockResponse::ok(payload_nested),
        ])
        .await;
        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();

        let files = vec![
            (server.url("/top.bin"), "top.bin".to_string(), Some(md5_top)),
            (
                server.url("/nested.bin"),
                "sub/nested.bin".to_string(),
                Some(md5_nested),
            ),
        ];

        let summary = download_files_with_policy_and_signal(
            &client,
            files,
            2,
            None,
            &RetryPolicy::default(),
            &signal,
        )
        .await
        .expect("default working directory download should succeed");

        let requests = server.finish().await;
        assert_eq!(requests.len(), 2);
        assert_eq!(summary.ok, 2);
        assert_eq!(summary.skipped, 0);
        assert!(summary.failed.is_empty());
        assert!(!summary.interrupted);
        assert_eq!(fs::read(dir.join("top.bin")).unwrap(), payload_top);
        assert_eq!(
            fs::read(dir.join("sub/nested.bin")).unwrap(),
            payload_nested
        );
        assert!(!dir.part_path("top.bin").exists());
        assert!(!dir.part_path("sub/nested.bin").exists());
    }

    #[test]
    fn test_prepare_output_dir_creates_missing_nested_directories() {
        let sandbox = TempTestDir::new("prepare-nested");
        let target = sandbox.join("level1/level2/output");
        assert!(!target.exists());

        let canonical = prepare_output_dir(&target)
            .expect("prepare_output_dir should create missing nested directories");
        assert!(target.is_dir());
        assert_eq!(canonical, fs::canonicalize(&target).unwrap());

        let canonical_again = prepare_output_dir(&target)
            .expect("prepare_output_dir should accept already-created directory");
        assert_eq!(canonical_again, canonical);
    }

    #[tokio::test]
    async fn test_download_files_in_dir_creates_nested_output_dir_and_preserves_archive_paths() {
        let sandbox = TempTestDir::new("nested-output-dir");
        let output_dir = sandbox.join("downloads/archive/item");
        assert!(!output_dir.exists());

        let payload_top = b"top-level archive file";
        let payload_deep = b"deeply nested archive binary payload";
        let payload_sibling = b"second nested archive file";
        let md5_top = md5_hex(payload_top);
        let md5_deep = md5_hex(payload_deep);
        let md5_sibling = md5_hex(payload_sibling);

        let server = MockServer::start(vec![
            MockResponse::ok(payload_top),
            MockResponse::ok(payload_deep),
            MockResponse::ok(payload_sibling),
        ])
        .await;
        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();

        let files = vec![
            (server.url("/top.txt"), "top.txt".to_string(), Some(md5_top)),
            (
                server.url("/dir1/sub/file.bin"),
                "dir1/sub/file.bin".to_string(),
                Some(md5_deep),
            ),
            (
                server.url("/dir2/data.bin"),
                "dir2/data.bin".to_string(),
                Some(md5_sibling),
            ),
        ];

        let summary = download_files_in_dir(
            &client,
            &output_dir,
            files,
            3,
            None,
            &RetryPolicy::default(),
            &signal,
        )
        .await
        .expect("download_files_in_dir should create missing nested output dir");

        let requests = server.finish().await;
        assert_eq!(requests.len(), 3);
        assert!(output_dir.is_dir());
        assert_eq!(summary.ok, 3);
        assert_eq!(summary.skipped, 0);
        assert!(summary.failed.is_empty());
        assert!(!summary.interrupted);

        assert_eq!(fs::read(output_dir.join("top.txt")).unwrap(), payload_top);
        assert_eq!(
            fs::read(output_dir.join("dir1/sub/file.bin")).unwrap(),
            payload_deep
        );
        assert_eq!(
            fs::read(output_dir.join("dir2/data.bin")).unwrap(),
            payload_sibling
        );
        assert!(!output_dir.join(part_file_path("top.txt")).exists());
        assert!(!output_dir
            .join(part_file_path("dir1/sub/file.bin"))
            .exists());
        assert!(!output_dir.join(part_file_path("dir2/data.bin")).exists());

        let sandbox_entries: Vec<OsString> = fs::read_dir(&sandbox.path)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(
            sandbox_entries,
            vec![OsString::from("downloads")],
            "nothing should be written outside the target output directory"
        );
        assert!(!sandbox.join("top.txt").exists());
        assert!(!sandbox.join("dir1").exists());
    }

    #[tokio::test]
    async fn test_prepare_output_dir_and_download_reject_invalid_paths_with_zero_requests() {
        let sandbox = TempTestDir::new("invalid-output-paths");
        let existing_file = sandbox.join("existing.txt");
        fs::write(&existing_file, b"untouched-regular-file").unwrap();
        let file_ancestor = existing_file.join("sub");
        let empty_path = PathBuf::from("");

        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();

        for (label, bad_path) in [
            ("existing-file", &existing_file),
            ("file-ancestor", &file_ancestor),
            ("empty-path", &empty_path),
        ] {
            let prep_err = prepare_output_dir(bad_path).unwrap_err();
            assert!(
                matches!(prep_err, IaGetError::FileSystem(_)),
                "case {label}: expected FileSystem error from prepare_output_dir, got {prep_err:?}"
            );

            let server =
                MockServer::start(vec![MockResponse::ok(b"must never be requested")]).await;
            let dl_err = download_files_in_dir(
                &client,
                bad_path,
                vec![(server.url("/file.bin"), "file.bin".to_string(), None)],
                1,
                None,
                &RetryPolicy::default(),
                &signal,
            )
            .await
            .unwrap_err();
            let requests = server.abort_and_collect().await;

            assert!(
                matches!(dl_err, IaGetError::FileSystem(_)),
                "case {label}: expected FileSystem error from download_files_in_dir, got {dl_err:?}"
            );
            assert!(
                requests.is_empty(),
                "case {label}: expected 0 HTTP requests for invalid output path"
            );
        }

        assert_eq!(fs::read(&existing_file).unwrap(), b"untouched-regular-file");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_unix_prepare_output_dir_and_download_reject_broken_symlink_with_zero_requests() {
        use std::os::unix::fs::symlink;

        let sandbox = TempTestDir::new("broken-output-symlink");
        let missing_target = sandbox.join("missing-target");
        let broken_link = sandbox.join("broken-out");
        symlink(&missing_target, &broken_link).unwrap();

        let prep_err = prepare_output_dir(&broken_link).unwrap_err();
        assert!(
            matches!(prep_err, IaGetError::FileSystem(_)),
            "expected FileSystem error for broken symlink, got {prep_err:?}"
        );
        assert!(
            prep_err.to_string().contains("broken symlink"),
            "unexpected error message: {prep_err}"
        );

        let server = MockServer::start(vec![MockResponse::ok(b"must never be requested")]).await;
        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();

        let dl_err = download_files_in_dir(
            &client,
            &broken_link,
            vec![(server.url("/file.bin"), "file.bin".to_string(), None)],
            1,
            None,
            &RetryPolicy::default(),
            &signal,
        )
        .await
        .unwrap_err();
        let requests = server.abort_and_collect().await;

        assert!(
            matches!(dl_err, IaGetError::FileSystem(_)),
            "expected FileSystem error from download_files_in_dir, got {dl_err:?}"
        );
        assert!(
            requests.is_empty(),
            "broken symlink output path must send 0 HTTP requests"
        );
        assert!(!missing_target.exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_unix_symlink_escape_relative_to_output_dir_rejects_escapes_and_downloads_siblings(
    ) {
        use std::os::unix::fs::symlink;

        let sandbox = TempTestDir::new("output-dir-symlink-escape");
        let output_dir = sandbox.join("out");
        let outside_dir = sandbox.join("outside");
        fs::create_dir_all(&output_dir).unwrap();
        fs::create_dir_all(&outside_dir).unwrap();

        let outside_victim = outside_dir.join("victim.bin");
        fs::write(&outside_victim, b"untouched-outside-victim").unwrap();

        symlink(&outside_dir, output_dir.join("escape_dir")).unwrap();
        symlink(&outside_victim, output_dir.join("escape_file.bin")).unwrap();

        let canonical_out = prepare_output_dir(&output_dir).unwrap();
        assert!(matches!(
            validate_contained_path(&canonical_out, Path::new("escape_dir/payload.bin"), true),
            Err(IaGetError::FileSystem(_))
        ));
        assert!(matches!(
            validate_contained_path(&canonical_out, Path::new("escape_file.bin"), true),
            Err(IaGetError::FileSystem(_))
        ));

        let safe_payload = b"safe sibling downloaded inside output-dir";
        let safe_md5 = md5_hex(safe_payload);
        let server = MockServer::start(vec![MockResponse::ok(safe_payload)]).await;
        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();

        let files = vec![
            (
                server.url("/escape-dir.bin"),
                "escape_dir/payload.bin".to_string(),
                None,
            ),
            (
                server.url("/escape-file.bin"),
                "escape_file.bin".to_string(),
                None,
            ),
            (
                server.url("/safe.bin"),
                "safe/sibling.bin".to_string(),
                Some(safe_md5),
            ),
        ];

        let summary = download_files_in_dir(
            &client,
            &output_dir,
            files,
            3,
            None,
            &RetryPolicy::default(),
            &signal,
        )
        .await
        .expect("batch with escaped entries should return summary");

        let requests = server.finish().await;
        assert_eq!(
            requests.len(),
            1,
            "escaped entries must send 0 HTTP requests; only safe sibling should be fetched"
        );
        assert!(requests[0].request_line.starts_with("GET /safe.bin "));

        assert_eq!(summary.ok, 1);
        assert_eq!(summary.skipped, 0);
        assert!(!summary.interrupted);
        assert_eq!(summary.failed.len(), 2);
        assert_eq!(summary.failed[0].file_path, "escape_dir/payload.bin");
        assert!(
            summary.failed[0]
                .error
                .starts_with("File operation failed:"),
            "expected FileSystem error display, got: {}",
            summary.failed[0].error
        );
        assert_eq!(summary.failed[1].file_path, "escape_file.bin");
        assert!(
            summary.failed[1]
                .error
                .starts_with("File operation failed:"),
            "expected FileSystem error display, got: {}",
            summary.failed[1].error
        );

        assert!(!outside_dir.join("payload.bin").exists());
        assert!(!outside_dir.join("payload.bin.part").exists());
        assert_eq!(
            fs::read(&outside_victim).unwrap(),
            b"untouched-outside-victim"
        );
        assert_eq!(
            fs::read(output_dir.join("safe/sibling.bin")).unwrap(),
            safe_payload
        );
        assert!(!output_dir.join(part_file_path("safe/sibling.bin")).exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_unix_prepare_output_dir_allows_symlinked_output_dir_while_inner_escapes_fail() {
        use std::os::unix::fs::symlink;

        let sandbox = TempTestDir::new("symlinked-output-dir");
        let real_out = sandbox.join("real_out");
        let link_out = sandbox.join("link_out");
        let outside_dir = sandbox.join("outside");
        fs::create_dir_all(&real_out).unwrap();
        fs::create_dir_all(&outside_dir).unwrap();
        symlink(&real_out, &link_out).unwrap();
        symlink(&outside_dir, real_out.join("escape_dir")).unwrap();

        let canonical = prepare_output_dir(&link_out)
            .expect("symlinked --output-dir pointing to an existing directory should succeed");
        assert_eq!(canonical, fs::canonicalize(&real_out).unwrap());

        assert!(matches!(
            validate_contained_path(&canonical, Path::new("escape_dir/bad.bin"), true),
            Err(IaGetError::FileSystem(_))
        ));

        let safe_payload = b"downloaded via symlinked output-dir";
        let safe_md5 = md5_hex(safe_payload);
        let server = MockServer::start(vec![MockResponse::ok(safe_payload)]).await;
        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();

        let files = vec![
            (
                server.url("/bad.bin"),
                "escape_dir/bad.bin".to_string(),
                None,
            ),
            (
                server.url("/good.bin"),
                "nested/good.bin".to_string(),
                Some(safe_md5),
            ),
        ];

        let summary = download_files_in_dir(
            &client,
            &link_out,
            files,
            2,
            None,
            &RetryPolicy::default(),
            &signal,
        )
        .await
        .expect("batch into symlinked output-dir should return summary");

        let requests = server.finish().await;
        assert_eq!(requests.len(), 1);
        assert!(requests[0].request_line.starts_with("GET /good.bin "));
        assert_eq!(summary.ok, 1);
        assert_eq!(summary.failed.len(), 1);
        assert_eq!(summary.failed[0].file_path, "escape_dir/bad.bin");
        assert!(summary.failed[0]
            .error
            .starts_with("File operation failed:"));
        assert!(!outside_dir.join("bad.bin").exists());
        assert_eq!(
            fs::read(real_out.join("nested/good.bin")).unwrap(),
            safe_payload
        );
        assert!(!real_out.join(part_file_path("nested/good.bin")).exists());
    }

    #[cfg(unix)]
    #[test]
    fn test_unix_root_path_prepare_and_contained_validation() {
        let root = prepare_output_dir(Path::new("/"))
            .expect("Unix root path '/' should pass prepare_output_dir");
        assert_eq!(root, PathBuf::from("/"));

        let validated =
            validate_contained_path(&root, Path::new("ia-get-nonexistent/file.bin"), true)
                .expect("missing relative path under '/' should be accepted");
        assert_eq!(validated, PathBuf::from("/ia-get-nonexistent/file.bin"));

        let escape_err =
            validate_contained_path(&root, Path::new("../outside.bin"), true).unwrap_err();
        assert!(matches!(escape_err, IaGetError::FileSystem(_)));
    }

    #[cfg(windows)]
    #[test]
    fn test_windows_drive_root_prepare_and_escape_rejection() {
        let temp = std::env::temp_dir();
        let mut drive_root = PathBuf::new();
        for component in temp.components() {
            match component {
                Component::Prefix(_) | Component::RootDir => drive_root.push(component.as_os_str()),
                _ => break,
            }
        }
        assert!(
            drive_root.has_root(),
            "expected drive root from temp_dir '{}', got '{}'",
            temp.display(),
            drive_root.display()
        );

        let canonical_root = prepare_output_dir(&drive_root)
            .expect("Windows drive root should pass prepare_output_dir");
        assert_eq!(canonical_root, fs::canonicalize(&drive_root).unwrap());

        let validated = validate_contained_path(
            &canonical_root,
            Path::new("ia-get-nonexistent/file.bin"),
            true,
        )
        .expect("missing relative path under Windows drive root should succeed");
        assert_eq!(
            validated,
            canonical_root.join("ia-get-nonexistent").join("file.bin")
        );

        for unsafe_path in [
            Path::new("../escape.bin"),
            Path::new("nested/../../escape.bin"),
            Path::new(r"\absolute.bin"),
            Path::new(r"C:\escape.bin"),
        ] {
            let err = validate_contained_path(&canonical_root, unsafe_path, true).unwrap_err();
            assert!(
                matches!(err, IaGetError::FileSystem(_)),
                "path '{}' should be rejected as FileSystem error, got {err:?}",
                unsafe_path.display()
            );
        }
    }

    #[test]
    fn test_parse_rate_limit_valid_raw_and_suffixes() {
        const KIB: u64 = 1024;
        const MIB: u64 = 1024 * 1024;
        const GIB: u64 = 1024 * 1024 * 1024;

        let cases = [
            ("1", 1u64),
            ("512", 512),
            ("1024", 1024),
            ("1048576", 1_048_576),
            ("10B", 10),
            ("10b", 10),
            ("500K", 500 * KIB),
            ("500k", 500 * KIB),
            ("500KB", 500 * KIB),
            ("500kb", 500 * KIB),
            ("500Kb", 500 * KIB),
            ("500kB", 500 * KIB),
            ("2M", 2 * MIB),
            ("2m", 2 * MIB),
            ("2MB", 2 * MIB),
            ("2mb", 2 * MIB),
            ("1G", GIB),
            ("1g", GIB),
            ("1GB", GIB),
            ("1gb", GIB),
        ];

        for (input, expected) in cases {
            assert_eq!(
                parse_rate_limit(input).unwrap_or_else(|e| panic!("input '{input}' failed: {e}")),
                expected,
                "unexpected parsed rate for '{input}'"
            );
        }

        // Maximum non-overflowing u64 boundaries for raw bytes, B, K, M, and G.
        let max_raw = u64::MAX.to_string();
        assert_eq!(parse_rate_limit(&max_raw).unwrap(), u64::MAX);
        assert_eq!(parse_rate_limit(&format!("{max_raw}B")).unwrap(), u64::MAX);

        let max_k = u64::MAX / KIB;
        assert_eq!(parse_rate_limit(&format!("{max_k}K")).unwrap(), max_k * KIB);

        let max_m = u64::MAX / MIB;
        assert_eq!(parse_rate_limit(&format!("{max_m}M")).unwrap(), max_m * MIB);

        let max_g = u64::MAX / GIB;
        assert_eq!(parse_rate_limit(&format!("{max_g}G")).unwrap(), max_g * GIB);
    }

    #[test]
    fn test_parse_rate_limit_rejects_invalid_zero_negative_malformed_and_overflow() {
        const KIB: u64 = 1024;
        const MIB: u64 = 1024 * 1024;
        const GIB: u64 = 1024 * 1024 * 1024;

        let overflow_k = format!("{}K", (u64::MAX / KIB) + 1);
        let overflow_m = format!("{}M", (u64::MAX / MIB) + 1);
        let overflow_g = format!("{}G", (u64::MAX / GIB) + 1);

        let invalid_cases: Vec<(&str, &str)> = vec![
            ("", "empty"),
            (" ", "whitespace"),
            ("\t", "whitespace"),
            ("\n", "whitespace"),
            (" 500K", "whitespace"),
            ("500K ", "whitespace"),
            ("500 K", "whitespace"),
            ("5 00K", "whitespace"),
            ("0", "greater than zero"),
            ("00", "greater than zero"),
            ("0B", "greater than zero"),
            ("0K", "greater than zero"),
            ("0M", "greater than zero"),
            ("0G", "greater than zero"),
            ("-1", "without a sign"),
            ("-500K", "without a sign"),
            ("+100", "without a sign"),
            ("1.5M", "decimal"),
            ("0.5K", "decimal"),
            (".5M", "decimal"),
            ("K", "positive integer"),
            ("KB", "positive integer"),
            ("B", "positive integer"),
            ("500T", "unknown unit suffix"),
            ("500KiB", "unknown unit suffix"),
            ("500MB/s", "unknown unit suffix"),
            ("500xyz", "unknown unit suffix"),
            ("18446744073709551616", "exceeds maximum"),
            ("99999999999999999999999999999", "exceeds maximum"),
            (&overflow_k, "exceeds maximum"),
            (&overflow_m, "exceeds maximum"),
            (&overflow_g, "exceeds maximum"),
        ];

        for (case, expected_reason) in invalid_cases {
            let err = parse_rate_limit(case)
                .expect_err(&format!("expected '{case}' to fail rate limit parsing"));
            match &err {
                IaGetError::InvalidRateLimit { input, reason } => {
                    assert_eq!(input, case);
                    assert!(
                        reason.contains(expected_reason),
                        "expected reason for '{case}' to contain '{expected_reason}', got '{reason}'"
                    );
                }
                other => panic!("expected InvalidRateLimit for '{case}', got {other:?}"),
            }
            assert!(
                err.to_string()
                    .contains(&format!("Invalid rate limit '{case}':")),
                "unexpected Display for '{case}': {err}"
            );
        }
    }

    #[test]
    fn test_rate_limiter_zero_chunk_and_zero_rate() {
        let t0 = Instant::now();
        let mut limiter = RateLimiter::with_start(1024, t0);

        assert_eq!(limiter.delay_for_chunk(0, t0), Duration::ZERO);
        assert_eq!(limiter.delay_for_chunk(512, t0), Duration::from_millis(500));

        // Zero-byte chunks do not disturb accumulated state or trigger a stall reset.
        let t1 = t0 + Duration::from_millis(500);
        assert_eq!(limiter.delay_for_chunk(0, t1), Duration::ZERO);
        assert_eq!(
            limiter.delay_for_chunk(0, t0 + Duration::from_secs(10)),
            Duration::ZERO
        );
        assert_eq!(limiter.delay_for_chunk(512, t1), Duration::from_millis(500));

        // Zero rate returns Duration::ZERO without dividing by zero or panicking.
        let mut zero_rate = RateLimiter::with_start(0, t0);
        assert_eq!(zero_rate.delay_for_chunk(0, t0), Duration::ZERO);
        assert_eq!(zero_rate.delay_for_chunk(1024, t0), Duration::ZERO);
    }

    #[test]
    fn test_rate_limiter_single_chunk_exact_duration() {
        let t0 = Instant::now();
        let mut limiter = RateLimiter::with_start(1024, t0);
        assert_eq!(limiter.delay_for_chunk(512, t0), Duration::from_millis(500));

        let mut limiter_100k = RateLimiter::with_start(100_000, t0);
        assert_eq!(
            limiter_100k.delay_for_chunk(25_000, t0),
            Duration::from_millis(250)
        );
    }

    #[test]
    fn test_rate_limiter_multi_chunk_compensates_overshoot_without_drift() {
        let t0 = Instant::now();
        let mut limiter = RateLimiter::with_start(1000, t0);

        // Chunk 1: 500 bytes expects 500ms.
        assert_eq!(limiter.delay_for_chunk(500, t0), Duration::from_millis(500));

        // Sleep overshoots by 30ms (within the 100ms stall window): next delay shrinks to 470ms.
        let t1 = t0 + Duration::from_millis(530);
        assert_eq!(limiter.delay_for_chunk(500, t1), Duration::from_millis(470));

        // Sleep overshoots by another 20ms from the 1000ms target: next delay shrinks to 480ms.
        let t2 = t0 + Duration::from_millis(1020);
        assert_eq!(limiter.delay_for_chunk(500, t2), Duration::from_millis(480));

        // Back on schedule at 1500ms: chunk 4 requires the exact nominal 500ms.
        let t3 = t0 + Duration::from_millis(1500);
        assert_eq!(limiter.delay_for_chunk(500, t3), Duration::from_millis(500));
    }

    #[test]
    fn test_rate_limiter_tiny_chunks_accumulate_without_truncation() {
        let t0 = Instant::now();
        let rate_2g = 2 * 1024 * 1024 * 1024u64;
        let mut limiter = RateLimiter::with_start(rate_2g, t0);

        // Individual 1-byte chunks at 2 GiB/s are < 1ns, yet cumulative bytes must not lose credit.
        assert_eq!(limiter.delay_for_chunk(1, t0), Duration::ZERO);
        assert_eq!(limiter.delay_for_chunk(1, t0), Duration::ZERO);
        assert_eq!(limiter.delay_for_chunk(1, t0), Duration::from_nanos(1));

        let mut last_delay = Duration::ZERO;
        for _ in 3..2_148 {
            last_delay = limiter.delay_for_chunk(1, t0);
        }
        assert_eq!(last_delay, Duration::from_nanos(1_000));
    }

    #[test]
    fn test_rate_limiter_long_stall_resets_window_preventing_burst() {
        let t0 = Instant::now();
        let mut limiter = RateLimiter::with_start(1024, t0);

        // Chunk 1: 512 bytes at t0 -> 500ms expected.
        assert_eq!(limiter.delay_for_chunk(512, t0), Duration::from_millis(500));

        // Exactly at the 100ms stall boundary (elapsed == prev_expected + 100ms): no reset yet.
        let t_boundary = t0 + Duration::from_millis(600);
        assert_eq!(
            limiter.delay_for_chunk(512, t_boundary),
            Duration::from_millis(400)
        );

        // Exceed the 100ms stall window past prev_expected (1000ms + 101ms): window resets.
        let t_stalled = t0 + Duration::from_millis(1101);
        assert_eq!(
            limiter.delay_for_chunk(512, t_stalled),
            Duration::from_millis(500)
        );

        // A longer 500ms idle stall also resets cleanly rather than allowing a catch-up burst.
        let t_long_stall = t_stalled + Duration::from_secs(1);
        assert_eq!(
            limiter.delay_for_chunk(512, t_long_stall),
            Duration::from_millis(500)
        );
    }

    #[test]
    fn test_rate_limiter_large_cumulative_bytes_do_not_overflow() {
        const GIB: u64 = 1024 * 1024 * 1024;
        const TIB: u64 = 1024 * GIB;

        let t0 = Instant::now();
        let mut limiter = RateLimiter::with_start(GIB, t0);
        let mut last = Duration::ZERO;
        for _ in 0..10 {
            last = limiter.delay_for_chunk(TIB, t0);
        }
        assert_eq!(last, Duration::from_secs(10 * 1024));

        // Saturating accumulation past u64::MAX must not panic or overflow u128 arithmetic.
        let mut extreme = RateLimiter::with_start(GIB, t0);
        let _ = extreme.delay_for_chunk(u64::MAX / 2, t0);
        let _ = extreme.delay_for_chunk(u64::MAX / 2, t0);
        let saturated_delay = extreme.delay_for_chunk(1024, t0);
        assert_eq!(
            saturated_delay,
            RateLimiter::expected_duration_for_bytes(u64::MAX, GIB)
        );
    }

    #[tokio::test]
    async fn test_download_files_with_rate_limit_throttles_transfer_and_verifies_md5() {
        let payload: Vec<u8> = (0..(40 * 1024)).map(|i| (i % 251) as u8).collect();
        let expected_md5 = md5_hex(&payload);
        let server = MockServer::start(vec![MockResponse::ok(&payload)]).await;
        let client = http_client_builder().build().unwrap();
        let temp_dir = TempTestDir::new("rate-limit-throttled");
        let signal = ShutdownSignal::new();
        let options = DownloadOptions {
            retry_policy: RetryPolicy::default(),
            limit_rate: Some(parse_rate_limit("100K").unwrap()),
            verbose: false,
        };

        let files = vec![(
            server.url("/throttled.bin"),
            "throttled.bin".to_string(),
            Some(expected_md5.clone()),
        )];

        let started = Instant::now();
        let summary = download_files_in_dir_with_options(
            &client,
            &temp_dir.path,
            files,
            1,
            None,
            &options,
            &signal,
        )
        .await
        .expect("throttled download should succeed");
        let elapsed = started.elapsed();

        let requests = server.finish().await;
        assert_eq!(requests.len(), 1);
        assert_eq!(summary.ok, 1);
        assert_eq!(summary.skipped, 0);
        assert!(summary.failed.is_empty());
        assert!(!summary.interrupted);
        assert!(
            elapsed >= Duration::from_millis(320),
            "40 KiB at 100 KiB/s (~400ms expected) completed too quickly: {elapsed:?}"
        );

        let saved = fs::read(temp_dir.join("throttled.bin")).unwrap();
        assert_eq!(saved, payload);
        assert_eq!(md5_hex(&saved), expected_md5);
        assert!(!temp_dir.part_path("throttled.bin").exists());
    }

    #[tokio::test]
    async fn test_download_files_unthrottled_completes_with_intact_content_and_md5() {
        let payload: Vec<u8> = (0..(40 * 1024)).map(|i| ((i * 7) % 251) as u8).collect();
        let expected_md5 = md5_hex(&payload);
        let server = MockServer::start(vec![MockResponse::ok(&payload)]).await;
        let client = http_client_builder().build().unwrap();
        let temp_dir = TempTestDir::new("rate-limit-unthrottled");
        let signal = ShutdownSignal::new();
        let options = DownloadOptions {
            retry_policy: RetryPolicy::default(),
            limit_rate: None,
            verbose: false,
        };

        let files = vec![(
            server.url("/unthrottled.bin"),
            "unthrottled.bin".to_string(),
            Some(expected_md5.clone()),
        )];

        let summary = download_files_in_dir_with_options(
            &client,
            &temp_dir.path,
            files,
            1,
            None,
            &options,
            &signal,
        )
        .await
        .expect("unthrottled download should succeed");

        let requests = server.finish().await;
        assert_eq!(requests.len(), 1);
        assert_eq!(summary.ok, 1);
        assert_eq!(summary.skipped, 0);
        assert!(summary.failed.is_empty());
        assert!(!summary.interrupted);

        let saved = fs::read(temp_dir.join("unthrottled.bin")).unwrap();
        assert_eq!(saved, payload);
        assert_eq!(md5_hex(&saved), expected_md5);
        assert!(!temp_dir.part_path("unthrottled.bin").exists());
    }

    #[tokio::test]
    async fn test_download_files_with_rate_limit_interrupts_immediately_on_shutdown_signal() {
        let payload: Vec<u8> = (0..(16 * 1024)).map(|i| (i % 239) as u8).collect();
        let expected_md5 = md5_hex(&payload);
        let server = MockServer::start(vec![MockResponse::ok(&payload)]).await;
        let client = http_client_builder().build().unwrap();
        let temp_dir = TempTestDir::new("rate-limit-interrupt");
        let signal = ShutdownSignal::new();
        let options = DownloadOptions {
            retry_policy: RetryPolicy::default(),
            limit_rate: Some(parse_rate_limit("1K").unwrap()),
            verbose: false,
        };

        let part_path = temp_dir.part_path("interrupted.bin");
        let trigger_signal = signal.clone();
        let watch_part = part_path.clone();
        let trigger_task = tokio::spawn(async move {
            let deadline = tokio::time::Instant::now() + Duration::from_millis(1500);
            while tokio::time::Instant::now() < deadline {
                if fs::metadata(&watch_part)
                    .map(|m| m.len() > 0)
                    .unwrap_or(false)
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
            trigger_signal.trigger();
        });

        let files = vec![(
            server.url("/interrupted.bin"),
            "interrupted.bin".to_string(),
            Some(expected_md5),
        )];

        let started = Instant::now();
        let summary = download_files_in_dir_with_options(
            &client,
            &temp_dir.path,
            files,
            1,
            None,
            &options,
            &signal,
        )
        .await
        .expect("interrupted throttled batch should return summary");
        let elapsed = started.elapsed();

        let _ = trigger_task.await;
        let _requests = server.abort_and_collect().await;

        assert!(
            elapsed < Duration::from_secs(2),
            "Ctrl+C must interrupt ~16s throttled sleep promptly, took {elapsed:?}"
        );
        assert!(summary.interrupted);
        assert_eq!(summary.ok, 0);
        assert_eq!(summary.skipped, 0);
        assert!(summary.failed.is_empty());
        assert!(!temp_dir.join("interrupted.bin").exists());

        let retained = fs::read(&part_path).expect(".part file must remain on disk");
        assert!(!retained.is_empty(), ".part file must retain written bytes");
        assert!(retained.len() <= payload.len());
        assert_eq!(retained.as_slice(), &payload[..retained.len()]);
    }

    #[tokio::test]
    async fn test_download_files_resumed_with_rate_limit_paces_only_remaining_bytes() {
        let prefix_len = 120 * 1024usize;
        let remainder_len = 20 * 1024usize;
        let total_len = prefix_len + remainder_len;
        let payload: Vec<u8> = (0..total_len).map(|i| ((i * 13) % 251) as u8).collect();
        let expected_md5 = md5_hex(&payload);

        let content_range = format!("bytes {prefix_len}-{}/{total_len}", total_len - 1);
        let server = MockServer::start(vec![MockResponse::with_headers(
            "206 Partial Content",
            vec![
                ("Content-Range", content_range),
                ("Content-Length", remainder_len.to_string()),
            ],
            &payload[prefix_len..],
        )])
        .await;

        let client = http_client_builder().build().unwrap();
        let temp_dir = TempTestDir::new("rate-limit-resume");
        fs::write(temp_dir.part_path("resumed.bin"), &payload[..prefix_len]).unwrap();

        let signal = ShutdownSignal::new();
        let options = DownloadOptions {
            retry_policy: RetryPolicy::default(),
            limit_rate: Some(parse_rate_limit("100K").unwrap()),
            verbose: false,
        };

        let files = vec![(
            server.url("/resumed.bin"),
            "resumed.bin".to_string(),
            Some(expected_md5.clone()),
        )];

        let started = Instant::now();
        let summary = download_files_in_dir_with_options(
            &client,
            &temp_dir.path,
            files,
            1,
            None,
            &options,
            &signal,
        )
        .await
        .expect("resumed throttled download should succeed");
        let elapsed = started.elapsed();

        let requests = server.finish().await;
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].header("Range"),
            Some(format!("bytes={prefix_len}-").as_str())
        );

        assert_eq!(summary.ok, 1);
        assert_eq!(summary.skipped, 0);
        assert!(summary.failed.is_empty());
        assert!(!summary.interrupted);
        assert!(
            elapsed >= Duration::from_millis(140),
            "20 KiB remainder at 100 KiB/s (~200ms expected) completed too quickly: {elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_millis(1000),
            "pacing must apply only to the 20 KiB remainder (~200ms), not the 140 KiB full file (~1400ms): {elapsed:?}"
        );

        let saved = fs::read(temp_dir.join("resumed.bin")).unwrap();
        assert_eq!(saved, payload);
        assert_eq!(md5_hex(&saved), expected_md5);
        assert!(!temp_dir.part_path("resumed.bin").exists());
    }

    #[test]
    fn proxy_config_prefers_lowercase_without_falling_back_to_uppercase() {
        let lookup = |name: &str| match name {
            "http_proxy" => Some(String::new()),
            "HTTP_PROXY" => Some("http://upper-http.example.com:8080".to_string()),
            "https_proxy" => Some("   ".to_string()),
            "HTTPS_PROXY" => Some("https://upper-https.example.com:8443".to_string()),
            "all_proxy" => Some("  socks5h://127.0.0.1:1080  ".to_string()),
            "ALL_PROXY" => Some("socks5://upper-all.example.com:1080".to_string()),
            "no_proxy" => Some(String::new()),
            "NO_PROXY" => Some("archive.org".to_string()),
            _ => None,
        };

        let config = ProxyConfig::from_lookup(lookup).unwrap();
        assert_eq!(config.http_proxy, None);
        assert_eq!(config.https_proxy, None);
        assert_eq!(
            config.all_proxy,
            Some(Url::parse("socks5h://127.0.0.1:1080").unwrap())
        );
        assert_eq!(config.no_proxy, None);
    }

    #[test]
    fn proxy_config_falls_back_to_uppercase_when_lowercase_unset() {
        let lookup = |name: &str| match name {
            "HTTP_PROXY" => Some("http://proxy.example.com:8080".to_string()),
            "HTTPS_PROXY" => Some("https://secure.example.com:8443".to_string()),
            "ALL_PROXY" => Some("socks5://127.0.0.1:1080".to_string()),
            "NO_PROXY" => Some(" localhost, .archive.org ".to_string()),
            _ => None,
        };

        let config = ProxyConfig::from_lookup(lookup).unwrap();
        assert_eq!(
            config.http_proxy,
            Some(Url::parse("http://proxy.example.com:8080").unwrap())
        );
        assert_eq!(
            config.https_proxy,
            Some(Url::parse("https://secure.example.com:8443").unwrap())
        );
        assert_eq!(
            config.all_proxy,
            Some(Url::parse("socks5://127.0.0.1:1080").unwrap())
        );
        assert_eq!(config.no_proxy.as_deref(), Some("localhost, .archive.org"));
    }

    #[test]
    fn proxy_config_accepts_all_supported_schemes_and_builds_client() {
        for raw in [
            "http://127.0.0.1:8080",
            "https://127.0.0.1:8443",
            "socks4://127.0.0.1:1080",
            "socks4a://127.0.0.1:1080",
            "socks5://127.0.0.1:1080",
            "socks5h://user:pass@127.0.0.1:1080",
        ] {
            let config = ProxyConfig::from_lookup(|name| {
                if name == "ALL_PROXY" {
                    Some(raw.to_string())
                } else {
                    None
                }
            })
            .unwrap_or_else(|err| panic!("expected '{raw}' to be valid, got {err}"));

            http_client_builder_with_proxy(&config)
                .unwrap_or_else(|err| panic!("builder failed for '{raw}': {err}"))
                .build()
                .unwrap_or_else(|err| panic!("client build failed for '{raw}': {err}"));
        }
    }

    #[test]
    fn proxy_config_rejects_invalid_urls_and_redacts_credentials() {
        let invalid_cases = [
            ("HTTP_PROXY", "proxy.example.com:8080", None),
            (
                "HTTP_PROXY",
                "user:topsecret@proxy.example.com:8080",
                Some("topsecret"),
            ),
            (
                "HTTPS_PROXY",
                "ftp://alice:s3cr3t@proxy.example.com:21",
                Some("s3cr3t"),
            ),
            ("ALL_PROXY", "socks://127.0.0.1:1080", None),
            (
                "HTTP_PROXY",
                "http://user:pw@proxy.example.com:0",
                Some("pw"),
            ),
            ("ALL_PROXY", "socks5://:1080", None),
            ("ALL_PROXY", "socks5:///path", None),
            (
                "HTTP_PROXY",
                "http://user:hunter2@proxy host:8080",
                Some("hunter2"),
            ),
            (
                "HTTP_PROXY",
                "http://user:hidden@proxy.example.com:8080\0",
                Some("hidden"),
            ),
            ("HTTP_PROXY", "http:example.com/a://b", None),
        ];

        for (var_name, raw, secret) in invalid_cases {
            let err = ProxyConfig::from_lookup(|name| {
                if name == var_name {
                    Some(raw.to_string())
                } else {
                    None
                }
            })
            .expect_err(&format!("expected '{raw}' to fail"));

            let rendered = err.to_string();
            match &err {
                IaGetError::InvalidProxy { var, value, .. } => {
                    assert_eq!(var, var_name);
                    if let Some(secret) = secret {
                        assert!(!value.contains(secret), "secret leaked in value: {value}");
                        assert!(
                            value.contains("<redacted>@"),
                            "expected redaction marker in value: {value}"
                        );
                        assert!(
                            !rendered.contains(secret),
                            "secret leaked in Display: {rendered}"
                        );
                    }
                }
                other => panic!("expected InvalidProxy for '{raw}', got {other:?}"),
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn proxy_config_rejects_non_utf8_os_value() {
        use std::os::unix::ffi::OsStringExt;

        let err = ProxyConfig::from_os_lookup(|name| match name {
            "http_proxy" => Some(OsString::from_vec(vec![0xff, 0xfe])),
            "HTTP_PROXY" => Some(OsString::from("http://127.0.0.1:8080")),
            _ => None,
        })
        .expect_err("non-UTF-8 http_proxy must fail without falling back");

        match err {
            IaGetError::InvalidProxy { var, value, .. } => {
                assert_eq!(var, "http_proxy");
                assert_eq!(value, "<non-UTF-8>");
            }
            other => panic!("expected InvalidProxy, got {other:?}"),
        }
    }

    #[test]
    fn proxy_config_empty_and_whitespace_only_env_produces_direct_client() {
        let empty = ProxyConfig::from_lookup(|_| None).unwrap();
        assert_eq!(empty, ProxyConfig::default());
        http_client_builder_with_proxy(&empty)
            .expect("empty proxy config should build client builder")
            .build()
            .expect("empty proxy config should build direct client");

        let whitespace_only = ProxyConfig::from_lookup(|name| match name {
            "HTTP_PROXY" => Some(String::new()),
            "HTTPS_PROXY" => Some("   ".to_string()),
            "ALL_PROXY" => Some("\t \n".to_string()),
            "NO_PROXY" => Some("  \t ".to_string()),
            _ => None,
        })
        .unwrap();
        assert_eq!(whitespace_only, ProxyConfig::default());
        http_client_builder_with_proxy(&whitespace_only)
            .expect("whitespace-only proxy config should build client builder")
            .build()
            .expect("whitespace-only proxy config should build direct client");
    }

    #[test]
    fn proxy_config_non_empty_lowercase_overrides_uppercase_for_all_vars() {
        let config = ProxyConfig::from_lookup(|name| match name {
            "http_proxy" => Some("http://lower-http.example:8080".to_string()),
            "HTTP_PROXY" => Some("http://upper-http.example:8080".to_string()),
            "https_proxy" => Some("https://lower-https.example:8443".to_string()),
            "HTTPS_PROXY" => Some("https://upper-https.example:8443".to_string()),
            "all_proxy" => Some("socks5://lower-all.example:1080".to_string()),
            "ALL_PROXY" => Some("socks5://upper-all.example:1080".to_string()),
            "no_proxy" => Some("127.0.0.1,localhost".to_string()),
            "NO_PROXY" => Some("archive.org".to_string()),
            _ => None,
        })
        .unwrap();

        assert_eq!(
            config.http_proxy,
            Some(Url::parse("http://lower-http.example:8080").unwrap())
        );
        assert_eq!(
            config.https_proxy,
            Some(Url::parse("https://lower-https.example:8443").unwrap())
        );
        assert_eq!(
            config.all_proxy,
            Some(Url::parse("socks5://lower-all.example:1080").unwrap())
        );
        assert_eq!(config.no_proxy.as_deref(), Some("127.0.0.1,localhost"));
    }

    #[test]
    fn proxy_config_accepts_case_insensitive_schemes_ipv6_credentials_and_omitted_socks_port() {
        let valid_cases = [
            ("HTTP://127.0.0.1:8080", "http"),
            ("HTTPS://127.0.0.1:8443", "https"),
            ("SOCKS4://127.0.0.1:1080", "socks4"),
            ("SOCKS4A://127.0.0.1:1080", "socks4a"),
            ("SOCKS5://127.0.0.1:1080", "socks5"),
            ("SOCKS5H://127.0.0.1:1080", "socks5h"),
            ("http://[::1]:8080", "http"),
            ("https://[::1]:8443", "https"),
            ("socks5://[::1]:1080", "socks5"),
            ("socks5h://[::1]:1080", "socks5h"),
            ("socks4://127.0.0.1", "socks4"),
            ("socks4a://proxy.example", "socks4a"),
            ("socks5://127.0.0.1", "socks5"),
            ("socks5h://proxy.example", "socks5h"),
            ("http://alice:secret@127.0.0.1:8080", "http"),
            ("https://token@127.0.0.1:8443", "https"),
            ("socks5://bob:pw@[::1]:1080", "socks5"),
        ];

        for (raw, expected_scheme) in valid_cases {
            let config = ProxyConfig::from_lookup(|name| {
                if name == "ALL_PROXY" {
                    Some(raw.to_string())
                } else {
                    None
                }
            })
            .unwrap_or_else(|err| panic!("expected '{raw}' to be valid, got {err}"));

            let parsed = config.all_proxy.as_ref().expect("all_proxy should be set");
            assert_eq!(parsed.scheme(), expected_scheme);

            http_client_builder_with_proxy(&config)
                .unwrap_or_else(|err| panic!("builder failed for '{raw}': {err}"))
                .build()
                .unwrap_or_else(|err| panic!("client build failed for '{raw}': {err}"));
        }
    }

    #[test]
    fn proxy_config_rejects_additional_malformed_urls_and_redacts_complex_credentials() {
        let invalid_cases = [
            ("HTTP_PROXY", "not-a-url", None),
            ("HTTP_PROXY", "proxy.local:8080", None),
            ("HTTP_PROXY", "http://", None),
            ("HTTP_PROXY", "http://:8080", None),
            ("ALL_PROXY", "socks5://", None),
            ("HTTP_PROXY", "http://127.0.0.1:99999", None),
            ("HTTPS_PROXY", "https://127.0.0.1:abc", None),
            ("ALL_PROXY", "socks5://127.0.0.1:0", None),
            ("HTTP_PROXY", "://bad", None),
            ("ALL_PROXY", "socks5://127.0.0.1\t:1080", None),
            (
                "HTTP_PROXY",
                "http://user:p@ssword@proxy.local:99999",
                Some("p@ssword"),
            ),
            (
                "ALL_PROXY",
                "socks5://alice:supersecret@:1080",
                Some("supersecret"),
            ),
        ];

        for (var_name, raw, secret) in invalid_cases {
            let err = ProxyConfig::from_lookup(|name| {
                if name == var_name {
                    Some(raw.to_string())
                } else {
                    None
                }
            })
            .expect_err(&format!("expected '{raw}' to fail"));

            let rendered = err.to_string();
            match &err {
                IaGetError::InvalidProxy { var, value, .. } => {
                    assert_eq!(var, var_name);
                    if let Some(secret) = secret {
                        assert!(!value.contains(secret), "secret leaked in value: {value}");
                        assert!(
                            value.contains("<redacted>@"),
                            "expected redaction marker in value: {value}"
                        );
                        assert!(
                            !rendered.contains(secret),
                            "secret leaked in Display: {rendered}"
                        );
                    }
                }
                other => panic!("expected InvalidProxy for '{raw}', got {other:?}"),
            }
        }

        // Direct builder validation also rejects unsupported schemes and redacts credentials.
        let direct_invalid = ProxyConfig {
            http_proxy: Some(Url::parse("ftp://alice:s3cr3t@proxy.local:21").unwrap()),
            ..ProxyConfig::default()
        };
        let err = http_client_builder_with_proxy(&direct_invalid)
            .expect_err("direct invalid scheme must be rejected");
        let rendered = err.to_string();
        match err {
            IaGetError::InvalidProxy { var, value, .. } => {
                assert_eq!(var, "HTTP_PROXY");
                assert!(value.contains("<redacted>@"), "unexpected value: {value}");
                assert!(!value.contains("s3cr3t"), "secret leaked in value: {value}");
                assert!(
                    !rendered.contains("s3cr3t"),
                    "secret leaked in Display: {rendered}"
                );
            }
            other => panic!("expected InvalidProxy, got {other:?}"),
        }
    }

    #[test]
    fn proxy_config_redacts_credentials_with_delimiters_and_invalid_scheme_prefixes() {
        let cases = [
            (
                "http://alice:p/ass@proxy.local:8080",
                "http://<redacted>@proxy.local:8080",
            ),
            (
                "http://alice:p?w@proxy.local:8080",
                "http://<redacted>@proxy.local:8080",
            ),
            (
                "http://alice:p#w@proxy.local:8080",
                "http://<redacted>@proxy.local:8080",
            ),
            (
                "alice:p/ass@proxy.local:8080",
                "<redacted>@proxy.local:8080",
            ),
            (
                "http:alice:secret@proxy.local/a://b",
                "<redacted>@proxy.local/a://b",
            ),
        ];
        let forbidden = ["alice", "p/ass", "p?w", "p#w", "secret"];

        for (raw, expected_redacted) in cases {
            let err = ProxyConfig::from_lookup(|name| {
                if name == "HTTP_PROXY" {
                    Some(raw.to_string())
                } else {
                    None
                }
            })
            .expect_err(&format!("expected '{raw}' to fail"));

            let rendered = err.to_string();
            match &err {
                IaGetError::InvalidProxy { var, value, .. } => {
                    assert_eq!(var, "HTTP_PROXY");
                    assert_eq!(value, expected_redacted);
                    assert!(
                        value.contains("<redacted>@"),
                        "expected redaction marker in value for '{raw}': {value}"
                    );
                    assert!(
                        rendered.contains("<redacted>@"),
                        "expected redaction marker in Display for '{raw}': {rendered}"
                    );
                    for needle in forbidden {
                        assert!(
                            !value.contains(needle),
                            "credential fragment '{needle}' leaked in value for '{raw}': {value}"
                        );
                        assert!(
                            !rendered.contains(needle),
                            "credential fragment '{needle}' leaked in Display for '{raw}': {rendered}"
                        );
                    }
                }
                other => panic!("expected InvalidProxy for '{raw}', got {other:?}"),
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn proxy_config_rejects_non_utf8_uppercase_and_no_proxy_os_values() {
        use std::os::unix::ffi::OsStringExt;

        let all_err = ProxyConfig::from_os_lookup(|name| match name {
            "ALL_PROXY" => Some(OsString::from_vec(vec![0xff, 0x80])),
            _ => None,
        })
        .expect_err("non-UTF-8 ALL_PROXY must fail");
        match all_err {
            IaGetError::InvalidProxy { var, value, .. } => {
                assert_eq!(var, "ALL_PROXY");
                assert_eq!(value, "<non-UTF-8>");
            }
            other => panic!("expected InvalidProxy, got {other:?}"),
        }

        let no_proxy_err = ProxyConfig::from_os_lookup(|name| match name {
            "NO_PROXY" => Some(OsString::from_vec(vec![0xfe, 0xff])),
            _ => None,
        })
        .expect_err("non-UTF-8 NO_PROXY must fail");
        match no_proxy_err {
            IaGetError::InvalidProxy { var, value, .. } => {
                assert_eq!(var, "NO_PROXY");
                assert_eq!(value, "<non-UTF-8>");
            }
            other => panic!("expected InvalidProxy, got {other:?}"),
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum MockSocks5Target {
        Ipv4([u8; 4], u16),
        Domain(String, u16),
        Ipv6([u8; 16], u16),
    }

    #[derive(Debug, Clone)]
    struct CapturedSocks5Connection {
        target: MockSocks5Target,
        request: CapturedRequest,
    }

    struct MockSocks5Server {
        addr: std::net::SocketAddr,
        connections: Arc<Mutex<Vec<CapturedSocks5Connection>>>,
        handle: Option<tokio::task::JoinHandle<()>>,
    }

    impl MockSocks5Server {
        async fn start(responses: Vec<MockResponse>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind mock SOCKS5 server");
            let addr = listener.local_addr().expect("mock SOCKS5 local_addr");
            let connections = Arc::new(Mutex::new(Vec::new()));
            let connections_clone = Arc::clone(&connections);

            let handle = tokio::spawn(async move {
                for response in responses {
                    let Ok((mut socket, _)) = listener.accept().await else {
                        break;
                    };
                    let _ = socket.set_nodelay(true);

                    let Some(target) = socks5_handshake(&mut socket).await else {
                        continue;
                    };
                    let Some(request) = read_http_request(&mut socket).await else {
                        continue;
                    };
                    connections_clone
                        .lock()
                        .await
                        .push(CapturedSocks5Connection { target, request });
                    write_mock_response(&mut socket, response).await;
                }
            });

            Self {
                addr,
                connections,
                handle: Some(handle),
            }
        }

        fn proxy_url(&self, scheme: &str) -> String {
            format!("{scheme}://127.0.0.1:{}", self.addr.port())
        }

        async fn finish(mut self) -> Vec<CapturedSocks5Connection> {
            if let Some(handle) = self.handle.take() {
                let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;
            }
            self.connections.lock().await.clone()
        }

        async fn abort_and_collect(mut self) -> Vec<CapturedSocks5Connection> {
            if let Some(handle) = self.handle.take() {
                handle.abort();
                let _ = handle.await;
            }
            self.connections.lock().await.clone()
        }
    }

    impl Drop for MockSocks5Server {
        fn drop(&mut self) {
            if let Some(handle) = self.handle.take() {
                handle.abort();
            }
        }
    }

    async fn socks5_handshake(socket: &mut TcpStream) -> Option<MockSocks5Target> {
        // RFC 1928 greeting: VER=0x05, NMETHODS, METHODS[NMETHODS]
        let mut greeting = [0u8; 2];
        socket.read_exact(&mut greeting).await.ok()?;
        if greeting[0] != 0x05 {
            return None;
        }
        let mut methods = vec![0u8; usize::from(greeting[1])];
        socket.read_exact(&mut methods).await.ok()?;
        // Select NO AUTHENTICATION REQUIRED (0x00)
        socket.write_all(&[0x05, 0x00]).await.ok()?;
        socket.flush().await.ok()?;

        // RFC 1928 request: VER=0x05, CMD=0x01 (CONNECT), RSV=0x00, ATYP
        let mut req_head = [0u8; 4];
        socket.read_exact(&mut req_head).await.ok()?;
        if req_head[0] != 0x05 || req_head[1] != 0x01 || req_head[2] != 0x00 {
            return None;
        }

        let target = match req_head[3] {
            0x01 => {
                let mut ipv4 = [0u8; 4];
                socket.read_exact(&mut ipv4).await.ok()?;
                let mut port_bytes = [0u8; 2];
                socket.read_exact(&mut port_bytes).await.ok()?;
                MockSocks5Target::Ipv4(ipv4, u16::from_be_bytes(port_bytes))
            }
            0x03 => {
                let mut len = [0u8; 1];
                socket.read_exact(&mut len).await.ok()?;
                let mut domain = vec![0u8; usize::from(len[0])];
                socket.read_exact(&mut domain).await.ok()?;
                let mut port_bytes = [0u8; 2];
                socket.read_exact(&mut port_bytes).await.ok()?;
                let domain_str = String::from_utf8(domain).ok()?;
                MockSocks5Target::Domain(domain_str, u16::from_be_bytes(port_bytes))
            }
            0x04 => {
                let mut ipv6 = [0u8; 16];
                socket.read_exact(&mut ipv6).await.ok()?;
                let mut port_bytes = [0u8; 2];
                socket.read_exact(&mut port_bytes).await.ok()?;
                MockSocks5Target::Ipv6(ipv6, u16::from_be_bytes(port_bytes))
            }
            _ => return None,
        };

        // Reply: VER=0x05, REP=0x00 (succeeded), RSV=0x00, ATYP=0x01 (IPv4), 127.0.0.1:0
        socket
            .write_all(&[0x05, 0x00, 0x00, 0x01, 127, 0, 0, 1, 0, 0])
            .await
            .ok()?;
        socket.flush().await.ok()?;

        Some(target)
    }

    #[tokio::test]
    async fn proxy_http_proxy_routes_download_and_verifies_md5() {
        let payload = b"downloaded through local mock HTTP proxy".to_vec();
        let expected_md5 = md5_hex(&payload);
        let proxy_server = MockServer::start(vec![MockResponse::ok(&payload)]).await;
        let proxy_url = format!("http://127.0.0.1:{}", proxy_server.addr.port());

        let config = ProxyConfig::from_lookup(|name| match name {
            "HTTP_PROXY" => Some(proxy_url.clone()),
            _ => None,
        })
        .unwrap();
        let client = http_client_builder_with_proxy(&config)
            .unwrap()
            .build()
            .unwrap();

        let temp_dir = TempTestDir::new("http-proxy-route");
        let signal = ShutdownSignal::new();
        let outcome = download_single_file(
            &client,
            &temp_dir.path,
            "http://archive.example/download/item/test.bin",
            "test.bin",
            Some(&expected_md5),
            &signal,
            None,
        )
        .await
        .expect("HTTP_PROXY download should succeed");

        assert_eq!(outcome, FileOutcome::Downloaded);
        let requests = proxy_server.finish().await;
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].request_line,
            "GET http://archive.example/download/item/test.bin HTTP/1.1"
        );

        let saved = fs::read(temp_dir.join("test.bin")).unwrap();
        assert_eq!(saved, payload);
        assert_eq!(md5_hex(&saved), expected_md5);
        assert!(!temp_dir.part_path("test.bin").exists());
    }

    #[tokio::test]
    async fn proxy_all_proxy_fallback_and_http_proxy_precedence() {
        // Fallback: ALL_PROXY handles HTTP when HTTP_PROXY is unset.
        {
            let payload = b"served via ALL_PROXY fallback".to_vec();
            let expected_md5 = md5_hex(&payload);
            let all_proxy_server = MockServer::start(vec![MockResponse::ok(&payload)]).await;
            let all_proxy_url = format!("http://127.0.0.1:{}", all_proxy_server.addr.port());

            let config = ProxyConfig::from_lookup(|name| match name {
                "ALL_PROXY" => Some(all_proxy_url.clone()),
                _ => None,
            })
            .unwrap();
            let client = http_client_builder_with_proxy(&config)
                .unwrap()
                .build()
                .unwrap();

            let temp_dir = TempTestDir::new("all-proxy-fallback");
            let signal = ShutdownSignal::new();
            let outcome = download_single_file(
                &client,
                &temp_dir.path,
                "http://archive.example/download/item/fallback.bin",
                "fallback.bin",
                Some(&expected_md5),
                &signal,
                None,
            )
            .await
            .expect("ALL_PROXY fallback download should succeed");

            assert_eq!(outcome, FileOutcome::Downloaded);
            let requests = all_proxy_server.finish().await;
            assert_eq!(requests.len(), 1);
            assert_eq!(
                requests[0].request_line,
                "GET http://archive.example/download/item/fallback.bin HTTP/1.1"
            );
            assert_eq!(fs::read(temp_dir.join("fallback.bin")).unwrap(), payload);
        }

        // Precedence: HTTP_PROXY overrides ALL_PROXY when both are set.
        {
            let payload = b"served via HTTP_PROXY precedence".to_vec();
            let expected_md5 = md5_hex(&payload);
            let http_proxy_server = MockServer::start(vec![MockResponse::ok(&payload)]).await;
            let all_proxy_server = MockServer::start(vec![MockResponse::ok(b"wrong-proxy")]).await;
            let http_proxy_url = format!("http://127.0.0.1:{}", http_proxy_server.addr.port());
            let all_proxy_url = format!("http://127.0.0.1:{}", all_proxy_server.addr.port());

            let config = ProxyConfig::from_lookup(|name| match name {
                "HTTP_PROXY" => Some(http_proxy_url.clone()),
                "ALL_PROXY" => Some(all_proxy_url.clone()),
                _ => None,
            })
            .unwrap();
            let client = http_client_builder_with_proxy(&config)
                .unwrap()
                .build()
                .unwrap();

            let temp_dir = TempTestDir::new("http-proxy-precedence");
            let signal = ShutdownSignal::new();
            let outcome = download_single_file(
                &client,
                &temp_dir.path,
                "http://archive.example/download/item/precedence.bin",
                "precedence.bin",
                Some(&expected_md5),
                &signal,
                None,
            )
            .await
            .expect("HTTP_PROXY precedence download should succeed");

            assert_eq!(outcome, FileOutcome::Downloaded);
            let http_requests = http_proxy_server.finish().await;
            let all_requests = all_proxy_server.abort_and_collect().await;
            assert_eq!(http_requests.len(), 1);
            assert_eq!(
                http_requests[0].request_line,
                "GET http://archive.example/download/item/precedence.bin HTTP/1.1"
            );
            assert!(
                all_requests.is_empty(),
                "ALL_PROXY must not be contacted when HTTP_PROXY matches"
            );
            assert_eq!(fs::read(temp_dir.join("precedence.bin")).unwrap(), payload);
        }
    }

    #[tokio::test]
    async fn proxy_https_proxy_sends_connect_without_external_network() {
        let proxy_server = MockServer::start(vec![MockResponse::with_headers(
            "502 Bad Gateway",
            vec![("Content-Length", "0".to_string())],
            b"",
        )])
        .await;
        let proxy_url = format!("http://127.0.0.1:{}", proxy_server.addr.port());

        let config = ProxyConfig::from_lookup(|name| match name {
            "HTTPS_PROXY" => Some(proxy_url.clone()),
            _ => None,
        })
        .unwrap();
        let client = http_client_builder_with_proxy(&config)
            .unwrap()
            .build()
            .unwrap();

        let temp_dir = TempTestDir::new("https-proxy-connect");
        let signal = ShutdownSignal::new();
        let policy = RetryPolicy {
            max_retries: 0,
            initial_delay: Duration::from_millis(5),
            max_delay: Duration::from_millis(10),
            max_retry_after: Duration::from_millis(50),
        };

        let result = download_single_file_with_policy(
            &client,
            &temp_dir.path,
            "https://archive.example/download/item/test.bin",
            "test.bin",
            None,
            &signal,
            None,
            &policy,
        )
        .await;
        assert!(
            result.is_err(),
            "CONNECT rejection with 502 Bad Gateway should fail the download"
        );

        let requests = proxy_server.finish().await;
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].request_line,
            "CONNECT archive.example:443 HTTP/1.1"
        );
    }

    #[tokio::test]
    async fn proxy_no_proxy_bypasses_http_proxy_for_matching_host() {
        let payload = b"served directly from local mock origin".to_vec();
        let expected_md5 = md5_hex(&payload);
        let origin_server = MockServer::start(vec![MockResponse::ok(&payload)]).await;
        let proxy_server = MockServer::start(vec![MockResponse::ok(b"wrong-from-proxy")]).await;
        let proxy_url = format!("http://127.0.0.1:{}", proxy_server.addr.port());

        let config = ProxyConfig::from_lookup(|name| match name {
            "HTTP_PROXY" => Some(proxy_url.clone()),
            "NO_PROXY" => Some("127.0.0.1".to_string()),
            _ => None,
        })
        .unwrap();
        let client = http_client_builder_with_proxy(&config)
            .unwrap()
            .build()
            .unwrap();

        let temp_dir = TempTestDir::new("no-proxy-http-bypass");
        let signal = ShutdownSignal::new();
        let origin_url = origin_server.url("/direct.bin");
        let outcome = download_single_file(
            &client,
            &temp_dir.path,
            &origin_url,
            "direct.bin",
            Some(&expected_md5),
            &signal,
            None,
        )
        .await
        .expect("NO_PROXY bypassed download should succeed");

        assert_eq!(outcome, FileOutcome::Downloaded);
        let origin_requests = origin_server.finish().await;
        let proxy_requests = proxy_server.abort_and_collect().await;
        assert_eq!(origin_requests.len(), 1);
        assert_eq!(origin_requests[0].request_line, "GET /direct.bin HTTP/1.1");
        assert!(
            proxy_requests.is_empty(),
            "HTTP proxy must receive 0 requests when bypassed by NO_PROXY"
        );

        let saved = fs::read(temp_dir.join("direct.bin")).unwrap();
        assert_eq!(saved, payload);
        assert_eq!(md5_hex(&saved), expected_md5);
        assert!(!temp_dir.part_path("direct.bin").exists());
    }

    #[tokio::test]
    async fn proxy_all_proxy_socks5_tunnels_download_and_verifies_md5() {
        let payload = b"tunneled through local mock SOCKS5 proxy".to_vec();
        let expected_md5 = md5_hex(&payload);
        let socks_server = MockSocks5Server::start(vec![MockResponse::ok(&payload)]).await;
        let socks_url = socks_server.proxy_url("socks5");

        let config = ProxyConfig::from_lookup(|name| match name {
            "ALL_PROXY" => Some(socks_url.clone()),
            _ => None,
        })
        .unwrap();
        let client = http_client_builder_with_proxy(&config)
            .unwrap()
            .build()
            .unwrap();

        let temp_dir = TempTestDir::new("socks5-tunnel");
        let signal = ShutdownSignal::new();
        let outcome = download_single_file(
            &client,
            &temp_dir.path,
            "http://127.0.0.1:80/socks5.bin",
            "socks5.bin",
            Some(&expected_md5),
            &signal,
            None,
        )
        .await
        .expect("SOCKS5 tunneled download should succeed");

        assert_eq!(outcome, FileOutcome::Downloaded);
        let connections = socks_server.finish().await;
        assert_eq!(connections.len(), 1);
        assert_eq!(
            connections[0].target,
            MockSocks5Target::Ipv4([127, 0, 0, 1], 80)
        );
        assert_eq!(
            connections[0].request.request_line,
            "GET /socks5.bin HTTP/1.1"
        );

        let saved = fs::read(temp_dir.join("socks5.bin")).unwrap();
        assert_eq!(saved, payload);
        assert_eq!(md5_hex(&saved), expected_md5);
        assert!(!temp_dir.part_path("socks5.bin").exists());
    }

    #[tokio::test]
    async fn proxy_all_proxy_socks5h_sends_domain_for_remote_dns_and_verifies_md5() {
        let payload = b"tunneled through local mock SOCKS5H proxy with remote DNS".to_vec();
        let expected_md5 = md5_hex(&payload);
        let socks_server = MockSocks5Server::start(vec![MockResponse::ok(&payload)]).await;
        let socks_url = socks_server.proxy_url("socks5h");

        let config = ProxyConfig::from_lookup(|name| match name {
            "ALL_PROXY" => Some(socks_url.clone()),
            _ => None,
        })
        .unwrap();
        let client = http_client_builder_with_proxy(&config)
            .unwrap()
            .build()
            .unwrap();

        let temp_dir = TempTestDir::new("socks5h-tunnel");
        let signal = ShutdownSignal::new();
        let outcome = download_single_file(
            &client,
            &temp_dir.path,
            "http://archive.example/socks5h.bin",
            "socks5h.bin",
            Some(&expected_md5),
            &signal,
            None,
        )
        .await
        .expect("SOCKS5H tunneled download should succeed without local DNS");

        assert_eq!(outcome, FileOutcome::Downloaded);
        let connections = socks_server.finish().await;
        assert_eq!(connections.len(), 1);
        assert_eq!(
            connections[0].target,
            MockSocks5Target::Domain("archive.example".to_string(), 80)
        );
        assert_eq!(
            connections[0].request.request_line,
            "GET /socks5h.bin HTTP/1.1"
        );
        assert_eq!(
            connections[0].request.header("Host"),
            Some("archive.example")
        );

        let saved = fs::read(temp_dir.join("socks5h.bin")).unwrap();
        assert_eq!(saved, payload);
        assert_eq!(md5_hex(&saved), expected_md5);
        assert!(!temp_dir.part_path("socks5h.bin").exists());
    }

    #[tokio::test]
    async fn proxy_no_proxy_bypasses_socks5_all_proxy() {
        let payload = b"origin payload bypassing SOCKS5 proxy".to_vec();
        let expected_md5 = md5_hex(&payload);
        let origin_server = MockServer::start(vec![MockResponse::ok(&payload)]).await;
        let socks_server =
            MockSocks5Server::start(vec![MockResponse::ok(b"wrong-from-socks5")]).await;
        let socks_url = socks_server.proxy_url("socks5");

        let config = ProxyConfig::from_lookup(|name| match name {
            "ALL_PROXY" => Some(socks_url.clone()),
            "NO_PROXY" => Some("127.0.0.1".to_string()),
            _ => None,
        })
        .unwrap();
        let client = http_client_builder_with_proxy(&config)
            .unwrap()
            .build()
            .unwrap();

        let temp_dir = TempTestDir::new("no-proxy-socks5-bypass");
        let signal = ShutdownSignal::new();
        let origin_url = origin_server.url("/bypass-socks.bin");
        let outcome = download_single_file(
            &client,
            &temp_dir.path,
            &origin_url,
            "bypass-socks.bin",
            Some(&expected_md5),
            &signal,
            None,
        )
        .await
        .expect("NO_PROXY bypassed SOCKS5 download should succeed");

        assert_eq!(outcome, FileOutcome::Downloaded);
        let origin_requests = origin_server.finish().await;
        let socks_connections = socks_server.abort_and_collect().await;
        assert_eq!(origin_requests.len(), 1);
        assert_eq!(
            origin_requests[0].request_line,
            "GET /bypass-socks.bin HTTP/1.1"
        );
        assert!(
            socks_connections.is_empty(),
            "SOCKS5 proxy must receive 0 connections when bypassed by NO_PROXY"
        );

        let saved = fs::read(temp_dir.join("bypass-socks.bin")).unwrap();
        assert_eq!(saved, payload);
        assert_eq!(md5_hex(&saved), expected_md5);
        assert!(!temp_dir.part_path("bypass-socks.bin").exists());
    }

    #[derive(Clone, Default)]
    struct CapturedLogs {
        bytes: Arc<std::sync::Mutex<Vec<u8>>>,
    }

    impl CapturedLogs {
        fn new() -> Self {
            Self::default()
        }

        fn as_bytes(&self) -> Vec<u8> {
            self.bytes.lock().unwrap().clone()
        }

        fn as_string(&self) -> String {
            String::from_utf8(self.as_bytes()).expect("verbose logs must be valid UTF-8")
        }
    }

    impl std::io::Write for CapturedLogs {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.bytes.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_verbose_logging_redirect_and_cookie_redaction_with_sentinel_secret() {
        const SENTINEL_NAME: &str = "ia_sentinel_cookie_name";
        const SENTINEL_VALUE: &str = "SUPER_SECRET_SENTINEL_9f8e7d6c";

        let payload = b"redirected payload after protected hop".to_vec();
        let expected_md5 = md5_hex(&payload);
        let server = MockServer::start(vec![
            MockResponse::with_headers(
                "302 Found",
                vec![
                    ("Location", "/public/final.bin".to_string()),
                    ("Content-Length", "0".to_string()),
                ],
                b"",
            ),
            MockResponse::ok(&payload),
        ])
        .await;

        let netscape =
            format!("127.0.0.1\tFALSE\t/protected/\tFALSE\t0\t{SENTINEL_NAME}\t{SENTINEL_VALUE}\n");
        let cookie_source = CookieSource::from_netscape_str(&netscape);

        let client = http_client_builder().build().unwrap();
        let temp_dir = TempTestDir::new("verbose-redirect-cookie");
        let signal = ShutdownSignal::new();
        let options = DownloadOptions {
            retry_policy: RetryPolicy {
                max_retries: 0,
                initial_delay: Duration::from_millis(5),
                max_delay: Duration::from_millis(10),
                max_retry_after: Duration::from_millis(50),
            },
            limit_rate: None,
            verbose: true,
        };
        let captured = CapturedLogs::new();
        let logger = VerboseLogger::with_writer(captured.clone());

        let start_url = server.url("/protected/start.bin");
        let final_url = server.url("/public/final.bin");

        let outcome = download_single_file_with_options(
            &client,
            &temp_dir.path,
            &start_url,
            "final.bin",
            Some(&expected_md5),
            &signal,
            Some(&cookie_source),
            &options,
            &logger,
        )
        .await
        .expect("redirected download with scoped cookie should succeed");
        assert_eq!(outcome, FileOutcome::Downloaded);
        assert_eq!(fs::read(temp_dir.join("final.bin")).unwrap(), payload);

        let requests = server.finish().await;
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[0].request_line,
            "GET /protected/start.bin HTTP/1.1"
        );
        assert_eq!(
            requests[0].header("Cookie"),
            Some(format!("{SENTINEL_NAME}={SENTINEL_VALUE}").as_str()),
            "sentinel cookie must genuinely be transmitted on hop 1"
        );
        assert_eq!(requests[1].request_line, "GET /public/final.bin HTTP/1.1");
        assert_eq!(
            requests[1].header("Cookie"),
            None,
            "path-scoped cookie must not be sent on hop 2"
        );

        let raw_bytes = captured.as_bytes();
        assert!(
            !raw_bytes.contains(&0x1b),
            "verbose logs must contain zero ANSI escape bytes"
        );

        let logs = captured.as_string();
        assert!(
            !logs.contains(SENTINEL_NAME),
            "verbose logs must never leak cookie name:\n{logs}"
        );
        assert!(
            !logs.contains(SENTINEL_VALUE),
            "verbose logs must never leak cookie value:\n{logs}"
        );
        assert!(
            logs.contains(&format!("[verbose] http: -> GET {start_url} (cookies=yes)")),
            "missing hop 1 GET with cookies=yes:\n{logs}"
        );
        assert!(
            logs.contains(&format!(
                "[verbose] http: <- 302 Found GET {start_url} (cookies=yes)"
            )),
            "missing hop 1 302 response:\n{logs}"
        );
        assert!(
            logs.contains(&format!(
                "[verbose] redirect: 302 Found {start_url} -> {final_url} (hop 1/"
            )),
            "missing 302 redirect from -> to line:\n{logs}"
        );
        assert!(
            logs.contains(&format!("[verbose] http: -> GET {final_url} (cookies=no)")),
            "missing hop 2 GET with cookies=no:\n{logs}"
        );
        assert!(
            logs.contains(&format!(
                "[verbose] http: <- 200 OK GET {final_url} (cookies=no)"
            )),
            "missing hop 2 200 OK response:\n{logs}"
        );
    }

    #[tokio::test]
    async fn test_verbose_logging_retries_with_retry_after_and_backoff() {
        let payload = b"recovered after 429 and 503".to_vec();
        let expected_md5 = md5_hex(&payload);
        let server = MockServer::start(vec![
            MockResponse::with_headers(
                "429 Too Many Requests",
                vec![
                    ("Retry-After", "0".to_string()),
                    ("Content-Length", "0".to_string()),
                ],
                b"",
            ),
            MockResponse::with_headers(
                "503 Service Unavailable",
                vec![("Content-Length", "0".to_string())],
                b"",
            ),
            MockResponse::ok(&payload),
        ])
        .await;

        let client = http_client_builder().build().unwrap();
        let temp_dir = TempTestDir::new("verbose-retries");
        let signal = ShutdownSignal::new();
        let options = DownloadOptions {
            retry_policy: RetryPolicy {
                max_retries: 3,
                initial_delay: Duration::from_millis(5),
                max_delay: Duration::from_millis(10),
                max_retry_after: Duration::from_millis(50),
            },
            limit_rate: None,
            verbose: true,
        };
        let captured = CapturedLogs::new();
        let logger = VerboseLogger::with_writer(captured.clone());
        let url = server.url("/flaky.bin");

        let outcome = download_single_file_with_options(
            &client,
            &temp_dir.path,
            &url,
            "flaky.bin",
            Some(&expected_md5),
            &signal,
            None,
            &options,
            &logger,
        )
        .await
        .expect("download should recover after 429 and 503");
        assert_eq!(outcome, FileOutcome::Downloaded);
        assert_eq!(fs::read(temp_dir.join("flaky.bin")).unwrap(), payload);

        let requests = server.finish().await;
        assert_eq!(requests.len(), 3);

        let raw_bytes = captured.as_bytes();
        assert!(
            !raw_bytes.contains(&0x1b),
            "retry verbose logs must contain zero ANSI escape bytes"
        );

        let logs = captured.as_string();
        assert!(
            logs.contains(&format!(
                "[verbose] retry: attempt 1/3 for {url} after HTTP 429 Too Many Requests; waiting 0.00s (retry-after)"
            )),
            "missing attempt 1 retry-after line:\n{logs}"
        );

        let backoff_line = logs
            .lines()
            .find(|line| {
                line.contains(&format!(
                    "[verbose] retry: attempt 2/3 for {url} after HTTP 503 Service Unavailable; waiting "
                )) && line.ends_with("s (backoff)")
            })
            .unwrap_or_else(|| panic!("missing attempt 2 backoff line:\n{logs}"));
        assert!(
            backoff_line.contains("waiting 0."),
            "expected sub-second backoff delay in line: {backoff_line}"
        );
        assert!(
            logs.contains(&format!("[verbose] http: <- 200 OK GET {url} (cookies=no)")),
            "missing final 200 OK log line:\n{logs}"
        );
    }

    #[tokio::test]
    async fn test_verbose_logging_batch_skips_downloads_failures_and_summary() {
        let existing_payload = b"already on disk and verified".to_vec();
        let existing_md5 = md5_hex(&existing_payload);

        let fresh_payload = b"freshly fetched file".to_vec();
        let fresh_md5 = md5_hex(&fresh_payload);

        let server = MockServer::start(vec![
            MockResponse::ok(&fresh_payload),
            MockResponse::with_headers(
                "404 Not Found",
                vec![("Content-Length", "0".to_string())],
                b"",
            ),
        ])
        .await;

        let temp_dir = TempTestDir::new("verbose-batch-mixed");
        fs::write(temp_dir.join("existing.bin"), &existing_payload).unwrap();

        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();
        let options = DownloadOptions {
            retry_policy: RetryPolicy {
                max_retries: 0,
                initial_delay: Duration::from_millis(5),
                max_delay: Duration::from_millis(10),
                max_retry_after: Duration::from_millis(50),
            },
            limit_rate: None,
            verbose: true,
        };
        let captured = CapturedLogs::new();
        let logger = VerboseLogger::with_writer(captured.clone());

        let batch = vec![
            (
                server.url("/existing.bin"),
                "existing.bin".to_string(),
                Some(existing_md5),
            ),
            (
                server.url("/fresh.bin"),
                "fresh.bin".to_string(),
                Some(fresh_md5),
            ),
            (server.url("/missing.bin"), "missing.bin".to_string(), None),
        ];

        let summary = download_files_in_dir_with_logger(
            &client,
            &temp_dir.path,
            batch,
            3,
            None,
            &options,
            &signal,
            &logger,
        )
        .await
        .expect("mixed batch should return summary");

        assert_eq!(summary.ok, 1);
        assert_eq!(summary.skipped, 1);
        assert_eq!(summary.failed.len(), 1);
        assert_eq!(summary.failed[0].file_path, "missing.bin");
        assert!(!summary.interrupted);
        assert_eq!(fs::read(temp_dir.join("fresh.bin")).unwrap(), fresh_payload);

        let requests = server.finish().await;
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].request_line, "GET /fresh.bin HTTP/1.1");
        assert_eq!(requests[1].request_line, "GET /missing.bin HTTP/1.1");

        let raw_bytes = captured.as_bytes();
        assert!(
            !raw_bytes.contains(&0x1b),
            "batch verbose logs must contain zero ANSI escape bytes"
        );

        let logs = captured.as_string();
        assert!(
            logs.contains(
                "[verbose] download: skipped 'existing.bin': already exists; MD5 verified"
            ),
            "missing skip reason:\n{logs}"
        );
        assert!(
            logs.contains("[verbose] download: completed 'fresh.bin' (2/3)"),
            "missing download completion line:\n{logs}"
        );
        assert!(
            logs.lines().any(|line| {
                line.starts_with("[verbose] download: failed 'missing.bin': ")
                    && line.contains("404")
            }),
            "missing failure reason for missing.bin:\n{logs}"
        );
        assert!(
            logs.contains("[verbose] summary: ok=1 skipped=1 failed=1 interrupted=false"),
            "missing batch summary counts:\n{logs}"
        );
    }

    #[tokio::test]
    async fn test_verbose_logging_disabled_emits_zero_lines() {
        let default_options = DownloadOptions::default();
        assert!(
            !default_options.verbose,
            "DownloadOptions::default() must disable verbose logging"
        );

        let disabled_from_options = VerboseLogger::from_enabled(default_options.verbose);
        assert!(!disabled_from_options.is_enabled());

        let disabled = VerboseLogger::disabled();
        assert!(!disabled.is_enabled());

        let payload = b"silent download payload".to_vec();
        let expected_md5 = md5_hex(&payload);
        let server = MockServer::start(vec![MockResponse::ok(&payload)]).await;
        let temp_dir = TempTestDir::new("verbose-disabled");
        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();

        let summary = download_files_in_dir_with_logger(
            &client,
            &temp_dir.path,
            vec![(
                server.url("/silent.bin"),
                "silent.bin".to_string(),
                Some(expected_md5),
            )],
            1,
            None,
            &default_options,
            &signal,
            &disabled,
        )
        .await
        .expect("disabled verbose batch should succeed");

        assert_eq!(summary.ok, 1);
        assert_eq!(summary.skipped, 0);
        assert!(summary.failed.is_empty());
        assert!(!summary.interrupted);
        assert_eq!(fs::read(temp_dir.join("silent.bin")).unwrap(), payload);

        let requests = server.finish().await;
        assert_eq!(requests.len(), 1);
    }

    #[test]
    fn test_validate_download_paths_rejects_final_and_staging_collisions() {
        let dup_err = validate_download_paths([("foo?.txt", "foo_.txt"), ("foo*.txt", "foo_.txt")])
            .unwrap_err();
        assert!(
            matches!(
                dup_err,
                IaGetError::PathCollision {
                    ref local_path,
                    ref first_entry,
                    ref second_entry,
                    ..
                } if local_path == "foo_.txt"
                    && first_entry == "foo?.txt"
                    && second_entry == "foo*.txt"
            ),
            "unexpected duplicate collision error: {dup_err:?}"
        );

        let slash_err =
            validate_download_paths([("a/file.txt", "a/file.txt"), ("a//file.txt", "a//file.txt")])
                .unwrap_err();
        assert!(
            matches!(
                slash_err,
                IaGetError::PathCollision {
                    ref local_path,
                    ..
                } if local_path == "a/file.txt"
            ),
            "unexpected normalised duplicate error: {slash_err:?}"
        );

        let part_after_err =
            validate_download_paths([("foo.txt", "foo.txt"), ("foo.txt.part", "foo.txt.part")])
                .unwrap_err();
        assert!(
            matches!(
                part_after_err,
                IaGetError::PathCollision {
                    ref local_path,
                    ref first_entry,
                    ref second_entry,
                    ..
                } if local_path == "foo.txt.part"
                    && first_entry == "foo.txt"
                    && second_entry == "foo.txt.part"
            ),
            "unexpected staging collision error (foo.txt then foo.txt.part): {part_after_err:?}"
        );

        let part_before_err =
            validate_download_paths([("foo.txt.part", "foo.txt.part"), ("foo.txt", "foo.txt")])
                .unwrap_err();
        assert!(
            matches!(
                part_before_err,
                IaGetError::PathCollision {
                    ref local_path,
                    ref first_entry,
                    ref second_entry,
                    ..
                } if local_path == "foo.txt.part"
                    && first_entry == "foo.txt.part"
                    && second_entry == "foo.txt"
            ),
            "unexpected staging collision error (foo.txt.part then foo.txt): {part_before_err:?}"
        );

        assert!(validate_download_paths([
            ("foo.txt", "foo.txt"),
            ("foo.txt.bak", "foo.txt.bak"),
            ("a/foo.txt", "a/foo.txt"),
            ("b/foo.txt", "b/foo.txt"),
            ("README.md", "README.md"),
            ("readme.md", "readme.md"),
        ])
        .is_ok());
    }

    #[tokio::test]
    async fn test_download_files_in_dir_rejects_colliding_paths_before_any_http_requests() {
        let cases: &[(&str, Vec<(&str, &str)>)] = &[
            (
                "duplicate-final",
                vec![("/a", "foo_.txt"), ("/b", "foo_.txt")],
            ),
            (
                "staging-then-final",
                vec![("/a", "foo.txt"), ("/b", "foo.txt.part")],
            ),
            (
                "final-then-staging",
                vec![("/a", "foo.txt.part"), ("/b", "foo.txt")],
            ),
        ];

        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();

        for (label, entries) in cases {
            let server =
                MockServer::start(vec![MockResponse::ok(b"must never be requested")]).await;
            let sandbox = TempTestDir::new(&format!("collision-{label}"));
            let target_dir = sandbox.join("not-created-yet");

            let files: Vec<(String, String, Option<String>)> = entries
                .iter()
                .map(|(url_path, local_path)| {
                    (server.url(url_path), (*local_path).to_string(), None)
                })
                .collect();
            let total = files.len();

            let err = download_files_in_dir(
                &client,
                &target_dir,
                files,
                total,
                None,
                &RetryPolicy::default(),
                &signal,
            )
            .await
            .unwrap_err();
            let requests = server.abort_and_collect().await;

            assert!(
                matches!(err, IaGetError::PathCollision { .. }),
                "case {label}: expected PathCollision, got {err:?}"
            );
            assert!(
                requests.is_empty(),
                "case {label}: expected 0 HTTP requests on collision, got {requests:?}"
            );
            assert!(
                !target_dir.exists(),
                "case {label}: colliding batch must fail before creating output directory"
            );
        }
    }

    #[test]
    fn test_validate_download_paths_case_insensitive_rejects_case_only_final_and_staging_collisions(
    ) {
        let cases = [
            (
                "filename-case",
                ("Foo.txt", "Foo.txt"),
                ("foo.txt", "foo.txt"),
                "foo.txt",
            ),
            (
                "subdir-and-filename-case",
                ("Dir/a.txt", "Dir/a.txt"),
                ("dir/A.txt", "dir/A.txt"),
                "dir/A.txt",
            ),
            (
                "staging-then-final-upper-base",
                ("a.txt", "a.txt"),
                ("A.TXT.part", "A.TXT.part"),
                "A.TXT.part",
            ),
            (
                "staging-then-final-upper-part",
                ("a.txt", "a.txt"),
                ("a.txt.PART", "a.txt.PART"),
                "a.txt.PART",
            ),
            (
                "final-then-staging-upper-base",
                ("A.TXT.part", "A.TXT.part"),
                ("a.txt", "a.txt"),
                "a.txt.part",
            ),
            (
                "final-then-staging-upper-part",
                ("a.txt.PART", "a.txt.PART"),
                ("a.txt", "a.txt"),
                "a.txt.part",
            ),
            (
                "nested-staging-case",
                ("Dir/a.txt", "Dir/a.txt"),
                ("dir/A.TXT.PART", "dir/A.TXT.PART"),
                "dir/A.TXT.PART",
            ),
            (
                "unicode-case",
                ("Caf\u{00e9}.txt", "Caf\u{00e9}.txt"),
                ("CAF\u{00c9}.txt", "CAF\u{00c9}.txt"),
                "CAF\u{00c9}.txt",
            ),
            (
                "sanitised-case",
                ("foo?.txt", "foo_.txt"),
                ("FOO*.TXT", "FOO_.TXT"),
                "FOO_.TXT",
            ),
        ];

        for (label, first, second, expected_local) in cases {
            let err = validate_download_paths_with_case_sensitivity(
                [first, second],
                PathCaseSensitivity::CaseInsensitive,
            )
            .unwrap_err();
            assert!(
                matches!(
                    err,
                    IaGetError::PathCollision {
                        ref local_path,
                        ref first_entry,
                        ref second_entry,
                        ref reason,
                    } if local_path == expected_local
                        && first_entry == first.0
                        && second_entry == second.0
                        && reason.contains("case-insensitive")
                ),
                "case {label}: expected case-insensitive PathCollision for '{expected_local}', got {err:?}"
            );
        }

        // Exact duplicates still report the exact reason even in case-insensitive mode.
        let exact_err = validate_download_paths_with_case_sensitivity(
            [("foo?.txt", "foo_.txt"), ("foo*.txt", "foo_.txt")],
            PathCaseSensitivity::CaseInsensitive,
        )
        .unwrap_err();
        assert!(
            matches!(
                exact_err,
                IaGetError::PathCollision {
                    ref local_path,
                    ref reason,
                    ..
                } if local_path == "foo_.txt"
                    && reason == "both entries map to the same destination path"
            ),
            "expected exact collision reason, got {exact_err:?}"
        );
    }

    #[test]
    fn test_validate_download_paths_case_sensitive_allows_case_only_differences() {
        assert!(validate_download_paths_with_case_sensitivity(
            [
                ("Foo.txt", "Foo.txt"),
                ("foo.txt", "foo.txt"),
                ("Dir/a.txt", "Dir/a.txt"),
                ("dir/A.txt", "dir/A.txt"),
                ("a.txt", "a.txt"),
                ("A.TXT.part", "A.TXT.part"),
                ("a.txt.PART", "a.txt.PART"),
                ("Caf\u{00e9}.txt", "Caf\u{00e9}.txt"),
                ("CAF\u{00c9}.txt", "CAF\u{00c9}.txt"),
            ],
            PathCaseSensitivity::CaseSensitive,
        )
        .is_ok());
    }

    /// Independently observes whether `dir` is hosted on a case-sensitive or case-insensitive
    /// filesystem by creating a single file and looking up its toggled-case counterpart.
    fn measure_host_dir_case_sensitivity(dir: &Path) -> PathCaseSensitivity {
        let lower = dir.join("host-case-probe-file.txt");
        let upper = dir.join("HOST-CASE-PROBE-FILE.TXT");
        fs::write(&lower, b"probe").expect("write host case probe");
        let sensitive = match fs::symlink_metadata(&upper) {
            Ok(_) => PathCaseSensitivity::CaseInsensitive,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                PathCaseSensitivity::CaseSensitive
            }
            Err(err) => panic!("unexpected error probing host case sensitivity: {err}"),
        };
        let _ = fs::remove_file(&lower);
        sensitive
    }

    #[test]
    fn test_detect_path_case_sensitivity_and_validate_for_dir_match_host_filesystem() {
        let oracle_dir = TempTestDir::new("case-oracle");
        let expected = measure_host_dir_case_sensitivity(&oracle_dir.path);

        // 1. Empty existing directory (exercises temp probe file + cleanup).
        let empty_dir = TempTestDir::new("case-empty");
        assert_eq!(detect_path_case_sensitivity(&empty_dir.path), expected);
        assert_eq!(
            fs::read_dir(&empty_dir.path).unwrap().count(),
            0,
            "temporary case probe file must be cleaned up"
        );

        // 2. Non-empty existing directory (exercises read-only child probe).
        let populated_dir = TempTestDir::new("case-populated");
        fs::write(populated_dir.join("Existing.txt"), b"data").unwrap();
        assert_eq!(detect_path_case_sensitivity(&populated_dir.path), expected);
        assert_eq!(
            fs::read_dir(&populated_dir.path).unwrap().count(),
            1,
            "read-only probe must not leave extra entries"
        );

        // 3. Not-yet-created nested directory resolves nearest existing ancestor without creating it.
        let missing_dir = empty_dir.join("not-created-yet/nested");
        assert_eq!(detect_path_case_sensitivity(&missing_dir), expected);
        assert!(
            !empty_dir.join("not-created-yet").exists(),
            "case sensitivity detection must not create missing output directories"
        );

        // 4. Read-only directory with an existing child still probes cleanly on Unix.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let ro_dir = TempTestDir::new("case-readonly");
            fs::write(ro_dir.join("Sample.txt"), b"ro").unwrap();
            fs::set_permissions(&ro_dir.path, fs::Permissions::from_mode(0o555)).unwrap();
            let detected = detect_path_case_sensitivity(&ro_dir.path);
            fs::set_permissions(&ro_dir.path, fs::Permissions::from_mode(0o755)).unwrap();
            assert_eq!(detected, expected);
        }

        // 5. validate_download_paths_for_dir matches host sensitivity for case-only pairs.
        let case_only_pairs = [
            [("Foo.txt", "Foo.txt"), ("foo.txt", "foo.txt")],
            [("a.txt", "a.txt"), ("A.TXT.part", "A.TXT.part")],
        ];
        for pair in case_only_pairs {
            let result = validate_download_paths_for_dir(&missing_dir, pair);
            match expected {
                PathCaseSensitivity::CaseSensitive => assert!(
                    result.is_ok(),
                    "case-sensitive host should allow {pair:?}, got {result:?}"
                ),
                PathCaseSensitivity::CaseInsensitive => assert!(
                    matches!(result, Err(IaGetError::PathCollision { .. })),
                    "case-insensitive host should reject {pair:?}, got {result:?}"
                ),
            }
        }
        assert!(
            !empty_dir.join("not-created-yet").exists(),
            "validate_download_paths_for_dir must not create missing directories"
        );
    }

    #[tokio::test]
    async fn test_download_files_in_dir_case_only_entries_follow_host_filesystem() {
        let sandbox = TempTestDir::new("case-download-host");
        let expected = measure_host_dir_case_sensitivity(&sandbox.path);
        let target_dir = sandbox.join("not-created-yet");

        let payload_upper = b"upper-file-contents";
        let payload_lower = b"lower-file-contents";
        let server = MockServer::start(vec![
            MockResponse::ok(payload_upper),
            MockResponse::ok(payload_lower),
        ])
        .await;

        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();
        let files = vec![
            (
                server.url("/upper"),
                "Foo.txt".to_string(),
                Some(format!("{:x}", md5::compute(payload_upper))),
            ),
            (
                server.url("/lower"),
                "foo.txt".to_string(),
                Some(format!("{:x}", md5::compute(payload_lower))),
            ),
        ];

        let result = download_files_in_dir(
            &client,
            &target_dir,
            files,
            2,
            None,
            &RetryPolicy::default(),
            &signal,
        )
        .await;

        match expected {
            PathCaseSensitivity::CaseInsensitive => {
                let err = result.unwrap_err();
                assert!(
                    matches!(err, IaGetError::PathCollision { .. }),
                    "expected PathCollision on case-insensitive volume, got {err:?}"
                );
                let requests = server.abort_and_collect().await;
                assert!(
                    requests.is_empty(),
                    "expected 0 HTTP requests on case collision, got {requests:?}"
                );
                assert!(
                    !target_dir.exists(),
                    "case collision must abort before creating output directory"
                );
            }
            PathCaseSensitivity::CaseSensitive => {
                let summary = result.expect("case-sensitive volume should download both files");
                assert_eq!(summary.ok, 2);
                assert!(summary.failed.is_empty());
                assert_eq!(fs::read(target_dir.join("Foo.txt")).unwrap(), payload_upper);
                assert_eq!(fs::read(target_dir.join("foo.txt")).unwrap(), payload_lower);
                let requests = server.finish().await;
                assert_eq!(requests.len(), 2);
            }
        }
    }

    /// Lookup rule simulated by [`MockDirectoryLookup`] for deterministic per-directory tests.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum MockDirRule {
        /// Case-sensitive and Unicode-normalisation-sensitive (for example Linux ext4).
        Exact,
        /// ASCII case-insensitive (for example a FAT32/SMB mount).
        CaseInsensitiveAscii,
        /// Case-sensitive, Unicode NFC/NFD normalisation-insensitive (for example case-sensitive
        /// APFS/HFS+).
        NormalizationInsensitive,
        /// Both Unicode case-folding (including `ß` -> `ss`) and NFC/NFD normalisation-insensitive
        /// (for example default APFS or ext4 `+F`).
        CaseAndNormalizationInsensitive,
        /// Simulates an inconclusive filesystem probe that cannot verify distinctness and must
        /// fail closed.
        InconclusiveProbe,
    }

    impl MockDirRule {
        fn canonical_key(self, name: &str) -> String {
            let normalize_nfc = |s: &str| {
                s.replace("e\u{0301}", "\u{00e9}")
                    .replace("E\u{0301}", "\u{00c9}")
                    .replace("a\u{030a}", "\u{00e5}")
                    .replace("A\u{030a}", "\u{00c5}")
            };

            match self {
                MockDirRule::Exact | MockDirRule::InconclusiveProbe => name.to_string(),
                MockDirRule::CaseInsensitiveAscii => name.to_ascii_lowercase(),
                MockDirRule::NormalizationInsensitive => normalize_nfc(name),
                MockDirRule::CaseAndNormalizationInsensitive => {
                    normalize_nfc(name).replace('\u{00df}', "ss").to_lowercase()
                }
            }
        }
    }

    /// Deterministic in-memory [`DirectoryLookup`] simulating mixed-mount directory trees and
    /// recording every probed directory path.
    struct MockDirectoryLookup {
        dirs: HashMap<PathBuf, MockDirRule>,
        probed_dirs: std::cell::RefCell<Vec<PathBuf>>,
    }

    impl MockDirectoryLookup {
        fn new(entries: impl IntoIterator<Item = (&'static str, MockDirRule)>) -> Self {
            Self {
                dirs: entries
                    .into_iter()
                    .map(|(path, rule)| (PathBuf::from(path), rule))
                    .collect(),
                probed_dirs: std::cell::RefCell::new(Vec::new()),
            }
        }

        fn probed(&self) -> Vec<PathBuf> {
            self.probed_dirs.borrow().clone()
        }

        fn clear_probed(&self) {
            self.probed_dirs.borrow_mut().clear();
        }
    }

    impl DirectoryLookup for MockDirectoryLookup {
        fn resolve_root(&self, dir: &Path) -> ProbeRoot {
            if self.dirs.contains_key(dir) {
                return ProbeRoot {
                    probe_dir: Some(dir.to_path_buf()),
                    exact_on_disk_dir: Some(dir.to_path_buf()),
                };
            }

            let ancestor = dir
                .ancestors()
                .find(|candidate| self.dirs.contains_key(*candidate))
                .map(Path::to_path_buf);
            ProbeRoot {
                probe_dir: ancestor,
                exact_on_disk_dir: None,
            }
        }

        fn resolve_child_dir(
            &self,
            _canonical_root: Option<&Path>,
            parent_on_disk: &Path,
            segment: &str,
        ) -> Option<PathBuf> {
            let exact = parent_on_disk.join(segment);
            if self.dirs.contains_key(&exact) {
                return Some(exact);
            }

            let parent_rule = self
                .dirs
                .get(parent_on_disk)
                .copied()
                .unwrap_or(MockDirRule::Exact);
            let target_key = parent_rule.canonical_key(segment);
            self.dirs.keys().find_map(|candidate| {
                if candidate.parent() == Some(parent_on_disk) {
                    let child_name = candidate.file_name()?.to_str()?;
                    if parent_rule.canonical_key(child_name) == target_key {
                        return Some(candidate.clone());
                    }
                }
                None
            })
        }

        fn resolve_equivalences(
            &self,
            probe_dir: Option<&Path>,
            names: &[String],
            candidates: &[usize],
        ) -> Result<DirEquivalenceResult> {
            let dir = probe_dir.expect("mock lookup expects a resolved probe_dir");
            self.probed_dirs.borrow_mut().push(dir.to_path_buf());
            let rule = self.dirs.get(dir).copied().unwrap_or(MockDirRule::Exact);
            if rule == MockDirRule::InconclusiveProbe {
                return Err(IaGetError::FileSystem(unverified_probe_error(Some(dir))));
            }

            let mut groups: Vec<usize> = (0..names.len()).collect();
            let mut seen: HashMap<String, usize> = HashMap::new();
            for &idx in candidates {
                let key = rule.canonical_key(&names[idx]);
                let group = *seen.entry(key).or_insert(idx);
                groups[idx] = group;
            }

            Ok(DirEquivalenceResult {
                groups,
                reason_suffix: " under the destination directory's filesystem lookup rules",
                unverified_error: None,
            })
        }
    }

    #[test]
    fn test_per_parent_lookup_case_insensitive_subdir_under_case_sensitive_root() {
        let lookup = MockDirectoryLookup::new([
            ("/root", MockDirRule::Exact),
            ("/root/sub", MockDirRule::CaseInsensitiveAscii),
        ]);
        let root = Path::new("/root");

        // Case-only difference in the case-sensitive root is allowed.
        assert!(validate_download_paths_with_lookup(
            root,
            &[("Foo.txt", "Foo.txt"), ("foo.txt", "foo.txt")],
            &lookup,
        )
        .is_ok());
        assert_eq!(lookup.probed(), vec![PathBuf::from("/root")]);

        // Case-only collisions inside the case-insensitive sub-mount are rejected.
        let sub_cases = [
            (
                "final-vs-final",
                ("sub/Foo.txt", "sub/Foo.txt"),
                ("sub/foo.txt", "sub/foo.txt"),
                "sub/foo.txt",
            ),
            (
                "staging-vs-final",
                ("sub/a.txt", "sub/a.txt"),
                ("sub/A.TXT.part", "sub/A.TXT.part"),
                "sub/A.TXT.part",
            ),
            (
                "final-vs-staging",
                ("sub/A.TXT.part", "sub/A.TXT.part"),
                ("sub/a.txt", "sub/a.txt"),
                "sub/a.txt.part",
            ),
            (
                "missing-nested-under-submount",
                ("sub/newdir/Foo.txt", "sub/newdir/Foo.txt"),
                ("sub/newdir/foo.txt", "sub/newdir/foo.txt"),
                "sub/newdir/foo.txt",
            ),
        ];

        for (label, first, second, expected_local) in sub_cases {
            lookup.clear_probed();
            let err =
                validate_download_paths_with_lookup(root, &[first, second], &lookup).unwrap_err();
            assert!(
                matches!(
                    err,
                    IaGetError::PathCollision {
                        ref local_path,
                        ref first_entry,
                        ref second_entry,
                        ref reason,
                    } if local_path == expected_local
                        && first_entry == first.0
                        && second_entry == second.0
                        && reason.contains("filesystem lookup rules")
                ),
                "case {label}: expected PathCollision for '{expected_local}', got {err:?}"
            );
            assert_eq!(
                lookup.probed(),
                vec![PathBuf::from("/root/sub")],
                "case {label}: expected probe inside /root/sub"
            );
        }
    }

    #[test]
    fn test_per_parent_lookup_case_sensitive_subdir_under_case_insensitive_root() {
        let lookup = MockDirectoryLookup::new([
            ("/root", MockDirRule::CaseInsensitiveAscii),
            ("/root/sub", MockDirRule::Exact),
        ]);
        let root = Path::new("/root");

        // Case-only collision in the case-insensitive root is rejected.
        let root_err = validate_download_paths_with_lookup(
            root,
            &[("Foo.txt", "Foo.txt"), ("foo.txt", "foo.txt")],
            &lookup,
        )
        .unwrap_err();
        assert!(matches!(
            root_err,
            IaGetError::PathCollision {
                ref local_path,
                ..
            } if local_path == "foo.txt"
        ));

        // Case-only difference inside the case-sensitive subdirectory is allowed.
        lookup.clear_probed();
        assert!(validate_download_paths_with_lookup(
            root,
            &[
                ("sub/Foo.txt", "sub/Foo.txt"),
                ("sub/foo.txt", "sub/foo.txt")
            ],
            &lookup,
        )
        .is_ok());
        assert_eq!(lookup.probed(), vec![PathBuf::from("/root/sub")]);

        // Equivalent parent spellings (`Sub` and `sub`) merge onto the existing case-sensitive
        // `/root/sub`, where `Foo.txt` and `foo.txt` remain distinct.
        lookup.clear_probed();
        assert!(validate_download_paths_with_lookup(
            root,
            &[
                ("Sub/Foo.txt", "Sub/Foo.txt"),
                ("sub/foo.txt", "sub/foo.txt")
            ],
            &lookup,
        )
        .is_ok());
        assert_eq!(
            lookup.probed(),
            vec![PathBuf::from("/root"), PathBuf::from("/root/sub")]
        );

        // Merging `Sub` and `sub` with an identical leaf filename still collides!
        let merged_leaf_err = validate_download_paths_with_lookup(
            root,
            &[
                ("Sub/Foo.txt", "Sub/Foo.txt"),
                ("sub/Foo.txt", "sub/Foo.txt"),
            ],
            &lookup,
        )
        .unwrap_err();
        assert!(
            matches!(
                merged_leaf_err,
                IaGetError::PathCollision {
                    ref local_path,
                    ref first_entry,
                    ref second_entry,
                    ref reason,
                } if local_path == "sub/Foo.txt"
                    && first_entry == "Sub/Foo.txt"
                    && second_entry == "sub/Foo.txt"
                    && reason.contains("filesystem lookup rules")
            ),
            "unexpected merged-parent collision error: {merged_leaf_err:?}"
        );
    }

    #[test]
    fn test_per_parent_lookup_unicode_nfc_nfd_and_folding_equivalence() {
        let norm_only = MockDirectoryLookup::new([
            ("/apfs-cs", MockDirRule::NormalizationInsensitive),
            ("/ext4", MockDirRule::Exact),
            ("/apfs-ci", MockDirRule::CaseAndNormalizationInsensitive),
        ]);

        // Case-sensitive but normalisation-insensitive directory allows ASCII case differences.
        assert!(validate_download_paths_with_lookup(
            Path::new("/apfs-cs"),
            &[("Foo.txt", "Foo.txt"), ("foo.txt", "foo.txt")],
            &norm_only,
        )
        .is_ok());

        // NFC vs NFD collisions (final, staging in both orders, and colliding parent dirs).
        let nfc_nfd_collisions = [
            (
                "nfc-vs-nfd-final",
                ("Caf\u{00e9}.txt", "Caf\u{00e9}.txt"),
                ("Cafe\u{0301}.txt", "Cafe\u{0301}.txt"),
                "Cafe\u{0301}.txt",
            ),
            (
                "nfc-vs-nfd-staging-after",
                ("Caf\u{00e9}.txt", "Caf\u{00e9}.txt"),
                ("Cafe\u{0301}.txt.part", "Cafe\u{0301}.txt.part"),
                "Cafe\u{0301}.txt.part",
            ),
            (
                "nfd-staging-before-nfc",
                ("Cafe\u{0301}.txt.part", "Cafe\u{0301}.txt.part"),
                ("Caf\u{00e9}.txt", "Caf\u{00e9}.txt"),
                "Caf\u{00e9}.txt.part",
            ),
            (
                "nfc-vs-nfd-parent-dir",
                ("Dir_Caf\u{00e9}/a.txt", "Dir_Caf\u{00e9}/a.txt"),
                ("Dir_Cafe\u{0301}/a.txt", "Dir_Cafe\u{0301}/a.txt"),
                "Dir_Cafe\u{0301}/a.txt",
            ),
        ];

        for (label, first, second, expected_local) in nfc_nfd_collisions {
            let err = validate_download_paths_with_lookup(
                Path::new("/apfs-cs"),
                &[first, second],
                &norm_only,
            )
            .unwrap_err();
            assert!(
                matches!(
                    err,
                    IaGetError::PathCollision {
                        ref local_path,
                        ref first_entry,
                        ref second_entry,
                        ref reason,
                    } if local_path == expected_local
                        && first_entry == first.0
                        && second_entry == second.0
                        && reason.contains("filesystem lookup rules")
                ),
                "case {label}: expected NFC/NFD PathCollision for '{expected_local}', got {err:?}"
            );

            // On a normalisation-sensitive filesystem, the exact same pair is allowed.
            assert!(
                validate_download_paths_with_lookup(
                    Path::new("/ext4"),
                    &[first, second],
                    &norm_only,
                )
                .is_ok(),
                "case {label}: normalisation-sensitive directory should allow {first:?} and {second:?}"
            );
        }

        // Equivalent NFC/NFD parent directories with distinct child filenames are allowed.
        assert!(validate_download_paths_with_lookup(
            Path::new("/apfs-cs"),
            &[
                ("Dir_Caf\u{00e9}/a.txt", "Dir_Caf\u{00e9}/a.txt"),
                ("Dir_Cafe\u{0301}/b.txt", "Dir_Cafe\u{0301}/b.txt"),
            ],
            &norm_only,
        )
        .is_ok());

        // Non-ASCII entry colliding with a pure-ASCII entry under Unicode full case folding.
        let folding_err = validate_download_paths_with_lookup(
            Path::new("/apfs-ci"),
            &[
                ("Stra\u{00df}e.txt", "Stra\u{00df}e.txt"),
                ("STRASSE.TXT", "STRASSE.TXT"),
            ],
            &norm_only,
        )
        .unwrap_err();
        assert!(matches!(
            folding_err,
            IaGetError::PathCollision {
                ref local_path,
                ..
            } if local_path == "STRASSE.TXT"
        ));
    }

    #[test]
    fn test_per_parent_lookup_merges_colliding_parent_subdirectories() {
        let lookup = MockDirectoryLookup::new([("/ci", MockDirRule::CaseInsensitiveAscii)]);
        let root = Path::new("/ci");

        let cases = [
            (
                "dir-case-and-file-case",
                ("Dir/a.txt", "Dir/a.txt"),
                ("dir/A.txt", "dir/A.txt"),
                "dir/A.txt",
            ),
            (
                "dir-case-and-staging-upper-part",
                ("Dir/a.txt", "Dir/a.txt"),
                ("dir/a.txt.PART", "dir/a.txt.PART"),
                "dir/a.txt.PART",
            ),
            (
                "dir-case-and-staging-before-final",
                ("Dir/a.txt.PART", "Dir/a.txt.PART"),
                ("dir/a.txt", "dir/a.txt"),
                "dir/a.txt.part",
            ),
        ];

        for (label, first, second, expected_local) in cases {
            let err =
                validate_download_paths_with_lookup(root, &[first, second], &lookup).unwrap_err();
            assert!(
                matches!(
                    err,
                    IaGetError::PathCollision {
                        ref local_path,
                        ref first_entry,
                        ref second_entry,
                        ..
                    } if local_path == expected_local
                        && first_entry == first.0
                        && second_entry == second.0
                ),
                "case {label}: expected PathCollision for '{expected_local}', got {err:?}"
            );
        }

        // Distinct files under merged case-equivalent parent directories are allowed.
        assert!(validate_download_paths_with_lookup(
            root,
            &[("Dir/a.txt", "Dir/a.txt"), ("dir/b.txt", "dir/b.txt")],
            &lookup,
        )
        .is_ok());
    }

    #[test]
    fn test_per_parent_lookup_file_vs_directory_collisions() {
        let lookup = MockDirectoryLookup::new([
            ("/exact", MockDirRule::Exact),
            ("/ci", MockDirRule::CaseInsensitiveAscii),
        ]);

        // Exact file-vs-directory and staging-vs-directory collisions fail with zero probing.
        let exact_cases = [
            (("foo", "foo"), ("foo/bar.txt", "foo/bar.txt"), "foo"),
            (("foo/bar.txt", "foo/bar.txt"), ("foo", "foo"), "foo"),
            (
                ("foo", "foo"),
                ("foo.part/bar.txt", "foo.part/bar.txt"),
                "foo.part",
            ),
            (
                ("foo.part/bar.txt", "foo.part/bar.txt"),
                ("foo", "foo"),
                "foo.part",
            ),
        ];
        for (first, second, expected_local) in exact_cases {
            lookup.clear_probed();
            let err =
                validate_download_paths_with_lookup(Path::new("/exact"), &[first, second], &lookup)
                    .unwrap_err();
            assert!(
                matches!(
                    err,
                    IaGetError::PathCollision {
                        ref local_path,
                        ref first_entry,
                        ref second_entry,
                        ..
                    } if local_path == expected_local
                        && first_entry == first.0
                        && second_entry == second.0
                ),
                "expected exact file-vs-dir PathCollision for '{expected_local}', got {err:?}"
            );
            assert!(
                lookup.probed().is_empty(),
                "exact file-vs-dir collision must not require filesystem probing"
            );
        }

        // Case-only file-vs-directory collision is allowed on exact dir and rejected on CI dir.
        assert!(validate_download_paths_with_lookup(
            Path::new("/exact"),
            &[("foo", "foo"), ("FOO/bar.txt", "FOO/bar.txt")],
            &lookup,
        )
        .is_ok());

        let ci_cases = [
            (("foo", "foo"), ("FOO/bar.txt", "FOO/bar.txt"), "FOO"),
            (("FOO/bar.txt", "FOO/bar.txt"), ("foo", "foo"), "foo"),
            (
                ("foo", "foo"),
                ("FOO.PART/bar.txt", "FOO.PART/bar.txt"),
                "FOO.PART",
            ),
        ];
        for (first, second, expected_local) in ci_cases {
            let err =
                validate_download_paths_with_lookup(Path::new("/ci"), &[first, second], &lookup)
                    .unwrap_err();
            assert!(
                matches!(
                    err,
                    IaGetError::PathCollision {
                        ref local_path,
                        ref first_entry,
                        ref second_entry,
                        ..
                    } if local_path == expected_local
                        && first_entry == first.0
                        && second_entry == second.0
                ),
                "expected CI file-vs-dir PathCollision for '{expected_local}', got {err:?}"
            );
        }
    }

    #[test]
    fn test_per_parent_lookup_zero_io_fast_path_and_missing_root_isolation() {
        let lookup = MockDirectoryLookup::new([
            ("/ancestor", MockDirRule::Exact),
            ("/ancestor/sub", MockDirRule::CaseInsensitiveAscii),
        ]);

        // Pure-ASCII batch with no ASCII case duplicates performs zero probing.
        assert!(validate_download_paths_with_lookup(
            Path::new("/ancestor"),
            &[
                ("a.txt", "a.txt"),
                ("b.txt", "b.txt"),
                ("sub/c.txt", "sub/c.txt"),
                ("sub/d.txt", "sub/d.txt"),
            ],
            &lookup,
        )
        .is_ok());
        assert!(lookup.probed().is_empty());

        // Single non-ASCII entry performs zero probing.
        assert!(validate_download_paths_with_lookup(
            Path::new("/ancestor"),
            &[("Caf\u{00e9}.txt", "Caf\u{00e9}.txt")],
            &lookup,
        )
        .is_ok());
        assert!(lookup.probed().is_empty());

        // When the output root does not exist yet (`/ancestor/missing`), planned child `sub` must
        // NOT resolve to the unrelated sibling `/ancestor/sub`; it must inherit `/ancestor`.
        assert!(validate_download_paths_with_lookup(
            Path::new("/ancestor/missing"),
            &[
                ("sub/Foo.txt", "sub/Foo.txt"),
                ("sub/foo.txt", "sub/foo.txt")
            ],
            &lookup,
        )
        .is_ok());
        assert_eq!(lookup.probed(), vec![PathBuf::from("/ancestor")]);
    }

    /// Independently observes whether `dir` treats NFC (`Caf\u{00e9}`) and NFD (`Cafe\u{0301}`)
    /// filenames as equivalent.
    fn measure_host_dir_nfc_nfd_equivalent(dir: &Path) -> bool {
        let nfc = dir.join("host-nfc-probe-caf\u{00e9}.txt");
        let nfd = dir.join("host-nfc-probe-cafe\u{0301}.txt");
        fs::write(&nfc, b"probe").expect("write host NFC probe");
        let equivalent = match fs::symlink_metadata(&nfd) {
            Ok(_) => true,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => false,
            Err(err) => panic!("unexpected error probing host NFC/NFD equivalence: {err}"),
        };
        let _ = fs::remove_file(&nfc);
        equivalent
    }

    /// Asserts that `dir` contains no leftover `.ia-get-fs-probe-*` entries.
    fn assert_no_probe_dirs_remaining(dir: &Path) {
        for entry in fs::read_dir(dir).expect("read directory for probe cleanup check") {
            let name = entry.expect("dir entry").file_name();
            let name_str = name.to_string_lossy();
            assert!(
                !name_str.starts_with(".ia-get-fs-probe-"),
                "leftover temporary probe directory found in {}: {}",
                dir.display(),
                name_str
            );
        }
    }

    #[test]
    fn test_real_host_probe_unicode_cleanup_symlink_safety_and_readonly_fallback() {
        let sandbox = TempTestDir::new("real-host-per-parent-probe");
        let host_case = measure_host_dir_case_sensitivity(&sandbox.path);
        let host_nfc_nfd = measure_host_dir_nfc_nfd_equivalent(&sandbox.path);

        // 1. NFC vs NFD final and `.part` pairs match the real host filesystem and leave no probe
        // directories behind.
        let nfc_pairs = [
            [
                ("Caf\u{00e9}.txt", "Caf\u{00e9}.txt"),
                ("Cafe\u{0301}.txt", "Cafe\u{0301}.txt"),
            ],
            [
                ("Caf\u{00e9}.txt", "Caf\u{00e9}.txt"),
                ("Cafe\u{0301}.txt.part", "Cafe\u{0301}.txt.part"),
            ],
        ];
        for pair in nfc_pairs {
            let result = validate_download_paths_for_dir(&sandbox.path, pair);
            if host_nfc_nfd {
                assert!(
                    matches!(result, Err(IaGetError::PathCollision { .. })),
                    "normalisation-insensitive host should reject {pair:?}, got {result:?}"
                );
            } else {
                assert!(
                    result.is_ok(),
                    "normalisation-sensitive host should allow {pair:?}, got {result:?}"
                );
            }
            assert_no_probe_dirs_remaining(&sandbox.path);
        }

        // 2. Missing output dir is never created and leaves no probe dirs in its existing parent.
        let missing_dir = sandbox.join("never-created/nested");
        let case_res = validate_download_paths_for_dir(
            &missing_dir,
            [("Foo.txt", "Foo.txt"), ("foo.txt", "foo.txt")],
        );
        match host_case {
            PathCaseSensitivity::CaseSensitive => assert!(case_res.is_ok()),
            PathCaseSensitivity::CaseInsensitive => {
                assert!(matches!(case_res, Err(IaGetError::PathCollision { .. })));
            }
        }
        assert!(
            !sandbox.join("never-created").exists(),
            "validate_download_paths_for_dir must not create missing directories"
        );
        assert_no_probe_dirs_remaining(&sandbox.path);

        // 3. Overlong filenames trigger `create_new` error -> clean probe dir removal -> fail closed.
        let long_lower = "a".repeat(300);
        let long_upper = "A".repeat(300);
        let long_res = validate_download_paths_for_dir(
            &sandbox.path,
            [
                (long_lower.as_str(), long_lower.as_str()),
                (long_upper.as_str(), long_upper.as_str()),
            ],
        );
        assert!(
            matches!(long_res, Err(IaGetError::FileSystem(_))),
            "overlong unprobable names in empty dir must fail closed, got {long_res:?}"
        );
        assert_no_probe_dirs_remaining(&sandbox.path);

        // 4. Symlinks inside output_dir are never followed for probing, and read-only directories
        // preserve the zero-I/O fast path while failing closed when candidates cannot be verified.
        #[cfg(unix)]
        {
            use std::os::unix::fs::{symlink, PermissionsExt};

            let external_target = TempTestDir::new("symlink-probe-external");
            fs::set_permissions(&external_target.path, fs::Permissions::from_mode(0o555)).unwrap();
            let link_path = sandbox.join("linked_sub");
            symlink(&external_target.path, &link_path).unwrap();

            let link_res = validate_download_paths_for_dir(
                &sandbox.path,
                [
                    ("linked_sub/Foo.txt", "linked_sub/Foo.txt"),
                    ("linked_sub/foo.txt", "linked_sub/foo.txt"),
                ],
            );
            fs::set_permissions(&external_target.path, fs::Permissions::from_mode(0o755)).unwrap();
            match host_case {
                PathCaseSensitivity::CaseSensitive => assert!(link_res.is_ok()),
                PathCaseSensitivity::CaseInsensitive => {
                    assert!(matches!(link_res, Err(IaGetError::PathCollision { .. })));
                }
            }
            assert_eq!(
                fs::read_dir(&external_target.path).unwrap().count(),
                0,
                "symlink target outside output_dir must never be written to during probing"
            );
            assert_no_probe_dirs_remaining(&sandbox.path);

            // Populated read-only directory:
            // - Non-colliding pure-ASCII entries succeed via zero-I/O fast path.
            // - Plain-ASCII case-only pair resolves via read-only child probe.
            // - Unicode NFC/NFD pair and trailing-dot ASCII pair fail closed.
            let ro_populated = TempTestDir::new("ro-probe-populated");
            fs::write(ro_populated.join("Sample.txt"), b"ro").unwrap();
            fs::set_permissions(&ro_populated.path, fs::Permissions::from_mode(0o555)).unwrap();
            let ro_pop_ascii_distinct = validate_download_paths_for_dir(
                &ro_populated.path,
                [("alpha.txt", "alpha.txt"), ("beta.txt", "beta.txt")],
            );
            let ro_pop_case_res = validate_download_paths_for_dir(
                &ro_populated.path,
                [("Foo.txt", "Foo.txt"), ("foo.txt", "foo.txt")],
            );
            let ro_pop_unicode_res = validate_download_paths_for_dir(
                &ro_populated.path,
                [
                    ("Caf\u{00e9}.txt", "Caf\u{00e9}.txt"),
                    ("Cafe\u{0301}.txt", "Cafe\u{0301}.txt"),
                ],
            );
            let ro_pop_trailing_dot_res = validate_download_paths_for_dir(
                &ro_populated.path,
                [("foo.", "foo."), ("foo", "foo")],
            );
            fs::set_permissions(&ro_populated.path, fs::Permissions::from_mode(0o755)).unwrap();

            assert!(
                ro_pop_ascii_distinct.is_ok(),
                "non-colliding pure-ASCII entries must succeed with zero I/O in read-only dir: {ro_pop_ascii_distinct:?}"
            );
            match host_case {
                PathCaseSensitivity::CaseSensitive => assert!(
                    ro_pop_case_res.is_ok(),
                    "populated read-only CS dir should allow Foo.txt and foo.txt via child probe, got {ro_pop_case_res:?}"
                ),
                PathCaseSensitivity::CaseInsensitive => {
                    assert!(
                        matches!(ro_pop_case_res, Err(IaGetError::PathCollision { .. })),
                        "populated read-only CI dir should reject Foo.txt and foo.txt, got {ro_pop_case_res:?}"
                    );
                }
            }
            assert!(
                matches!(ro_pop_unicode_res, Err(IaGetError::FileSystem(_))),
                "read-only populated dir with Unicode candidates must fail closed, got {ro_pop_unicode_res:?}"
            );
            assert!(
                matches!(ro_pop_trailing_dot_res, Err(IaGetError::FileSystem(_))),
                "read-only populated dir with trailing-dot ASCII candidates must fail closed, got {ro_pop_trailing_dot_res:?}"
            );
            assert_no_probe_dirs_remaining(&ro_populated.path);

            // Empty read-only directory:
            // - Non-colliding pure-ASCII entries succeed via zero-I/O fast path.
            // - ASCII case-only and Unicode candidates fail closed (no child probe possible).
            let ro_empty = TempTestDir::new("ro-probe-empty");
            fs::set_permissions(&ro_empty.path, fs::Permissions::from_mode(0o555)).unwrap();
            let ro_empty_ascii_distinct = validate_download_paths_for_dir(
                &ro_empty.path,
                [("alpha.txt", "alpha.txt"), ("beta.txt", "beta.txt")],
            );
            let ro_empty_case_res = validate_download_paths_for_dir(
                &ro_empty.path,
                [("Foo.txt", "Foo.txt"), ("foo.txt", "foo.txt")],
            );
            let ro_empty_unicode_res = validate_download_paths_for_dir(
                &ro_empty.path,
                [
                    ("Caf\u{00e9}.txt", "Caf\u{00e9}.txt"),
                    ("Cafe\u{0301}.txt", "Cafe\u{0301}.txt"),
                ],
            );
            fs::set_permissions(&ro_empty.path, fs::Permissions::from_mode(0o755)).unwrap();

            assert!(
                ro_empty_ascii_distinct.is_ok(),
                "non-colliding pure-ASCII entries must succeed with zero I/O in empty read-only dir: {ro_empty_ascii_distinct:?}"
            );
            assert!(
                matches!(ro_empty_case_res, Err(IaGetError::FileSystem(_))),
                "empty read-only dir with ASCII case-only candidates must fail closed, got {ro_empty_case_res:?}"
            );
            assert!(
                matches!(ro_empty_unicode_res, Err(IaGetError::FileSystem(_))),
                "empty read-only dir with Unicode candidates must fail closed, got {ro_empty_unicode_res:?}"
            );
            assert_no_probe_dirs_remaining(&ro_empty.path);
        }
    }

    #[test]
    fn test_prefixed_file_probe_fallback_when_temp_subdir_unavailable() {
        let sandbox = TempTestDir::new("prefixed-probe-fallback");
        let host_case = measure_host_dir_case_sensitivity(&sandbox.path);
        let host_nfc_nfd = measure_host_dir_nfc_nfd_equivalent(&sandbox.path);
        let no_subdir_lookup = OsDirectoryLookup::without_temp_subdir_probe();

        // 1. ASCII case-only pair matches host semantics when temp subdir creation is unavailable.
        let case_res = validate_download_paths_with_lookup(
            &sandbox.path,
            &[("Foo.txt", "Foo.txt"), ("foo.txt", "foo.txt")],
            &no_subdir_lookup,
        );
        match host_case {
            PathCaseSensitivity::CaseSensitive => assert!(
                case_res.is_ok(),
                "prefixed probe on CS host should allow Foo.txt and foo.txt, got {case_res:?}"
            ),
            PathCaseSensitivity::CaseInsensitive => assert!(
                matches!(case_res, Err(IaGetError::PathCollision { .. })),
                "prefixed probe on CI host should reject Foo.txt and foo.txt, got {case_res:?}"
            ),
        }
        assert_no_probe_dirs_remaining(&sandbox.path);

        // 2. Unicode NFC vs NFD final and `.part` pairs match host semantics via prefixed files.
        let nfc_pairs = [
            [
                ("Caf\u{00e9}.txt", "Caf\u{00e9}.txt"),
                ("Cafe\u{0301}.txt", "Cafe\u{0301}.txt"),
            ],
            [
                ("Caf\u{00e9}.txt", "Caf\u{00e9}.txt"),
                ("Cafe\u{0301}.txt.part", "Cafe\u{0301}.txt.part"),
            ],
        ];
        for pair in nfc_pairs {
            let result =
                validate_download_paths_with_lookup(&sandbox.path, &pair, &no_subdir_lookup);
            if host_nfc_nfd {
                assert!(
                    matches!(result, Err(IaGetError::PathCollision { .. })),
                    "prefixed probe on normalisation-insensitive host should reject {pair:?}, got {result:?}"
                );
            } else {
                assert!(
                    result.is_ok(),
                    "prefixed probe on normalisation-sensitive host should allow {pair:?}, got {result:?}"
                );
            }
            assert_no_probe_dirs_remaining(&sandbox.path);
        }

        // 3. Direct prefixed probe verifies both collision detection and distinctness, leaving zero
        // `.ia-get-fs-probe-*` entries behind.
        let colliding_names = vec!["same.txt".to_string(), "same.txt".to_string()];
        assert_eq!(
            probe_prefixed_names_in_dir(&sandbox.path, &colliding_names, &[0, 1]),
            PrefixedProbeOutcome::Resolved(vec![0, 0])
        );
        assert_no_probe_dirs_remaining(&sandbox.path);

        let distinct_names = vec!["alpha.txt".to_string(), "beta.txt".to_string()];
        assert_eq!(
            probe_prefixed_names_in_dir(&sandbox.path, &distinct_names, &[0, 1]),
            PrefixedProbeOutcome::Resolved(vec![0, 1])
        );
        assert_no_probe_dirs_remaining(&sandbox.path);

        // 4. Pre-existing foreign file matching the first probe filename is never mistaken for a
        // collision and is never removed; the probe retries cleanly with the next sequence number.
        let pid = std::process::id();
        let initial_seq: u64 = 900_000_000;
        let foreign_path = sandbox.join(&format!(".ia-get-fs-probe-{pid}-{initial_seq}-beta.txt"));
        fs::write(&foreign_path, b"pre-existing-foreign-file").unwrap();

        let mut seq = initial_seq;
        let foreign_res =
            probe_prefixed_names_in_dir_with_seq(&sandbox.path, &distinct_names, &[0, 1], || {
                let current = seq;
                seq += 1;
                current
            });
        assert_eq!(foreign_res, PrefixedProbeOutcome::Resolved(vec![0, 1]));
        assert_eq!(
            seq,
            initial_seq + 2,
            "probe should retry with next sequence after foreign collision"
        );
        assert_eq!(
            fs::read(&foreign_path).unwrap(),
            b"pre-existing-foreign-file",
            "pre-existing foreign file must remain untouched"
        );
        fs::remove_file(&foreign_path).unwrap();
        assert_no_probe_dirs_remaining(&sandbox.path);
    }

    #[test]
    fn test_per_parent_lookup_inconclusive_probe_fails_closed() {
        let lookup = MockDirectoryLookup::new([
            ("/exact", MockDirRule::Exact),
            ("/exact/unverifiable", MockDirRule::InconclusiveProbe),
            ("/unverifiable", MockDirRule::InconclusiveProbe),
        ]);

        // Zero-I/O fast path still succeeds on an unverifiable directory for non-colliding ASCII.
        assert!(validate_download_paths_with_lookup(
            Path::new("/unverifiable"),
            &[("alpha.txt", "alpha.txt"), ("beta.txt", "beta.txt")],
            &lookup,
        )
        .is_ok());
        assert!(lookup.probed().is_empty());

        // Unicode and case-only candidates fail closed when the probe is inconclusive.
        for entries in [
            [
                ("Caf\u{00e9}.txt", "Caf\u{00e9}.txt"),
                ("Cafe\u{0301}.txt", "Cafe\u{0301}.txt"),
            ],
            [("Foo.txt", "Foo.txt"), ("foo.txt", "foo.txt")],
        ] {
            lookup.clear_probed();
            let err =
                validate_download_paths_with_lookup(Path::new("/unverifiable"), &entries, &lookup)
                    .unwrap_err();
            assert!(
                matches!(err, IaGetError::FileSystem(ref msg) if msg.contains("could not verify")),
                "inconclusive probe must fail closed with IaGetError::FileSystem, got {err:?}"
            );
            assert_eq!(lookup.probed(), vec![PathBuf::from("/unverifiable")]);
        }

        // Inconclusive subdirectory under an exact root fails closed for candidates in that subdir.
        lookup.clear_probed();
        let subdir_err = validate_download_paths_with_lookup(
            Path::new("/exact"),
            &[
                (
                    "unverifiable/Caf\u{00e9}.txt",
                    "unverifiable/Caf\u{00e9}.txt",
                ),
                (
                    "unverifiable/Cafe\u{0301}.txt",
                    "unverifiable/Cafe\u{0301}.txt",
                ),
            ],
            &lookup,
        )
        .unwrap_err();
        assert!(
            matches!(subdir_err, IaGetError::FileSystem(ref msg) if msg.contains("/exact/unverifiable")),
            "inconclusive child directory probe must fail closed, got {subdir_err:?}"
        );
        assert_eq!(lookup.probed(), vec![PathBuf::from("/exact/unverifiable")]);
    }

    #[test]
    fn test_overlong_and_unprobable_names_fail_closed_even_in_populated_dir() {
        let populated = TempTestDir::new("overlong-populated-failclosed");
        fs::write(populated.join("Sample.txt"), b"existing").unwrap();
        let host_case = measure_host_dir_case_sensitivity(&populated.path);

        // Overlong ASCII names cannot be probed and must never silently pass even when read-only
        // child probing can observe host case sensitivity.
        let long_lower = "a".repeat(300);
        let long_upper = "A".repeat(300);
        let ascii_res = validate_download_paths_for_dir(
            &populated.path,
            [
                (long_lower.as_str(), long_lower.as_str()),
                (long_upper.as_str(), long_upper.as_str()),
            ],
        );
        match host_case {
            PathCaseSensitivity::CaseSensitive => assert!(
                matches!(ascii_res, Err(IaGetError::FileSystem(_))),
                "overlong ASCII pair on CS host must fail closed with FileSystem error, got {ascii_res:?}"
            ),
            PathCaseSensitivity::CaseInsensitive => assert!(
                matches!(
                    ascii_res,
                    Err(IaGetError::PathCollision { .. } | IaGetError::FileSystem(_))
                ),
                "overlong ASCII pair on CI host must not silently pass, got {ascii_res:?}"
            ),
        }
        assert_no_probe_dirs_remaining(&populated.path);

        // Overlong Unicode names always fail closed with FileSystem error.
        let long_nfc = format!("Caf\u{00e9}-{}", "a".repeat(300));
        let long_nfd = format!("Cafe\u{0301}-{}", "a".repeat(300));
        let unicode_res = validate_download_paths_for_dir(
            &populated.path,
            [
                (long_nfc.as_str(), long_nfc.as_str()),
                (long_nfd.as_str(), long_nfd.as_str()),
            ],
        );
        assert!(
            matches!(unicode_res, Err(IaGetError::FileSystem(_))),
            "overlong Unicode pair must fail closed, got {unicode_res:?}"
        );
        assert_no_probe_dirs_remaining(&populated.path);

        // Prefixed-file probe also fails closed for overlong names and leaves zero files behind.
        let no_subdir_lookup = OsDirectoryLookup::without_temp_subdir_probe();
        let prefixed_res = validate_download_paths_with_lookup(
            &populated.path,
            &[
                (long_nfc.as_str(), long_nfc.as_str()),
                (long_nfd.as_str(), long_nfd.as_str()),
            ],
            &no_subdir_lookup,
        );
        assert!(
            matches!(prefixed_res, Err(IaGetError::FileSystem(_))),
            "prefixed probe for overlong Unicode pair must fail closed, got {prefixed_res:?}"
        );
        assert_no_probe_dirs_remaining(&populated.path);
    }

    #[tokio::test]
    async fn test_download_files_in_dir_fail_closed_probe_aborts_before_http_requests() {
        let sandbox = TempTestDir::new("failclosed-aborts-transfers");
        let server = MockServer::start(vec![
            MockResponse::ok(b"must never be requested"),
            MockResponse::ok(b"must never be requested"),
        ])
        .await;

        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();

        // Overlong Unicode candidates cannot be verified by live probing and must abort before
        // any HTTP transfer starts on every platform.
        let long_nfc = format!("Caf\u{00e9}-{}", "a".repeat(300));
        let long_nfd = format!("Cafe\u{0301}-{}", "a".repeat(300));
        let files = vec![
            (server.url("/nfc"), long_nfc, None),
            (server.url("/nfd"), long_nfd, None),
        ];

        let err = download_files_in_dir(
            &client,
            &sandbox.path,
            files,
            2,
            None,
            &RetryPolicy::default(),
            &signal,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(err, IaGetError::FileSystem(ref msg) if msg.contains("could not verify")),
            "expected fail-closed FileSystem error, got {err:?}"
        );

        let requests = server.abort_and_collect().await;
        assert!(
            requests.is_empty(),
            "fail-closed validation error must abort before any HTTP transfer requests: {requests:?}"
        );
        assert_no_probe_dirs_remaining(&sandbox.path);
    }
}
