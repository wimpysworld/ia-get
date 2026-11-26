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
use reqwest::header::HeaderValue;
use reqwest::{Client, ClientBuilder, RequestBuilder, Response, StatusCode, Url};
use tokio::sync::Notify;

use crate::constants::{
    INITIAL_RETRY_DELAY_MS, MAX_REDIRECTS, MAX_RETRIES, MAX_RETRY_AFTER_SECS, MAX_RETRY_DELAY_SECS,
};
use crate::cookie::CookieSource;
use crate::error::IaGetError; // Import IaGetError for explicit error conversion
use crate::utils::{
    create_progress_bar, format_duration, format_run_summary, format_size, format_transfer_rate,
    print_run_summary, sanitize_url_for_log, VerboseLogger,
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
    if !signal.is_running() {
        return Err(interrupted_error());
    }
    if duration.is_zero() {
        return Ok(());
    }
    await_interruptible(tokio::time::sleep(duration), signal).await
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

fn parse_ascii_uint<T: std::str::FromStr>(input: &str, expected_len: Option<usize>) -> Option<T> {
    let valid_len = expected_len.map_or(!input.is_empty(), |len| input.len() == len);
    if !valid_len || !input.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    input.parse::<T>().ok()
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
    let hour = parse_ascii_uint::<u8>(&input[0..2], Some(2))?;
    let minute = parse_ascii_uint::<u8>(&input[3..5], Some(2))?;
    let second = parse_ascii_uint::<u8>(&input[6..8], Some(2))?;
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
    let day = parse_ascii_uint::<u8>(&input[5..7], Some(2))?;
    let month = parse_month(&input[8..11])?;
    let year = parse_ascii_uint::<i64>(&input[12..16], Some(4))?;
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
    let day = parse_ascii_uint::<u8>(&rest[0..2], Some(2))?;
    let month = parse_month(&rest[3..6])?;
    let two_digit_year = parse_ascii_uint::<i64>(&rest[7..9], Some(2))?;
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
        parse_ascii_uint::<u8>(single_digit, Some(1))?
    } else {
        parse_ascii_uint::<u8>(day_str, Some(2))?
    };
    let (hour, minute, second) = parse_hms(&input[11..19])?;
    let year = parse_ascii_uint::<i64>(&input[20..24], Some(4))?;
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
        return Some(match parse_ascii_uint::<u64>(trimmed, None) {
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

/// Attaches a scoped `Cookie` header to `request` when `cookie_source` yields one for `target_url`.
pub fn attach_cookie_header(
    request: RequestBuilder,
    cookie_source: Option<&CookieSource>,
    target_url: &Url,
) -> Result<RequestBuilder> {
    if let Some(cookie_value) = cookie_source
        .map(|source| source.header_value_for_url(target_url))
        .transpose()?
        .flatten()
    {
        Ok(request.header(reqwest::header::COOKIE, cookie_value))
    } else {
        Ok(request)
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

/// Parses an RFC 9110 Section 14.4 `Content-Range` header value for the `bytes` unit.
fn parse_content_range(header: &str) -> Option<ContentRange> {
    let trimmed = header.trim();
    let (unit, range_spec) = trimmed.split_once(' ')?;
    if !unit.eq_ignore_ascii_case("bytes") {
        return None;
    }

    let (range_part, total_part) = range_spec.split_once('/')?;

    if range_part == "*" {
        let complete_length = parse_ascii_uint::<u64>(total_part, None)?;
        return Some(ContentRange::Unsatisfied { complete_length });
    }

    let (start_str, end_str) = range_part.split_once('-')?;
    let start = parse_ascii_uint::<u64>(start_str, None)?;
    let end = parse_ascii_uint::<u64>(end_str, None)?;

    if start > end {
        return None;
    }

    let complete_length = if total_part == "*" {
        None
    } else {
        let total = parse_ascii_uint::<u64>(total_part, None)?;
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

/// Validates a target directory (optionally creating missing directories when
/// `create_if_missing` is `true`) and returns its canonical path.
///
/// When `create_if_missing` is `false`, this helper is strictly read-only: it never creates or
/// modifies directories and verifies search access via `canonical.join(".")`.
pub(crate) fn validate_or_prepare_dir(
    path: &Path,
    label_cap: &str,
    label_lower: &str,
    create_if_missing: bool,
) -> Result<PathBuf> {
    if path.as_os_str().is_empty() {
        return Err(IaGetError::FileSystem(format!(
            "{label_cap} directory path must not be empty"
        )));
    }

    match fs::metadata(path) {
        Ok(meta) if meta.is_dir() => {
            let canonical = resolve_destination_root(path)?;
            if !create_if_missing {
                // Confirm search access into the directory itself without modifying anything.
                fs::metadata(canonical.join(".")).map_err(|err| {
                    IaGetError::FileSystem(format!(
                        "Failed to access {label_lower} directory '{}': {}",
                        path.display(),
                        err
                    ))
                })?;
            }
            Ok(canonical)
        }
        Ok(_) => Err(IaGetError::FileSystem(format!(
            "{label_cap} path '{}' exists and is not a directory",
            path.display()
        ))),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            if fs::symlink_metadata(path).is_ok() {
                return Err(IaGetError::FileSystem(format!(
                    "{label_cap} path '{}' is a broken symlink",
                    path.display()
                )));
            }

            if !create_if_missing {
                return Err(IaGetError::FileSystem(format!(
                    "{label_cap} directory '{}' does not exist",
                    path.display()
                )));
            }

            fs::create_dir_all(path).map_err(|create_err| {
                IaGetError::FileSystem(format!(
                    "Failed to create {label_lower} directory '{}': {}",
                    path.display(),
                    create_err
                ))
            })?;

            resolve_destination_root(path)
        }
        Err(err) => Err(IaGetError::FileSystem(format!(
            "Failed to access {label_lower} directory '{}': {}",
            path.display(),
            err
        ))),
    }
}

/// Validates and prepares the destination output directory, creating missing directories
/// when needed and returning its canonical path.
pub fn prepare_output_dir(path: &Path) -> Result<PathBuf> {
    validate_or_prepare_dir(path, "Output", "output", true)
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

/// Typed internal failure classification for contained-file inspection, opening, and hashing.
enum ContainedFileError {
    /// Path containment or safety refusal (symlink, non-regular file, escape, unsafe path).
    Containment(IaGetError),
    /// Operation was interrupted by a shutdown signal.
    Interrupted(IaGetError),
    /// Ordinary filesystem I/O failure.
    Io(IaGetError),
}

impl ContainedFileError {
    fn into_ia_get_error(self) -> IaGetError {
        match self {
            Self::Containment(err) | Self::Interrupted(err) | Self::Io(err) => err,
        }
    }
}

fn validate_contained_path_inner(
    canonical_root: &Path,
    rel_path: &Path,
    allow_missing_leaf: bool,
) -> std::result::Result<(PathBuf, Option<fs::Metadata>), ContainedFileError> {
    let clean_rel = validate_relative_path(rel_path).map_err(ContainedFileError::Containment)?;
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
                    return Err(ContainedFileError::Containment(IaGetError::FileSystem(
                        format!(
                            "Refusing symlinked directory component '{}'",
                            current.display()
                        ),
                    )));
                }
                if !meta.file_type().is_dir() {
                    return Err(ContainedFileError::Containment(IaGetError::FileSystem(
                        format!(
                            "Expected directory at '{}', found non-directory",
                            current.display()
                        ),
                    )));
                }
                let canonical_current =
                    fs::canonicalize(&current).map_err(|e| ContainedFileError::Io(e.into()))?;
                if !canonical_current.starts_with(canonical_root) {
                    return Err(ContainedFileError::Containment(IaGetError::FileSystem(
                        format!(
                            "Path '{}' escapes destination root '{}'",
                            current.display(),
                            canonical_root.display()
                        ),
                    )));
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                if allow_missing_leaf {
                    for remaining in &parents[index + 1..] {
                        current.push(remaining);
                    }
                    current.push(leaf);
                    return Ok((current, None));
                }
                return Err(ContainedFileError::Io(err.into()));
            }
            Err(err) => return Err(ContainedFileError::Io(err.into())),
        }
    }

    current.push(leaf);
    match fs::symlink_metadata(&current) {
        Ok(meta) => {
            if meta.file_type().is_symlink() {
                return Err(ContainedFileError::Containment(IaGetError::FileSystem(
                    format!("Refusing symlink target '{}'", current.display()),
                )));
            }
            if !meta.file_type().is_file() {
                return Err(ContainedFileError::Containment(IaGetError::FileSystem(
                    format!("Refusing non-regular file target '{}'", current.display()),
                )));
            }
            let canonical_leaf =
                fs::canonicalize(&current).map_err(|e| ContainedFileError::Io(e.into()))?;
            if !canonical_leaf.starts_with(canonical_root) {
                return Err(ContainedFileError::Containment(IaGetError::FileSystem(
                    format!(
                        "Target '{}' escapes destination root '{}'",
                        current.display(),
                        canonical_root.display()
                    ),
                )));
            }
            Ok((current, Some(meta)))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            if allow_missing_leaf {
                Ok((current, None))
            } else {
                Err(ContainedFileError::Io(err.into()))
            }
        }
        Err(err) => Err(ContainedFileError::Io(err.into())),
    }
}

/// Validates that `rel_path` stays inside `canonical_root` with no symlinks or non-regular leaves.
pub(crate) fn validate_contained_path(
    canonical_root: &Path,
    rel_path: &Path,
    allow_missing_leaf: bool,
) -> Result<PathBuf> {
    validate_contained_path_inner(canonical_root, rel_path, allow_missing_leaf)
        .map(|(path, _)| path)
        .map_err(ContainedFileError::into_ia_get_error)
}

/// Validates that `rel_path` stays inside `canonical_root` with no symlinks or non-regular leaves,
/// returning `Ok(Some((path, metadata)))` for an existing regular file or `Ok(None)` when absent.
pub(crate) fn inspect_contained_regular_file(
    canonical_root: &Path,
    rel_path: &Path,
) -> Result<Option<(PathBuf, fs::Metadata)>> {
    match validate_contained_path_inner(canonical_root, rel_path, true)
        .map_err(ContainedFileError::into_ia_get_error)?
    {
        (path, Some(meta)) => Ok(Some((path, meta))),
        (_, None) => Ok(None),
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

fn open_regular_file_for_read_in_canonical_root_typed(
    canonical_root: &Path,
    rel_path: &Path,
) -> std::result::Result<File, ContainedFileError> {
    let (target_path, _) = validate_contained_path_inner(canonical_root, rel_path, false)?;

    let mut options = fs::OpenOptions::new();
    options.read(true);
    configure_no_follow(&mut options);

    let file = options
        .open(&target_path)
        .map_err(|e| ContainedFileError::Io(e.into()))?;
    let meta = file
        .metadata()
        .map_err(|e| ContainedFileError::Io(e.into()))?;
    if !meta.file_type().is_file() {
        return Err(ContainedFileError::Containment(IaGetError::FileSystem(
            format!("Refusing non-regular file '{}'", target_path.display()),
        )));
    }

    Ok(file)
}

/// Opens a verified regular file inside `root` for reading without following symlinks.
pub(crate) fn open_regular_file_for_read(root: &Path, rel_path: &Path) -> Result<File> {
    let canonical_root = resolve_destination_root(root)?;
    open_regular_file_for_read_in_canonical_root_typed(&canonical_root, rel_path)
        .map_err(ContainedFileError::into_ia_get_error)
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

/// Maximum bytes read from a probe file token (`"<idx>"` or `"<pid>:<seq>:<idx>"`).
const MAX_PROBE_TOKEN_BYTES: u64 = 128;

/// Returns `true` when `name` is a single normal path component safe to probe on the host OS.
fn is_probable_component_name(name: &str) -> bool {
    #[cfg(windows)]
    {
        let base = name.split('.').next().unwrap_or(name);
        let trimmed_stem = base.trim_end_matches(|c: char| c.is_whitespace() || c == '.');
        if crate::utils::is_windows_reserved_name(trimmed_stem) {
            return false;
        }
    }

    let mut comps = Path::new(name).components();
    matches!(
        (comps.next(), comps.next()),
        (Some(Component::Normal(_)), None)
    )
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

/// Outcome of reading a bounded probe token back from an already-existing probe file.
enum ProbeTokenRead {
    Matched(usize),
    ForeignOrUnreadable,
    InvalidIndex,
}

/// Reads and validates a bounded probe token from `probe_path` without following symlinks.
fn read_probe_token_index(
    probe_path: &Path,
    token_prefix: &str,
    current_idx: usize,
    names_len: usize,
) -> ProbeTokenRead {
    let mut read_opts = fs::OpenOptions::new();
    read_opts.read(true);
    configure_no_follow(&mut read_opts);
    let Ok(mut file) = read_opts.open(probe_path) else {
        return ProbeTokenRead::ForeignOrUnreadable;
    };
    if !file.metadata().ok().map(|m| m.is_file()).unwrap_or(false) {
        return ProbeTokenRead::ForeignOrUnreadable;
    }
    let mut bytes = Vec::new();
    let read_res = (&mut file)
        .take(MAX_PROBE_TOKEN_BYTES)
        .read_to_end(&mut bytes);
    drop(file);
    if read_res.is_err() || bytes.len() as u64 == MAX_PROBE_TOKEN_BYTES {
        return ProbeTokenRead::ForeignOrUnreadable;
    }
    let Some(idx_str) = std::str::from_utf8(&bytes)
        .ok()
        .and_then(|tag| tag.trim().strip_prefix(token_prefix))
    else {
        return ProbeTokenRead::ForeignOrUnreadable;
    };
    let Ok(earlier_idx) = idx_str.parse::<usize>() else {
        return ProbeTokenRead::InvalidIndex;
    };
    if earlier_idx >= current_idx || earlier_idx >= names_len {
        return ProbeTokenRead::InvalidIndex;
    }
    ProbeTokenRead::Matched(earlier_idx)
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
                match read_probe_token_index(&probe_path, "", idx, names.len()) {
                    ProbeTokenRead::Matched(earlier_idx) => groups[idx] = groups[earlier_idx],
                    ProbeTokenRead::ForeignOrUnreadable | ProbeTokenRead::InvalidIndex => {
                        return TempDirProbeOutcome::Inconclusive;
                    }
                }
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
                    match read_probe_token_index(&probe_path, &token_prefix, idx, names.len()) {
                        ProbeTokenRead::Matched(earlier_idx) => groups[idx] = groups[earlier_idx],
                        ProbeTokenRead::ForeignOrUnreadable => {
                            retry_with_new_seq = true;
                            break;
                        }
                        ProbeTokenRead::InvalidIndex => {
                            return PrefixedProbeOutcome::Inconclusive;
                        }
                    }
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

impl CollisionRole {
    const fn label(self) -> &'static str {
        match self {
            Self::Final => "destination path",
            Self::Staging => "staging '.part' path",
            Self::Subdir => "directory path",
        }
    }
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
        (CollisionRole::Subdir, CollisionRole::Subdir) => {
            unreachable!("matching subdirectories merge instead of colliding")
        }
        (a, b) if a == b => format!("both entries map to the same {}{suffix}", b.label()),
        (a, b) => format!("{} collides with {}{suffix}", b.label(), a.label()),
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
        let role = if depth + 1 < item.segments.len() {
            CollisionRole::Subdir
        } else {
            match item.kind {
                PlannedPathKind::Final => CollisionRole::Final,
                PlannedPathKind::Staging => CollisionRole::Staging,
            }
        };

        for (existing_role, existing_idx) in [
            (CollisionRole::Final, occ.final_item),
            (CollisionRole::Staging, occ.staging_item),
            (CollisionRole::Subdir, occ.subdir_first_item),
        ] {
            if role == CollisionRole::Subdir && existing_role == CollisionRole::Subdir {
                continue;
            }
            if let Some(first_idx) = existing_idx {
                if all_items[first_idx].entry_idx != item.entry_idx {
                    return Err(build_level_collision_error(
                        &all_items[first_idx],
                        item,
                        depth,
                        existing_role,
                        role,
                        level_reason_suffix,
                        inherited_reason_suffix,
                    ));
                }
            }
        }

        match role {
            CollisionRole::Subdir => {
                occ.subdir_first_item.get_or_insert(item_idx);
                occ.subdir_items.push(item_idx);
                if !occ.subdir_name_ids.contains(&name_id) {
                    occ.subdir_name_ids.push(name_id);
                }
            }
            CollisionRole::Final => {
                occ.final_item.get_or_insert(item_idx);
            }
            CollisionRole::Staging => {
                occ.staging_item.get_or_insert(item_idx);
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
    Ok(
        inspect_contained_regular_file(&canonical_root, rel_part_path)?
            .map_or(0, |(_, meta)| meta.len()),
    )
}

/// Atomically promote a verified `.part` file to its final destination path inside `root`.
fn promote_part_file(root: &Path, rel_part_path: &Path, rel_final_path: &Path) -> Result<()> {
    let canonical_root = resolve_destination_root(root)?;
    let part_path = validate_contained_path(&canonical_root, rel_part_path, false)?;
    let final_path = validate_contained_path(&canonical_root, rel_final_path, true)?;
    fs::rename(&part_path, &final_path)?;
    Ok(())
}

fn calculate_md5_in_canonical_root_typed(
    canonical_root: &Path,
    rel_path: &Path,
    running: &Arc<AtomicBool>,
    show_progress: bool,
) -> std::result::Result<String, ContainedFileError> {
    let file = open_regular_file_for_read_in_canonical_root_typed(canonical_root, rel_path)?;
    let file_size = file
        .metadata()
        .map_err(|e| ContainedFileError::Io(e.into()))?
        .len();
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
        let bytes_read = reader
            .read(&mut buffer)
            .map_err(|e| ContainedFileError::Io(e.into()))?;
        if bytes_read == 0 {
            break;
        }

        if !running.load(Ordering::SeqCst) {
            if let Some(ref progress_bar) = pb {
                progress_bar.finish_and_clear();
            }
            return Err(ContainedFileError::Interrupted(
                std::io::Error::new(
                    std::io::ErrorKind::Interrupted,
                    "Hash calculation interrupted by signal",
                )
                .into(),
            ));
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

/// Calculates the MD5 hash of a verified regular file inside `root`,
/// optionally displaying a progress bar for large files.
pub(crate) fn calculate_md5_with_progress(
    root: &Path,
    rel_path: &Path,
    running: &Arc<AtomicBool>,
    show_progress: bool,
) -> Result<String> {
    let canonical_root = resolve_destination_root(root)?;
    calculate_md5_in_canonical_root_typed(&canonical_root, rel_path, running, show_progress)
        .map_err(ContainedFileError::into_ia_get_error)
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

    if inspect_contained_regular_file(&canonical_root, rel_path)?.is_none() {
        return Ok(ExistingFileStatus::Missing);
    }

    let Some(expected) = expected_md5 else {
        return Ok(ExistingFileStatus::CompleteNoMd5);
    };

    match calculate_md5_in_canonical_root_typed(&canonical_root, rel_path, running, true) {
        Ok(local_md5) if local_md5 == expected => Ok(ExistingFileStatus::CompleteMd5Verified),
        Ok(_) => Ok(ExistingFileStatus::NeedsRedownload),
        Err(ContainedFileError::Containment(err) | ContainedFileError::Interrupted(err)) => {
            Err(err)
        }
        Err(ContainedFileError::Io(err)) => {
            println!(
                "{} {} to calculate MD5 hash: {}",
                "╰╼".cyan().dimmed(),
                "Failed".red().bold(),
                err
            );
            Ok(ExistingFileStatus::NeedsRedownload)
        }
    }
}

/// Verifies that `path` is a real directory and not a symlink or non-directory entry.
fn verify_non_symlink_dir(path: &Path, meta: &fs::Metadata) -> Result<()> {
    if meta.file_type().is_symlink() {
        return Err(IaGetError::FileSystem(format!(
            "Refusing symlinked directory '{}'",
            path.display()
        )));
    }
    if !meta.file_type().is_dir() {
        return Err(IaGetError::FileSystem(format!(
            "Cannot create directory '{}': existing path is not a directory",
            path.display()
        )));
    }
    Ok(())
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
                Ok(meta) => verify_non_symlink_dir(&current, &meta)?,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    match fs::create_dir(&current) {
                        Ok(()) => {}
                        Err(create_err)
                            if create_err.kind() == std::io::ErrorKind::AlreadyExists =>
                        {
                            let meta = fs::symlink_metadata(&current)?;
                            verify_non_symlink_dir(&current, &meta)?;
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

/// Kind of retryable failure encountered while downloading file content.
enum DownloadRetryKind<'a> {
    Transport(&'a reqwest::Error),
    HttpStatus {
        status: StatusCode,
        retry_after: Option<&'a str>,
    },
    MidStream(&'a IaGetError),
}

/// Logs, prints, and waits for a retryable download failure, returning `Ok(false)` when retries
/// are exhausted or `Ok(true)` once ready for the next attempt.
async fn wait_for_download_retry(
    retry_count: &mut u32,
    policy: &RetryPolicy,
    safe_initial_url: &str,
    kind: DownloadRetryKind<'_>,
    signal: &ShutdownSignal,
    logger: &VerboseLogger,
) -> Result<bool> {
    if *retry_count >= policy.max_retries {
        let exhausted_detail = match kind {
            DownloadRetryKind::Transport(err) => format!(": {err}"),
            DownloadRetryKind::HttpStatus { status, .. } => format!(" after HTTP {status}"),
            DownloadRetryKind::MidStream(err) => format!(" mid-stream: {err}"),
        };
        logger.log(
            "retry",
            format_args!(
                "exhausted ({}/{}) for {}{}",
                *retry_count, policy.max_retries, safe_initial_url, exhausted_detail
            ),
        );
        println!(
            "{} {}      {} Maximum retries ({}) exceeded",
            "├╼".cyan().dimmed(),
            "Failed".red().bold(),
            "✘".red().bold(),
            policy.max_retries
        );
        return Ok(false);
    }

    *retry_count += 1;
    let retry_after = match kind {
        DownloadRetryKind::HttpStatus { retry_after, .. } => retry_after,
        _ => None,
    };
    let (delay, delay_source) =
        compute_retry_delay_with_source(*retry_count, retry_after, SystemTime::now(), policy);
    let wait_secs = delay.as_secs_f64();
    let (log_reason, status_line) = match kind {
        DownloadRetryKind::Transport(err) => (
            err.to_string(),
            format!(
                "Connection error (attempt {}/{}): {err}",
                *retry_count, policy.max_retries
            ),
        ),
        DownloadRetryKind::HttpStatus { status, .. } => (
            format!("HTTP {status}"),
            format!(
                "HTTP {status} (attempt {}/{})",
                *retry_count, policy.max_retries
            ),
        ),
        DownloadRetryKind::MidStream(err) => (
            format!("mid-stream error ({err})"),
            format!(
                "Download error (attempt {}/{}): {err}",
                *retry_count, policy.max_retries
            ),
        ),
    };
    logger.log(
        "retry",
        format_args!(
            "attempt {}/{} for {} after {log_reason}; waiting {wait_secs:.2}s ({delay_source})",
            *retry_count, policy.max_retries, safe_initial_url
        ),
    );
    println!(
        "{} {}      {} {status_line}",
        "├╼".cyan().dimmed(),
        "Retry".yellow().bold(),
        "⟳".yellow().bold()
    );
    println!(
        "{} {}      Waiting {wait_secs:.1}s before retry...",
        "├╼".cyan().dimmed(),
        "Wait".white()
    );

    sleep_interruptible(delay, signal).await?;
    Ok(true)
}

/// Discards a partial file once so the next attempt refetches without `Range`, failing if a range
/// reset already happened during this download.
fn reset_partial_for_unranged_refetch(
    has_reset_range: &mut bool,
    root: &Path,
    part_path: &Path,
    logger: &VerboseLogger,
    reset_log: std::fmt::Arguments<'_>,
    repeat_log: std::fmt::Arguments<'_>,
    repeat_err: impl FnOnce() -> IaGetError,
) -> Result<()> {
    if *has_reset_range {
        logger.log("download", repeat_log);
        return Err(repeat_err());
    }
    logger.log("download", reset_log);
    *has_reset_range = true;
    remove_part_file_safely(root, part_path)
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
                let mut request = attach_cookie_header(
                    client.get(target_url.clone()),
                    cookie_source,
                    target_url,
                )?;
                if let Some(range_header) = range_header.as_ref() {
                    request = request.header(reqwest::header::RANGE, range_header.clone());
                }
                Ok(request)
            })
            .await?;

        let mut response = match send_outcome {
            RedirectSendOutcome::Response(resp) => resp,
            RedirectSendOutcome::TransportError(e) => {
                let safe_err = e.without_url();
                if !is_retryable_reqwest_error(&safe_err) {
                    return Err(safe_err.into());
                }
                if !wait_for_download_retry(
                    &mut retry_count,
                    policy,
                    &safe_initial_url,
                    DownloadRetryKind::Transport(&safe_err),
                    signal,
                    logger,
                )
                .await?
                {
                    return Err(safe_err.into());
                }
                continue;
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

            if !wait_for_download_retry(
                &mut retry_count,
                policy,
                &safe_initial_url,
                DownloadRetryKind::HttpStatus {
                    status,
                    retry_after: retry_after.as_deref(),
                },
                signal,
                logger,
            )
            .await?
            {
                return Err(IaGetError::Network(format!("HTTP error {}", status)));
            }
            continue;
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
                    reset_partial_for_unranged_refetch(
                        &mut has_reset_range,
                        root,
                        part_path,
                        logger,
                        format_args!(
                            "complete partial file '{}' failed MD5 verification; discarding partial file and refetching without Range",
                            part_path.display()
                        ),
                        format_args!(
                            "complete partial file '{}' failed MD5 verification after range reset",
                            part_path.display()
                        ),
                        || IaGetError::Integrity(format!("MD5 mismatch for {}", part_path.display())),
                    )?;
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
                reset_partial_for_unranged_refetch(
                    &mut has_reset_range,
                    root,
                    part_path,
                    logger,
                    format_args!(
                        "HTTP {status} for '{}': discarding partial file ({current_file_size} bytes) and refetching without Range",
                        part_path.display()
                    ),
                    format_args!(
                        "HTTP {status} for '{}': repeated invalid range response after reset",
                        part_path.display()
                    ),
                    || {
                        IaGetError::Network(format!(
                            "HTTP error {}: repeated invalid range response after reset",
                            status
                        ))
                    },
                )?;
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

                // Ensure file is flushed and closed before sleeping or returning
                if retry_count >= policy.max_retries {
                    let _ = file.flush();
                    drop(file);
                } else {
                    file.flush()?;
                    drop(file);
                }

                if !wait_for_download_retry(
                    &mut retry_count,
                    policy,
                    &safe_initial_url,
                    DownloadRetryKind::MidStream(&e),
                    signal,
                    logger,
                )
                .await?
                {
                    return Err(e);
                }
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
    format_run_summary(
        "Summary:",
        summary.ok,
        Some(summary.skipped),
        summary
            .failed
            .iter()
            .map(|failed| (failed.file_path.as_str(), failed.error.as_str())),
        summary.interrupted,
    )
}

/// Prints a coloured end-of-batch summary to stdout.
fn print_batch_summary(summary: &BatchSummary) {
    print_run_summary(
        "Summary:",
        summary.ok,
        Some(summary.skipped),
        summary
            .failed
            .iter()
            .map(|failed| (failed.file_path.as_str(), failed.error.as_str())),
        summary.interrupted,
    );
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

fn record_batch_interrupted(
    summary: &mut BatchSummary,
    logger: &VerboseLogger,
    reason: std::fmt::Arguments<'_>,
) {
    summary.interrupted = true;
    logger.log("download", reason);
    println!(
        "\n{} Download interrupted. Run the command again to resume remaining files.",
        "✘".red().bold()
    );
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
            record_batch_interrupted(
                &mut summary,
                logger,
                format_args!(
                    "interrupted before file {}/{total_files} ('{file_path}')",
                    index + 1
                ),
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
                    record_batch_interrupted(
                        &mut summary,
                        logger,
                        format_args!(
                            "interrupted while downloading '{file_path}' ({}/{total_files})",
                            index + 1
                        ),
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
    use crate::test_support::{
        fast_retry_policy, md5_hex, slow_retry_policy, CapturedLogs, CapturedRequest, MockResponse,
        MockServer, TempTestDir,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::Mutex;

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

    struct DownloadFixture {
        dir: TempTestDir,
        client: reqwest::Client,
        signal: ShutdownSignal,
    }

    impl DownloadFixture {
        fn new(prefix: &str) -> Self {
            Self::with_client(prefix, http_client_builder().build().unwrap())
        }

        fn with_client(prefix: &str, client: reqwest::Client) -> Self {
            Self {
                dir: TempTestDir::new(prefix),
                client,
                signal: ShutdownSignal::new(),
            }
        }

        fn path(&self) -> &Path {
            self.dir.path()
        }

        fn join(&self, rel: impl AsRef<Path>) -> PathBuf {
            self.dir.join(rel)
        }

        fn part_path(&self, rel: &str) -> PathBuf {
            self.dir.part_path(rel)
        }

        fn write(&self, rel: impl AsRef<Path>, data: impl AsRef<[u8]>) -> PathBuf {
            self.dir.write(rel, data)
        }

        fn write_part(&self, rel: &str, data: impl AsRef<[u8]>) -> PathBuf {
            let path = self.part_path(rel);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent dirs for .part fixture");
            }
            fs::write(&path, data).expect("write .part fixture");
            path
        }

        async fn download(
            &self,
            url: &str,
            rel_path: &str,
            expected_md5: Option<&str>,
            cookie_source: Option<&CookieSource>,
        ) -> Result<FileOutcome> {
            download_single_file(
                &self.client,
                self.path(),
                url,
                rel_path,
                expected_md5,
                &self.signal,
                cookie_source,
            )
            .await
        }

        async fn download_with_policy(
            &self,
            url: &str,
            rel_path: &str,
            expected_md5: Option<&str>,
            cookie_source: Option<&CookieSource>,
            policy: &RetryPolicy,
        ) -> Result<FileOutcome> {
            download_single_file_with_policy(
                &self.client,
                self.path(),
                url,
                rel_path,
                expected_md5,
                &self.signal,
                cookie_source,
                policy,
            )
            .await
        }

        fn assert_downloaded(&self, rel_path: &str, expected: &[u8]) {
            assert_downloaded_file(self.path(), rel_path, expected);
        }
    }

    fn assert_downloaded_file(dir: &Path, rel_path: &str, expected: &[u8]) {
        assert_eq!(
            fs::read(dir.join(rel_path)).unwrap(),
            expected,
            "unexpected downloaded bytes for '{rel_path}'"
        );
        assert!(
            !dir.join(part_file_path(rel_path)).exists(),
            ".part file for '{rel_path}' should be promoted away"
        );
    }

    fn assert_path_collision(
        err: IaGetError,
        expected_local: &str,
        expected_first: Option<&str>,
        expected_second: Option<&str>,
        reason_contains: Option<&str>,
        context: &str,
    ) {
        match err {
            IaGetError::PathCollision {
                local_path,
                first_entry,
                second_entry,
                reason,
            } => {
                assert_eq!(
                    local_path, expected_local,
                    "{context}: unexpected local_path"
                );
                if let Some(first) = expected_first {
                    assert_eq!(first_entry, first, "{context}: unexpected first_entry");
                }
                if let Some(second) = expected_second {
                    assert_eq!(second_entry, second, "{context}: unexpected second_entry");
                }
                if let Some(needle) = reason_contains {
                    assert!(
                        reason.contains(needle),
                        "{context}: expected reason to contain '{needle}', got '{reason}'"
                    );
                }
            }
            other => panic!("{context}: expected PathCollision, got {other:?}"),
        }
    }

    fn same_paths<'a>(paths: &[&'a str]) -> Vec<(&'a str, &'a str)> {
        paths.iter().map(|&p| (p, p)).collect()
    }

    /// Independently observes whether `dir` treats `first` and `second` as equivalent names.
    fn probe_host_dir_equivalent(dir: &Path, first: &str, second: &str) -> bool {
        let first_path = dir.join(first);
        let second_path = dir.join(second);
        fs::write(&first_path, b"probe").expect("write host equivalence probe");
        let equivalent = match fs::symlink_metadata(&second_path) {
            Ok(_) => true,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => false,
            Err(err) => panic!("unexpected error probing host equivalence: {err}"),
        };
        let _ = fs::remove_file(&first_path);
        equivalent
    }

    fn measure_host_dir_case_sensitivity(dir: &Path) -> PathCaseSensitivity {
        if probe_host_dir_equivalent(dir, "host-case-probe-file.txt", "HOST-CASE-PROBE-FILE.TXT") {
            PathCaseSensitivity::CaseInsensitive
        } else {
            PathCaseSensitivity::CaseSensitive
        }
    }

    #[cfg(unix)]
    fn with_readonly_dir<T>(dir: &Path, f: impl FnOnce() -> T) -> T {
        use std::os::unix::fs::PermissionsExt;

        struct RestoreWritable<'a>(&'a Path);
        impl Drop for RestoreWritable<'_> {
            fn drop(&mut self) {
                let _ = fs::set_permissions(self.0, fs::Permissions::from_mode(0o755));
            }
        }

        fs::set_permissions(dir, fs::Permissions::from_mode(0o555)).unwrap();
        let _restore = RestoreWritable(dir);
        f()
    }

    fn proxy_config_from_vars(vars: &[(&str, &str)]) -> Result<ProxyConfig> {
        ProxyConfig::from_lookup(|name| {
            vars.iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| (*v).to_string())
        })
    }

    fn proxy_client(vars: &[(&str, &str)]) -> reqwest::Client {
        let config = proxy_config_from_vars(vars).unwrap();
        http_client_builder_with_proxy(&config)
            .unwrap()
            .build()
            .unwrap()
    }

    fn assert_invalid_proxy(
        err: IaGetError,
        expected_var: &str,
        expected_value: Option<&str>,
        forbidden_secrets: &[&str],
        expect_redacted: bool,
        context: &str,
    ) {
        let rendered = err.to_string();
        match err {
            IaGetError::InvalidProxy { var, value, .. } => {
                assert_eq!(var, expected_var, "{context}: unexpected var");
                if let Some(expected) = expected_value {
                    assert_eq!(value, expected, "{context}: unexpected redacted value");
                }
                if expect_redacted {
                    assert!(
                        value.contains("<redacted>@"),
                        "{context}: expected redaction marker in value: {value}"
                    );
                    assert!(
                        rendered.contains("<redacted>@"),
                        "{context}: expected redaction marker in Display: {rendered}"
                    );
                }
                for secret in forbidden_secrets {
                    assert!(
                        !value.contains(secret),
                        "{context}: secret '{secret}' leaked in value: {value}"
                    );
                    assert!(
                        !rendered.contains(secret),
                        "{context}: secret '{secret}' leaked in Display: {rendered}"
                    );
                }
            }
            other => panic!("{context}: expected InvalidProxy, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_content_range() {
        let max_range = format!("bytes 0-{}/{}", u64::MAX - 1, u64::MAX);
        let max_open = format!("bytes {}-{}/*", u64::MAX, u64::MAX);
        let max_unsat = format!("  bytes */{}  ", u64::MAX);

        let valid_cases: [(&str, ContentRange); 12] = [
            (
                "bytes 0-99/1000",
                ContentRange::Bytes {
                    start: 0,
                    end: 99,
                    complete_length: Some(1000),
                },
            ),
            (
                "bytes 0-0/1",
                ContentRange::Bytes {
                    start: 0,
                    end: 0,
                    complete_length: Some(1),
                },
            ),
            (
                "bytes 100-199/*",
                ContentRange::Bytes {
                    start: 100,
                    end: 199,
                    complete_length: None,
                },
            ),
            (
                "BYTES 50-50/51",
                ContentRange::Bytes {
                    start: 50,
                    end: 50,
                    complete_length: Some(51),
                },
            ),
            (
                "Bytes 10-20/*",
                ContentRange::Bytes {
                    start: 10,
                    end: 20,
                    complete_length: None,
                },
            ),
            (
                "  bytes 0-9/10  ",
                ContentRange::Bytes {
                    start: 0,
                    end: 9,
                    complete_length: Some(10),
                },
            ),
            (
                &max_range,
                ContentRange::Bytes {
                    start: 0,
                    end: u64::MAX - 1,
                    complete_length: Some(u64::MAX),
                },
            ),
            (
                &max_open,
                ContentRange::Bytes {
                    start: u64::MAX,
                    end: u64::MAX,
                    complete_length: None,
                },
            ),
            (
                "bytes */1234",
                ContentRange::Unsatisfied {
                    complete_length: 1234,
                },
            ),
            (
                "bytes */0",
                ContentRange::Unsatisfied { complete_length: 0 },
            ),
            (
                "ByTeS */42",
                ContentRange::Unsatisfied {
                    complete_length: 42,
                },
            ),
            (
                &max_unsat,
                ContentRange::Unsatisfied {
                    complete_length: u64::MAX,
                },
            ),
        ];

        for (header, expected) in valid_cases {
            assert_eq!(parse_content_range(header), Some(expected), "{header:?}");
        }

        for invalid in [
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
        ] {
            assert_eq!(parse_content_range(invalid), None, "header: {invalid:?}");
        }
    }

    #[test]
    fn test_compute_progress_total() {
        for (offset, content_len, complete_len, expected) in [
            (0, Some(500), None, 500),
            (100, Some(400), Some(1000), 1000),
            (100, Some(400), None, 500),
            (100, None, None, 100),
            (0, None, None, 0),
            (100, Some(u64::MAX), None, u64::MAX),
            (u64::MAX, Some(1), None, u64::MAX),
            (u64::MAX, Some(u64::MAX), None, u64::MAX),
            (100, Some(u64::MAX), Some(500), 500),
        ] {
            assert_eq!(
                compute_progress_total(offset, content_len, complete_len),
                expected
            );
        }
    }

    #[test]
    fn test_classify_download_response() {
        let ok_cases = [
            // 200 OK
            (
                StatusCode::OK,
                None,
                Some(1024),
                0,
                true,
                ResponseAction::WriteFromStart {
                    total_expected_size: 1024,
                },
            ),
            (
                StatusCode::OK,
                None,
                Some(1024),
                256,
                true,
                ResponseAction::WriteFromStart {
                    total_expected_size: 1024,
                },
            ),
            (
                StatusCode::OK,
                None,
                None,
                256,
                false,
                ResponseAction::WriteFromStart {
                    total_expected_size: 0,
                },
            ),
            // 206 Partial Content
            (
                StatusCode::PARTIAL_CONTENT,
                Some("bytes 100-199/500"),
                Some(100),
                100,
                true,
                ResponseAction::AppendFromOffset {
                    total_expected_size: 500,
                },
            ),
            (
                StatusCode::PARTIAL_CONTENT,
                Some("bytes 100-199/*"),
                Some(100),
                100,
                false,
                ResponseAction::AppendFromOffset {
                    total_expected_size: 200,
                },
            ),
            (
                StatusCode::PARTIAL_CONTENT,
                Some("bytes 100-199/*"),
                Some(u64::MAX),
                100,
                false,
                ResponseAction::AppendFromOffset {
                    total_expected_size: u64::MAX,
                },
            ),
            (
                StatusCode::PARTIAL_CONTENT,
                Some("bytes 50-199/500"),
                Some(150),
                100,
                true,
                ResponseAction::DiscardAndRefetchUnranged,
            ),
            (
                StatusCode::PARTIAL_CONTENT,
                None,
                Some(100),
                100,
                true,
                ResponseAction::DiscardAndRefetchUnranged,
            ),
            (
                StatusCode::PARTIAL_CONTENT,
                Some("bytes */100"),
                Some(0),
                100,
                true,
                ResponseAction::DiscardAndRefetchUnranged,
            ),
            (
                StatusCode::PARTIAL_CONTENT,
                Some("bytes 150-199/500"),
                Some(50),
                100,
                true,
                ResponseAction::DiscardAndRefetchUnranged,
            ),
            (
                StatusCode::PARTIAL_CONTENT,
                Some("not-a-valid-range"),
                Some(100),
                100,
                true,
                ResponseAction::DiscardAndRefetchUnranged,
            ),
            // 416 Range Not Satisfiable
            (
                StatusCode::RANGE_NOT_SATISFIABLE,
                Some("bytes */500"),
                None,
                500,
                false,
                ResponseAction::VerifyCompletePart,
            ),
            (
                StatusCode::RANGE_NOT_SATISFIABLE,
                Some("bytes */400"),
                None,
                500,
                true,
                ResponseAction::DiscardAndRefetchUnranged,
            ),
            (
                StatusCode::RANGE_NOT_SATISFIABLE,
                Some("bytes */600"),
                None,
                500,
                true,
                ResponseAction::DiscardAndRefetchUnranged,
            ),
            (
                StatusCode::RANGE_NOT_SATISFIABLE,
                Some("bytes 0-99/500"),
                None,
                500,
                true,
                ResponseAction::DiscardAndRefetchUnranged,
            ),
            (
                StatusCode::RANGE_NOT_SATISFIABLE,
                None,
                None,
                500,
                true,
                ResponseAction::VerifyCompletePart,
            ),
            (
                StatusCode::RANGE_NOT_SATISFIABLE,
                None,
                None,
                500,
                false,
                ResponseAction::DiscardAndRefetchUnranged,
            ),
            (
                StatusCode::RANGE_NOT_SATISFIABLE,
                Some("invalid"),
                None,
                500,
                true,
                ResponseAction::VerifyCompletePart,
            ),
            (
                StatusCode::RANGE_NOT_SATISFIABLE,
                Some("invalid"),
                None,
                500,
                false,
                ResponseAction::DiscardAndRefetchUnranged,
            ),
        ];

        for (status, content_range, content_length, offset, has_md5, expected) in ok_cases {
            assert_eq!(
                classify_download_response(status, content_range, content_length, offset, has_md5)
                    .unwrap(),
                expected,
                "{status} range={content_range:?} offset={offset} md5={has_md5}"
            );
        }

        // Unsolicited 206 / 416 when offset == 0 must error
        for (status, range, cl) in [
            (
                StatusCode::PARTIAL_CONTENT,
                Some("bytes 0-99/200"),
                Some(100),
            ),
            (
                StatusCode::RANGE_NOT_SATISFIABLE,
                Some("bytes */0"),
                Some(0),
            ),
        ] {
            let err = classify_download_response(status, range, cl, 0, true).unwrap_err();
            assert!(matches!(err, IaGetError::Network(_)));
        }

        // Other statuses always return Network("HTTP error <status>")
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
        for (input, expected) in [
            ("file.zip", "file.zip.part"),
            ("archive.tar.gz", "archive.tar.gz.part"),
            ("README", "README.part"),
            (
                "dir.v1/sub.dir/data.2026.tar.xz",
                "dir.v1/sub.dir/data.2026.tar.xz.part",
            ),
            (".hidden", ".hidden.part"),
            ("file name (1).bin", "file name (1).bin.part"),
        ] {
            assert_eq!(part_file_path(input), PathBuf::from(expected));
        }
    }

    #[tokio::test]
    async fn test_download_fresh_http_200_verifies_md5_and_promotes_part() {
        let payload = b"fresh archive download payload 1234567890";
        let expected_md5 = md5_hex(payload);
        let server = MockServer::start(vec![MockResponse::ok(payload)]).await;
        let fx = DownloadFixture::new("fresh-200");

        fx.download(
            &server.url("/item.bin"),
            "nested/item.bin",
            Some(&expected_md5),
            None,
        )
        .await
        .expect("fresh 200 download should succeed");

        let requests = server.finish().await;
        assert_eq!(requests.len(), 1);
        assert!(requests[0].request_line.starts_with("GET /item.bin "));
        assert_eq!(requests[0].header("range"), None);
        fx.assert_downloaded("nested/item.bin", payload);
    }

    #[tokio::test]
    async fn test_download_interrupted_mid_stream_leaves_part_without_final_file() {
        let first_chunk = b"0123456789";
        let second_chunk = b"abcdefghij";
        let total_len = first_chunk.len() + second_chunk.len();
        let fx = DownloadFixture::new("interrupt");
        let rel_path = "interrupted.bin";
        let part_path = fx.part_path(rel_path);

        let server = MockServer::start(vec![MockResponse::MidStreamInterrupt {
            status_line: "200 OK",
            headers: vec![("Content-Length".to_string(), total_len.to_string())],
            first_chunk: first_chunk.to_vec(),
            second_chunk: Some(second_chunk.to_vec()),
            watch_path: part_path.clone(),
            expected_part_len: first_chunk.len() as u64,
            signal: fx.signal.clone(),
        }])
        .await;

        let err = fx
            .download(&server.url("/interrupted.bin"), rel_path, None, None)
            .await
            .unwrap_err();

        let requests = server.finish().await;
        assert_eq!(requests.len(), 1);
        assert!(matches!(err, IaGetError::FileSystem(_)));
        assert!(
            !fx.join(rel_path).exists(),
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
        let full: Vec<u8> = [prefix.as_slice(), suffix.as_slice()].concat();
        let expected_md5 = md5_hex(&full);

        let fx = DownloadFixture::new("resume-206");
        fx.write_part("resumed.bin", prefix);

        let server = MockServer::start(vec![MockResponse::partial_content(
            prefix.len() as u64,
            full.len() as u64,
            suffix,
        )])
        .await;

        fx.download(
            &server.url("/resumed.bin"),
            "resumed.bin",
            Some(&expected_md5),
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
        fx.assert_downloaded("resumed.bin", &full);
    }

    #[tokio::test]
    async fn test_download_http_200_to_ranged_request_restarts_without_duplication() {
        let stale_prefix = b"stale-prefix-bytes";
        let full_payload = b"complete fresh payload from byte zero";
        let expected_md5 = md5_hex(full_payload);

        let fx = DownloadFixture::new("ranged-200");
        fx.write_part("restarted.bin", stale_prefix);

        let server = MockServer::start(vec![MockResponse::ok(full_payload)]).await;
        fx.download(
            &server.url("/restarted.bin"),
            "restarted.bin",
            Some(&expected_md5),
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
        fx.assert_downloaded("restarted.bin", full_payload);
    }

    #[tokio::test]
    async fn test_download_bad_206_and_416_discard_and_refetch_unranged() {
        let full_payload = b"valid complete archive payload!!";
        let expected_md5 = md5_hex(full_payload);
        let corrupt_equal = b"corrupt equal length part data!!";
        assert_eq!(corrupt_equal.len(), full_payload.len());
        assert_ne!(corrupt_equal, full_payload);

        let cases: Vec<(&str, &[u8], MockResponse)> = vec![
            (
                "mismatched-206",
                b"stale-01",
                MockResponse::partial_content(0, full_payload.len() as u64, full_payload),
            ),
            (
                "missing-cr-206",
                b"partial",
                MockResponse::with_headers(
                    "206 Partial Content",
                    vec![("Content-Length", "4".to_string())],
                    b"junk",
                ),
            ),
            (
                "malformed-cr-206",
                b"partial",
                MockResponse::with_headers(
                    "206 Partial Content",
                    vec![
                        ("Content-Range", "bytes not-valid".to_string()),
                        ("Content-Length", "4".to_string()),
                    ],
                    b"junk",
                ),
            ),
            (
                "oversized-416",
                b"valid complete archive payload!! PLUS extra stale trailing data",
                MockResponse::range_416(full_payload.len() as u64),
            ),
            (
                "corrupt-equal-len-416",
                corrupt_equal,
                MockResponse::range_416(full_payload.len() as u64),
            ),
        ];

        for (label, stale_part, first_response) in cases {
            let fx = DownloadFixture::new(label);
            fx.write_part("item.bin", stale_part);

            let server =
                MockServer::start(vec![first_response, MockResponse::ok(full_payload)]).await;
            fx.download(
                &server.url("/item.bin"),
                "item.bin",
                Some(&expected_md5),
                None,
            )
            .await
            .unwrap_or_else(|err| panic!("case {label}: expected clean refetch, got {err}"));

            let requests = server.finish().await;
            assert_eq!(requests.len(), 2, "case {label}");
            assert_eq!(
                requests[0].header("range"),
                Some(format!("bytes={}-", stale_part.len()).as_str()),
                "case {label}"
            );
            assert_eq!(
                requests[1].header("range"),
                None,
                "case {label}: refetch must be unranged"
            );
            fx.assert_downloaded("item.bin", full_payload);
        }
    }

    #[tokio::test]
    async fn test_download_http_206_repeated_on_unranged_retry_fails_with_network_error() {
        let stale_prefix = b"partial";
        let fx = DownloadFixture::new("repeated-206");
        fx.write_part("repeated206.bin", stale_prefix);

        let server = MockServer::start(vec![
            MockResponse::with_headers(
                "206 Partial Content",
                vec![("Content-Length", "4".to_string())],
                b"junk",
            ),
            MockResponse::partial_content(0, 10, b"junk"),
        ])
        .await;

        let err = fx
            .download(
                &server.url("/repeated206.bin"),
                "repeated206.bin",
                None,
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
        assert!(!fx.join("repeated206.bin").exists());
        assert!(!fx.part_path("repeated206.bin").exists());
    }

    #[tokio::test]
    async fn test_download_http_416_matching_complete_part_verifies_in_place() {
        let full_payload = b"already complete part payload";
        let expected_md5 = md5_hex(full_payload);

        let fx = DownloadFixture::new("416-complete");
        fx.write_part("complete.bin", full_payload);

        let server =
            MockServer::start(vec![MockResponse::range_416(full_payload.len() as u64)]).await;

        fx.download(
            &server.url("/complete.bin"),
            "complete.bin",
            Some(&expected_md5),
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
        fx.assert_downloaded("complete.bin", full_payload);
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

        let policy = fast_retry_policy(2);

        for (label, status_line, body, expected_requests) in error_cases {
            for existing_prefix in [None, Some(b"preserved-partial-prefix".as_slice())] {
                let mode = if existing_prefix.is_some() {
                    "resume"
                } else {
                    "fresh"
                };
                let fx = DownloadFixture::new(&format!("err-{mode}-{label}"));
                let rel_path = "target.bin";
                if let Some(prefix) = existing_prefix {
                    fx.write_part(rel_path, prefix);
                }

                let responses = (0..expected_requests)
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
                let server = MockServer::start(responses).await;

                let err = fx
                    .download_with_policy(&server.url("/target.bin"), rel_path, None, None, &policy)
                    .await
                    .unwrap_err();

                let requests = server.finish().await;
                assert_eq!(requests.len(), expected_requests, "{mode} {label}");
                assert!(matches!(err, IaGetError::Network(_)), "{mode} {label}");
                assert!(
                    !fx.join(rel_path).exists(),
                    "{mode} {label} must not create final file"
                );
                match existing_prefix {
                    None => assert!(
                        !fx.part_path(rel_path).exists(),
                        "fresh {label} must not create .part"
                    ),
                    Some(prefix) => {
                        for request in &requests {
                            assert_eq!(
                                request.header("range"),
                                Some(format!("bytes={}-", prefix.len()).as_str()),
                                "resume {label}"
                            );
                        }
                        assert_eq!(
                            fs::read(fx.part_path(rel_path)).unwrap(),
                            prefix,
                            "resume {label} must not modify .part with error body"
                        );
                    }
                }
            }
        }
    }

    #[tokio::test]
    async fn test_download_md5_mismatch_returns_integrity_error_and_does_not_promote() {
        let payload = b"corrupted payload bytes";
        let wrong_md5 = "00000000000000000000000000000000";
        let server = MockServer::start(vec![MockResponse::ok(payload)]).await;
        let fx = DownloadFixture::new("md5-mismatch");

        let err = fx
            .download(
                &server.url("/corrupt.bin"),
                "corrupt.bin",
                Some(wrong_md5),
                None,
            )
            .await
            .unwrap_err();

        let requests = server.finish().await;
        assert_eq!(requests.len(), 1);
        assert!(matches!(err, IaGetError::Integrity(_)));
        assert!(
            !fx.join("corrupt.bin").exists(),
            "MD5 mismatch must never promote .part to final path"
        );
        assert_eq!(fs::read(fx.part_path("corrupt.bin")).unwrap(), payload);
    }

    #[tokio::test]
    async fn test_download_http_206_huge_content_length_does_not_overflow_or_panic() {
        let prefix = b"hello";
        let chunk = b"world";
        let fx = DownloadFixture::new("u64-max-cl");
        let rel_path = "huge-cl.bin";
        let part_path = fx.write_part(rel_path, prefix);

        // hyper reserves u64::MAX and u64::MAX - 1 as DecodedLength sentinels, so
        // u64::MAX - 2 is the largest Content-Length it passes to reqwest; with a
        // 5-byte prefix it still overflows current_offset + content_length without
        // saturating arithmetic. (u64::MAX itself is also covered by unit tests.)
        let huge_content_length = u64::MAX - 2;
        assert!(prefix.len() as u64 > u64::MAX - huge_content_length);

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
            signal: fx.signal.clone(),
        }])
        .await;

        let result = fx
            .download(&server.url("/huge-cl.bin"), rel_path, None, None)
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
        assert!(!fx.join(rel_path).exists());
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

        // compute_retry_delay prefers valid Retry-After (clamped) and falls back to jittered backoff
        let now = utc_time(2026, 5, 1, 12, 0, 0);
        let retry_policy = RetryPolicy {
            max_retries: 3,
            initial_delay: Duration::from_secs(2),
            max_delay: Duration::from_secs(16),
            max_retry_after: Duration::from_secs(30),
        };
        assert_eq!(
            compute_retry_delay(1, Some("12"), now, &retry_policy),
            Duration::from_secs(12)
        );
        assert_eq!(
            compute_retry_delay(1, Some("120"), now, &retry_policy),
            Duration::from_secs(30)
        );
        let fallback_none = compute_retry_delay(1, None, now, &retry_policy);
        assert!(
            fallback_none >= Duration::from_secs(1) && fallback_none <= Duration::from_secs(2),
            "expected fallback in [1s, 2s], got {fallback_none:?}"
        );
        let fallback_invalid = compute_retry_delay(2, Some("not-a-date"), now, &retry_policy);
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

        // Transient 503 + 429 (with Retry-After: 0) then 200 succeeds
        let server = MockServer::start(vec![
            MockResponse::status("503 Service Unavailable", b"busy"),
            MockResponse::retry_after("429 Too Many Requests", "0", b"slow down"),
            MockResponse::ok(b"<files/>"),
        ])
        .await;
        let mut events = Vec::new();
        let meta_url = server.parsed_url("/meta.xml");
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
        assert_eq!((events[0].0, events[0].1), (1, 3));
        assert!(events[0].3.contains("503"));
        assert_eq!((events[1].0, events[1].2), (2, Duration::ZERO));
        assert!(events[1].3.contains("429"));

        // Permanent 404 fails immediately without retrying
        let server_404 =
            MockServer::start(vec![MockResponse::status("404 Not Found", b"missing")]).await;
        let mut retry_calls = 0;
        let missing_url = server_404.parsed_url("/missing");
        let err = send_with_retry(
            &missing_url,
            &policy,
            &signal,
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
        let err = send_with_retry(
            &down_url,
            &small_policy,
            &signal,
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
        let err = send_with_retry(
            &rate_limited_url,
            &slow_retry_policy(2),
            &cancel_signal,
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

    #[tokio::test]
    async fn test_batch_mixed_skips_retries_failures_and_continues() {
        let verified_payload = b"already downloaded and verified";
        let verified_md5 = md5_hex(verified_payload);
        let unhashed_payload = b"already present without md5";
        let corrupt_payload = b"corrupted file payload";
        let fresh_payload = b"freshly downloaded nested payload";
        let fresh_md5 = md5_hex(fresh_payload);

        let fx = DownloadFixture::new("batch-mixed-6");
        fx.write("existing_verified.bin", verified_payload);
        fx.write("existing_unhashed.bin", unhashed_payload);

        let server = MockServer::start(vec![
            MockResponse::status("404 Not Found", b"not found"),
            MockResponse::status("503 Service Unavailable", b"busy"),
            MockResponse::status("503 Service Unavailable", b"busy"),
            MockResponse::ok(corrupt_payload),
            MockResponse::ok(fresh_payload),
        ])
        .await;

        let policy = fast_retry_policy(1);
        let files = vec![
            (
                server.url("/existing_verified.bin"),
                "existing_verified.bin".to_string(),
                Some(verified_md5),
            ),
            (
                server.url("/existing_unhashed.bin"),
                "existing_unhashed.bin".to_string(),
                None,
            ),
            (server.url("/missing.bin"), "missing.bin".to_string(), None),
            (server.url("/down.bin"), "down.bin".to_string(), None),
            (
                server.url("/corrupt.bin"),
                "corrupt.bin".to_string(),
                Some("00000000000000000000000000000000".to_string()),
            ),
            (
                server.url("/nested/fresh.bin"),
                "nested/fresh.bin".to_string(),
                Some(fresh_md5),
            ),
        ];

        let summary =
            download_files_in_dir(&fx.client, fx.path(), files, 6, None, &policy, &fx.signal)
                .await
                .unwrap();

        let requests = server.finish().await;
        let expected_lines = [
            "GET /missing.bin ",
            "GET /down.bin ",
            "GET /down.bin ",
            "GET /corrupt.bin ",
            "GET /nested/fresh.bin ",
        ];
        assert_eq!(requests.len(), expected_lines.len());
        for (req, prefix) in requests.iter().zip(expected_lines) {
            assert!(
                req.request_line.starts_with(prefix),
                "expected '{prefix}', got '{}'",
                req.request_line
            );
        }

        assert_eq!(summary.ok, 1);
        assert_eq!(summary.skipped, 2);
        assert!(!summary.interrupted);
        assert_eq!(summary.failed.len(), 3);
        for (idx, (path, needle)) in [
            ("missing.bin", "404"),
            ("down.bin", "503"),
            ("corrupt.bin", "MD5 mismatch"),
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(summary.failed[idx].file_path, path);
            assert!(
                summary.failed[idx].error.contains(needle),
                "expected '{needle}' in '{}'",
                summary.failed[idx].error
            );
        }

        fx.assert_downloaded("existing_verified.bin", verified_payload);
        fx.assert_downloaded("existing_unhashed.bin", unhashed_payload);
        fx.assert_downloaded("nested/fresh.bin", fresh_payload);
        for absent in ["missing.bin", "down.bin"] {
            assert!(!fx.join(absent).exists());
            assert!(!fx.part_path(absent).exists());
        }
        assert!(
            !fx.join("corrupt.bin").exists(),
            "MD5 mismatch must not promote to final path"
        );
        assert_eq!(
            fs::read(fx.part_path("corrupt.bin")).unwrap(),
            corrupt_payload
        );

        let formatted = format_batch_summary(&summary);
        assert!(formatted.contains("1 ok"), "summary was: {formatted}");
        assert!(formatted.contains("2 skipped"), "summary was: {formatted}");
        assert!(formatted.contains("3 failed"), "summary was: {formatted}");
        assert!(
            formatted.contains("missing.bin"),
            "summary was: {formatted}"
        );
    }

    #[tokio::test]
    async fn test_batch_ctrl_c_mid_stream_stops_cleanly_without_recording_failure() {
        let payload1 = b"completed first file";
        let md5_1 = md5_hex(payload1);
        let first_chunk = b"partial-before-ctrl-c";
        let second_chunk = b"-never-completed";
        let total_len = first_chunk.len() + second_chunk.len();

        let fx = DownloadFixture::new("batch-ctrl-c-mid");
        let part2 = fx.part_path("file2.bin");

        let server = MockServer::start(vec![
            MockResponse::ok(payload1),
            MockResponse::MidStreamInterrupt {
                status_line: "200 OK",
                headers: vec![("Content-Length".to_string(), total_len.to_string())],
                first_chunk: first_chunk.to_vec(),
                second_chunk: Some(second_chunk.to_vec()),
                watch_path: part2.clone(),
                expected_part_len: first_chunk.len() as u64,
                signal: fx.signal.clone(),
            },
            MockResponse::ok(b"file3 must not be requested"),
        ])
        .await;

        let policy = fast_retry_policy(1);
        let files = vec![
            (
                server.url("/file1.bin"),
                "file1.bin".to_string(),
                Some(md5_1),
            ),
            (server.url("/file2.bin"), "file2.bin".to_string(), None),
            (server.url("/file3.bin"), "file3.bin".to_string(), None),
        ];

        let summary =
            download_files_in_dir(&fx.client, fx.path(), files, 3, None, &policy, &fx.signal)
                .await
                .unwrap();

        let requests = server.abort().await;
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

        fx.assert_downloaded("file1.bin", payload1);
        assert!(!fx.join("file2.bin").exists());
        assert_eq!(fs::read(&part2).unwrap(), first_chunk);
        assert!(!fx.join("file3.bin").exists());
        assert!(!fx.part_path("file3.bin").exists());
    }

    #[tokio::test]
    async fn test_batch_ctrl_c_before_start_records_interrupted_with_zero_requests() {
        let server = MockServer::start(vec![
            MockResponse::ok(b"should not be requested 1"),
            MockResponse::ok(b"should not be requested 2"),
        ])
        .await;

        let fx = DownloadFixture::new("batch-ctrl-c-before-start");
        fx.signal.trigger();

        let policy = fast_retry_policy(1);
        let files = vec![
            (server.url("/file1.bin"), "file1.bin".to_string(), None),
            (server.url("/file2.bin"), "file2.bin".to_string(), None),
        ];

        let summary =
            download_files_in_dir(&fx.client, fx.path(), files, 2, None, &policy, &fx.signal)
                .await
                .unwrap();

        let requests = server.abort().await;
        assert!(requests.is_empty(), "expected 0 requests, got {requests:?}");
        assert_eq!(summary.ok, 0);
        assert_eq!(summary.skipped, 0);
        assert!(summary.failed.is_empty());
        assert!(summary.interrupted);
        for name in ["file1.bin", "file2.bin"] {
            assert!(!fx.join(name).exists());
            assert!(!fx.part_path(name).exists());
        }
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

    #[cfg(unix)]
    fn create_fifo(path: &Path) {
        let status = std::process::Command::new("mkfifo")
            .arg(path)
            .status()
            .expect("run mkfifo");
        assert!(status.success(), "mkfifo failed for {}", path.display());
    }

    #[tokio::test]
    async fn test_download_refuses_unsafe_paths_and_special_targets_with_zero_requests() {
        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();

        // 1. Traversal, absolute, empty, and current-dir paths
        let dir = TempTestDir::new("unsafe-paths");
        let root = dir.join("root");
        fs::create_dir_all(&root).unwrap();
        let server = MockServer::start(vec![MockResponse::ok(b"should never be fetched")]).await;
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
        assert!(server.abort().await.is_empty());
        assert!(!dir.join("escape.bin").exists());

        // 2. Directory at target (with/without MD5), directory at .part, regular file blocking dir
        type SetupFn = fn(&Path);
        let cross_cases: [(&str, &str, Option<&str>, SetupFn); 4] = [
            ("dir-target-nomd5", "item.bin", None, |r| {
                fs::create_dir(r.join("item.bin")).unwrap();
            }),
            (
                "dir-target-md5",
                "item.bin",
                Some("d41d8cd98f00b204e9800998ecf8427e"),
                |r| {
                    fs::create_dir(r.join("item.bin")).unwrap();
                },
            ),
            ("dir-part", "item.bin", None, |r| {
                fs::create_dir(r.join("item.bin.part")).unwrap();
            }),
            ("file-blocks-dir", "blocker/item.bin", None, |r| {
                fs::write(r.join("blocker"), b"existing regular file").unwrap();
            }),
        ];
        for (label, rel_path, expected_md5, setup) in cross_cases {
            let fx = DownloadFixture::new(label);
            setup(fx.path());
            let server = MockServer::start(vec![MockResponse::ok(b"payload")]).await;
            let err = fx
                .download(&server.url("/item.bin"), rel_path, expected_md5, None)
                .await
                .unwrap_err();
            assert!(matches!(err, IaGetError::FileSystem(_)), "case {label}");
            assert!(server.abort().await.is_empty(), "case {label}");
            if label == "file-blocks-dir" {
                assert_eq!(
                    fs::read(fx.join("blocker")).unwrap(),
                    b"existing regular file"
                );
            } else if label == "dir-part" {
                assert!(fx.part_path("item.bin").is_dir());
                assert!(!fx.join("item.bin").exists());
            } else {
                assert!(fx.join("item.bin").is_dir());
                assert!(!fx.part_path("item.bin").exists());
            }
        }

        // 3. Unix symlinks (intermediate dir, live/dangling .part, live/dangling target) and FIFOs
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            enum UnixSpecial {
                SymlinkParent,
                SymlinkPartLive,
                SymlinkPartDangling,
                SymlinkTargetLive,
                SymlinkTargetDangling,
                FifoTarget,
                FifoPart,
            }

            let md5_some = Some("d41d8cd98f00b204e9800998ecf8427e");
            let unix_cases: [(&str, UnixSpecial, &str, Option<&str>); 10] = [
                (
                    "symlink-parent",
                    UnixSpecial::SymlinkParent,
                    "sub/victim.bin",
                    None,
                ),
                (
                    "symlink-part-live",
                    UnixSpecial::SymlinkPartLive,
                    "item.bin",
                    None,
                ),
                (
                    "symlink-part-dangling",
                    UnixSpecial::SymlinkPartDangling,
                    "item.bin",
                    None,
                ),
                (
                    "symlink-target-live-nomd5",
                    UnixSpecial::SymlinkTargetLive,
                    "item.bin",
                    None,
                ),
                (
                    "symlink-target-live-md5",
                    UnixSpecial::SymlinkTargetLive,
                    "item.bin",
                    md5_some,
                ),
                (
                    "symlink-target-dangling-nomd5",
                    UnixSpecial::SymlinkTargetDangling,
                    "item.bin",
                    None,
                ),
                (
                    "symlink-target-dangling-md5",
                    UnixSpecial::SymlinkTargetDangling,
                    "item.bin",
                    md5_some,
                ),
                (
                    "fifo-target-nomd5",
                    UnixSpecial::FifoTarget,
                    "item.bin",
                    None,
                ),
                (
                    "fifo-target-md5",
                    UnixSpecial::FifoTarget,
                    "item.bin",
                    md5_some,
                ),
                ("fifo-part", UnixSpecial::FifoPart, "item.bin", None),
            ];

            for (label, kind, rel_path, expected_md5) in unix_cases {
                let sandbox = TempTestDir::new(label);
                let root = sandbox.join("root");
                let outside = sandbox.join("outside");
                fs::create_dir_all(&root).unwrap();
                fs::create_dir_all(&outside).unwrap();
                let victim = outside.join("victim.bin");
                let missing = outside.join("missing.bin");

                match kind {
                    UnixSpecial::SymlinkParent => {
                        fs::write(&victim, b"original outside data").unwrap();
                        symlink(&outside, root.join("sub")).unwrap();
                    }
                    UnixSpecial::SymlinkPartLive => {
                        fs::write(&victim, b"do not overwrite").unwrap();
                        symlink(&victim, root.join("item.bin.part")).unwrap();
                    }
                    UnixSpecial::SymlinkPartDangling => {
                        symlink(&missing, root.join("item.bin.part")).unwrap();
                    }
                    UnixSpecial::SymlinkTargetLive => {
                        fs::write(&victim, b"protected target").unwrap();
                        symlink(&victim, root.join("item.bin")).unwrap();
                    }
                    UnixSpecial::SymlinkTargetDangling => {
                        symlink(&missing, root.join("item.bin")).unwrap();
                    }
                    UnixSpecial::FifoTarget => create_fifo(&root.join("item.bin")),
                    UnixSpecial::FifoPart => create_fifo(&root.join("item.bin.part")),
                }

                let server = MockServer::start(vec![MockResponse::ok(b"payload")]).await;
                let result = tokio::time::timeout(
                    Duration::from_secs(5),
                    download_single_file(
                        &client,
                        &root,
                        &server.url("/item.bin"),
                        rel_path,
                        expected_md5,
                        &signal,
                        None,
                    ),
                )
                .await
                .unwrap_or_else(|_| panic!("case {label}: check must not hang"));

                assert!(
                    matches!(result.unwrap_err(), IaGetError::FileSystem(_)),
                    "case {label}"
                );
                assert!(server.abort().await.is_empty(), "case {label}");
                match kind {
                    UnixSpecial::SymlinkParent => {
                        assert_eq!(fs::read(&victim).unwrap(), b"original outside data");
                        assert!(!outside.join("victim.bin.part").exists());
                    }
                    UnixSpecial::SymlinkPartLive => {
                        assert_eq!(fs::read(&victim).unwrap(), b"do not overwrite");
                        assert!(!root.join("item.bin").exists());
                    }
                    UnixSpecial::SymlinkPartDangling => {
                        assert!(!missing.exists());
                        assert!(!root.join("item.bin").exists());
                    }
                    UnixSpecial::SymlinkTargetLive => {
                        assert_eq!(fs::read(&victim).unwrap(), b"protected target");
                    }
                    UnixSpecial::SymlinkTargetDangling => assert!(!missing.exists()),
                    UnixSpecial::FifoTarget | UnixSpecial::FifoPart => {}
                }
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn test_unix_open_nofollow_flag_rejects_symlink() {
        use std::os::unix::fs::{symlink, OpenOptionsExt};

        let Some(flag) = unix_open_nofollow_flag() else {
            return;
        };

        let dir = TempTestDir::new("nofollow-flag");
        let real_file = dir.write("real.bin", b"hello");
        let link_file = dir.join("link.bin");
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
    #[test]
    fn test_unix_promote_part_file_refuses_symlink_fifo_and_directory_targets() {
        use std::os::unix::fs::symlink;

        enum PromoteTarget {
            LiveSymlink,
            DanglingSymlink,
            Fifo,
            Directory,
        }

        for (label, kind) in [
            ("promote-symlink-live", PromoteTarget::LiveSymlink),
            ("promote-symlink-dangling", PromoteTarget::DanglingSymlink),
            ("promote-fifo", PromoteTarget::Fifo),
            ("promote-dir", PromoteTarget::Directory),
        ] {
            let dir = TempTestDir::new(label);
            let root = dir.join("root");
            let outside = dir.join("outside");
            fs::create_dir_all(&root).unwrap();
            fs::create_dir_all(&outside).unwrap();
            fs::write(root.join("item.bin.part"), b"staged").unwrap();

            let outside_file = outside.join("victim.bin");
            match kind {
                PromoteTarget::LiveSymlink => {
                    fs::write(&outside_file, b"original").unwrap();
                    symlink(&outside_file, root.join("item.bin")).unwrap();
                }
                PromoteTarget::DanglingSymlink => {
                    symlink(&outside_file, root.join("item.bin")).unwrap();
                }
                PromoteTarget::Fifo => create_fifo(&root.join("item.bin")),
                PromoteTarget::Directory => fs::create_dir(root.join("item.bin")).unwrap(),
            }

            let err = promote_part_file(&root, Path::new("item.bin.part"), Path::new("item.bin"))
                .unwrap_err();
            assert!(matches!(err, IaGetError::FileSystem(_)), "case {label}");
            assert_eq!(fs::read(root.join("item.bin.part")).unwrap(), b"staged");
            match kind {
                PromoteTarget::LiveSymlink => {
                    assert_eq!(fs::read(&outside_file).unwrap(), b"original");
                }
                PromoteTarget::DanglingSymlink => assert!(!outside_file.exists()),
                PromoteTarget::Directory => assert!(root.join("item.bin").is_dir()),
                PromoteTarget::Fifo => {}
            }
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_unix_symlinked_output_root_allows_contained_downloads_and_blocks_inner_escapes() {
        use std::os::unix::fs::symlink;

        let sandbox = TempTestDir::new("symlinked-output-root");
        let real_root = sandbox.join("real_root");
        let link_root = sandbox.join("link_root");
        let outside = sandbox.join("outside");
        fs::create_dir_all(&real_root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        symlink(&real_root, &link_root).unwrap();
        symlink(&outside, real_root.join("escape")).unwrap();

        let canonical = prepare_output_dir(&link_root)
            .expect("symlinked --output-dir pointing to an existing directory should succeed");
        assert_eq!(canonical, fs::canonicalize(&real_root).unwrap());
        assert!(matches!(
            validate_contained_path(&canonical, Path::new("escape/bad.bin"), true),
            Err(IaGetError::FileSystem(_))
        ));

        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();

        // Single-file download into symlinked root succeeds; inner symlink escape fails with 0 requests
        let single_payload = b"downloaded through symlinked root";
        let server_single = MockServer::start(vec![MockResponse::ok(single_payload)]).await;
        download_single_file(
            &client,
            &link_root,
            &server_single.url("/single.bin"),
            "nested/single.bin",
            Some(&md5_hex(single_payload)),
            &signal,
            None,
        )
        .await
        .expect("single download into symlinked root should succeed");
        assert_eq!(server_single.finish().await.len(), 1);
        assert_downloaded_file(&real_root, "nested/single.bin", single_payload);

        let server_single_bad = MockServer::start(vec![MockResponse::ok(b"nope")]).await;
        let err = download_single_file(
            &client,
            &link_root,
            &server_single_bad.url("/bad.bin"),
            "escape/single_bad.bin",
            None,
            &signal,
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, IaGetError::FileSystem(_)));
        assert!(server_single_bad.abort().await.is_empty());

        // Batch download into symlinked root skips inner escape (0 requests) and downloads sibling
        let batch_payload = b"batch downloaded via symlinked output-dir";
        let server_batch = MockServer::start(vec![MockResponse::ok(batch_payload)]).await;
        let files = vec![
            (
                server_batch.url("/batch_bad.bin"),
                "escape/batch_bad.bin".to_string(),
                None,
            ),
            (
                server_batch.url("/batch_good.bin"),
                "nested/batch_good.bin".to_string(),
                Some(md5_hex(batch_payload)),
            ),
        ];
        let summary = download_files_in_dir(
            &client,
            &link_root,
            files,
            2,
            None,
            &RetryPolicy::default(),
            &signal,
        )
        .await
        .expect("batch into symlinked output-dir should return summary");

        let batch_requests = server_batch.finish().await;
        assert_eq!(batch_requests.len(), 1);
        assert!(batch_requests[0]
            .request_line
            .starts_with("GET /batch_good.bin "));
        assert_eq!(summary.ok, 1);
        assert_eq!(summary.failed.len(), 1);
        assert_eq!(summary.failed[0].file_path, "escape/batch_bad.bin");
        assert!(summary.failed[0]
            .error
            .starts_with("File operation failed:"));
        assert!(!outside.join("single_bad.bin").exists());
        assert!(!outside.join("batch_bad.bin").exists());
        assert_downloaded_file(&real_root, "nested/batch_good.bin", batch_payload);
    }

    #[tokio::test]
    async fn test_download_cross_host_redirect_strips_cookie_and_verifies_md5() {
        let payload = b"cross-host redirected archive payload with verified md5";
        let expected_md5 = md5_hex(payload);

        let target_server = MockServer::start_on_localhost(vec![MockResponse::ok(payload)]).await;
        let origin_server = MockServer::start(vec![MockResponse::redirect(
            "302 Found",
            target_server.url("/final/item.bin"),
        )])
        .await;

        let cookie_source =
            CookieSource::from_netscape_str("127.0.0.1\tFALSE\t/\tFALSE\t0\tsession\tsecret123\n");
        let fx = DownloadFixture::new("redirect-strip-cookie");

        fx.download(
            &origin_server.url("/download/item.bin"),
            "item.bin",
            Some(&expected_md5),
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
        fx.assert_downloaded("item.bin", payload);
    }

    #[tokio::test]
    async fn test_download_same_host_redirect_rescopes_and_orders_path_cookies() {
        let payload = b"same-host path-scoped redirect payload";
        let expected_md5 = md5_hex(payload);

        let server = MockServer::start(vec![
            MockResponse::redirect("302 Found", "/download/public/item.bin"),
            MockResponse::ok(payload),
        ])
        .await;

        // Place shorter path before longer path in file order to verify RFC 6265 longest-path-first sorting.
        let cookie_source = CookieSource::from_netscape_str(concat!(
            "127.0.0.1\tFALSE\t/\tFALSE\t0\troot\t1\n",
            "127.0.0.1\tFALSE\t/download/private\tFALSE\t0\tprivate\t1\n",
        ));

        let fx = DownloadFixture::new("redirect-path-scope");
        fx.download(
            &server.url("/download/private/item.bin"),
            "item.bin",
            Some(&expected_md5),
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
        fx.assert_downloaded("item.bin", payload);
    }

    #[tokio::test]
    async fn test_download_resume_preserves_range_across_all_redirect_statuses() {
        let prefix = b"partial-prefix-";
        let suffix = b"resumed-payload-after-redirect";
        let full: Vec<u8> = [prefix.as_slice(), suffix.as_slice()].concat();
        let expected_md5 = md5_hex(&full);
        let expected_range = format!("bytes={}-", prefix.len());

        for (code, status_line) in [
            ("301", "301 Moved Permanently"),
            ("302", "302 Found"),
            ("303", "303 See Other"),
            ("307", "307 Temporary Redirect"),
            ("308", "308 Permanent Redirect"),
        ] {
            let fx = DownloadFixture::new(&format!("resume-redirect-{code}"));
            fx.write_part("resumed.bin", prefix);

            let server = MockServer::start(vec![
                MockResponse::redirect(status_line, "/cdn/resumed.bin"),
                MockResponse::partial_content(prefix.len() as u64, full.len() as u64, suffix),
            ])
            .await;

            fx.download(
                &server.url("/download/resumed.bin"),
                "resumed.bin",
                Some(&expected_md5),
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
            fx.assert_downloaded("resumed.bin", &full);
        }
    }

    #[tokio::test]
    async fn test_download_raw_vs_cookies_txt_disambiguation_end_to_end() {
        let payload = b"cookie source disambiguation payload";
        let expected_md5 = md5_hex(payload);

        // Raw cookie string: strictly scoped to HTTPS archive.org, so http://127.0.0.1 receives no Cookie.
        let raw_source = CookieSource::from_input("logged-in-user=yes; logged-in-sig=abc123")
            .expect("raw cookie string should parse");
        assert!(matches!(raw_source, CookieSource::Raw(_)));

        let fx_raw = DownloadFixture::new("cookie-raw-e2e");
        let server_raw = MockServer::start(vec![MockResponse::ok(payload)]).await;
        fx_raw
            .download(
                &server_raw.url("/download/item.bin"),
                "item.bin",
                Some(&expected_md5),
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
        fx_raw.assert_downloaded("item.bin", payload);

        // Netscape cookies.txt file: attaches only matching host and path cookies.
        let fx_file = DownloadFixture::new("cookie-file-e2e");
        let cookie_file = fx_file.write(
            "cookies.txt",
            concat!(
                "# Netscape HTTP Cookie File\n",
                "127.0.0.1\tFALSE\t/download\tFALSE\t0\tsession\tfromfile\n",
                "127.0.0.1\tFALSE\t/other\tFALSE\t0\tother\tskip\n",
                "example.com\tFALSE\t/\tFALSE\t0\tforeign\tskip\n",
            ),
        );
        let file_source = CookieSource::from_input(cookie_file.to_str().unwrap())
            .expect("cookies.txt file should parse");
        assert!(matches!(file_source, CookieSource::NetscapeFile(_)));

        let server_file = MockServer::start(vec![MockResponse::ok(payload)]).await;
        fx_file
            .download(
                &server_file.url("/download/item.bin"),
                "item.bin",
                Some(&expected_md5),
                Some(&file_source),
            )
            .await
            .expect("download with cookies.txt file should succeed");

        let file_requests = server_file.finish().await;
        assert_eq!(file_requests.len(), 1);
        assert_eq!(file_requests[0].header("cookie"), Some("session=fromfile"));
        fx_file.assert_downloaded("item.bin", payload);
    }

    #[tokio::test]
    async fn test_download_redirect_errors_fail_fast_without_retrying() {
        let policy = slow_retry_policy(3);

        let mut too_many_responses: Vec<MockResponse> = (0..=MAX_REDIRECTS)
            .map(|hop| MockResponse::redirect("302 Found", format!("/hop-{}", hop + 1)))
            .collect();
        too_many_responses.push(MockResponse::ok(b"should not retry"));

        let cases: Vec<(&str, &str, Vec<MockResponse>, usize, &str)> = vec![
            (
                "direct-loop",
                "/a",
                vec![
                    MockResponse::redirect("302 Found", "/a"),
                    MockResponse::ok(b"should not retry"),
                ],
                1,
                "Redirect loop",
            ),
            (
                "two-hop-loop",
                "/a",
                vec![
                    MockResponse::redirect("302 Found", "/b"),
                    MockResponse::redirect("302 Found", "/a"),
                    MockResponse::ok(b"should not retry"),
                ],
                2,
                "Redirect loop",
            ),
            (
                "fragment-only-loop",
                "/a",
                vec![
                    MockResponse::redirect("302 Found", "/a#section"),
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
                    MockResponse::status("302 Found", b""),
                    MockResponse::ok(b"should not retry"),
                ],
                1,
                "missing Location",
            ),
            (
                "empty-location",
                "/empty",
                vec![
                    MockResponse::redirect("302 Found", "   "),
                    MockResponse::ok(b"should not retry"),
                ],
                1,
                "empty Location",
            ),
            (
                "non-http-scheme",
                "/scheme",
                vec![
                    MockResponse::redirect("302 Found", "file:///etc/passwd"),
                    MockResponse::ok(b"should not retry"),
                ],
                1,
                "Unsupported redirect scheme",
            ),
        ];

        for (label, initial_path, responses, expected_requests, expected_msg) in cases {
            let fx = DownloadFixture::new(&format!("redirect-err-{label}"));
            let server = MockServer::start(responses).await;

            let start = std::time::Instant::now();
            let err = fx
                .download_with_policy(&server.url(initial_path), "item.bin", None, None, &policy)
                .await
                .unwrap_err();
            let elapsed = start.elapsed();

            let requests = server.abort().await;
            assert_eq!(requests.len(), expected_requests, "case {label}");
            assert!(
                elapsed < Duration::from_secs(2),
                "case {label}: redirect error must fail fast, took {elapsed:?}"
            );
            assert!(
                matches!(err, IaGetError::Network(_)),
                "case {label}: {err:?}"
            );
            assert!(
                err.to_string().contains(expected_msg),
                "case {label}: expected '{expected_msg}' in '{err}'"
            );
            assert!(!fx.join("item.bin").exists(), "case {label}");
            assert!(!fx.part_path("item.bin").exists(), "case {label}");
        }
    }

    #[tokio::test]
    async fn test_download_retry_after_redirected_503_restarts_from_initial_hop() {
        let payload = b"payload recovered after redirected 503";
        let expected_md5 = md5_hex(payload);

        let server = MockServer::start(vec![
            MockResponse::redirect("302 Found", "/cdn/item.bin"),
            MockResponse::retry_after("503 Service Unavailable", "0", b"busy"),
            MockResponse::redirect("302 Found", "/cdn/item.bin"),
            MockResponse::ok(payload),
        ])
        .await;

        let fx = DownloadFixture::new("redirect-retry-503");
        let policy = slow_retry_policy(2);

        let start = std::time::Instant::now();
        fx.download_with_policy(
            &server.url("/download/item.bin"),
            "item.bin",
            Some(&expected_md5),
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
            "Retry-After: 0 should retry immediately, took {elapsed:?}"
        );
        fx.assert_downloaded("item.bin", payload);
    }

    #[tokio::test]
    async fn test_download_cookie_expiry_reevaluated_across_retry_wait() {
        let payload = b"payload delivered after short-lived cookie expired";
        let expected_md5 = md5_hex(payload);

        let server = MockServer::start(vec![
            MockResponse::retry_after("503 Service Unavailable", "2", b"busy"),
            MockResponse::ok(payload),
        ])
        .await;

        let fx = DownloadFixture::new("cookie-expiry-retry");
        let policy = slow_retry_policy(2);

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

        fx.download_with_policy(
            &server.url("/download/item.bin"),
            "item.bin",
            Some(&expected_md5),
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
        fx.assert_downloaded("item.bin", payload);
    }

    #[test]
    fn test_resolve_redirect_target_allows_valid_transitions() {
        for (current_str, location, expected) in [
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
        ] {
            let current = Url::parse(current_str).unwrap();
            let resolved = resolve_redirect_target(&current, location)
                .unwrap_or_else(|err| panic!("{current_str} -> {location} failed: {err}"));
            assert_eq!(resolved.as_str(), expected);
        }
    }

    #[test]
    fn test_resolve_redirect_target_rejects_downgrade_and_invalid_locations() {
        let current = Url::parse("https://archive.org/download/item/a").unwrap();

        for (location, expected_msg) in [
            ("http://archive.org/download/item/a", "downgrade"),
            ("http://mirror.example/a", "downgrade"),
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
        let _cwd_guard = CurrentDirGuard::enter(dir.path());

        let canonical = prepare_output_dir(Path::new("."))
            .expect("prepare_output_dir('.') should resolve current working directory");
        assert_eq!(canonical, fs::canonicalize(dir.path()).unwrap());
    }

    #[tokio::test]
    async fn test_download_files_with_policy_and_signal_writes_to_default_working_dir() {
        let _cwd_lock = CURRENT_DIR_LOCK.lock().await;
        let fx = DownloadFixture::new("default-cwd-download");
        let _cwd_guard = CurrentDirGuard::enter(fx.path());

        let payload_top = b"default working directory top-level payload";
        let payload_nested = b"default working directory nested payload";
        let server = MockServer::start(vec![
            MockResponse::ok(payload_top),
            MockResponse::ok(payload_nested),
        ])
        .await;

        let files = vec![
            (
                server.url("/top.bin"),
                "top.bin".to_string(),
                Some(md5_hex(payload_top)),
            ),
            (
                server.url("/nested.bin"),
                "sub/nested.bin".to_string(),
                Some(md5_hex(payload_nested)),
            ),
        ];

        let summary = download_files_with_policy_and_signal(
            &fx.client,
            files,
            2,
            None,
            &RetryPolicy::default(),
            &fx.signal,
        )
        .await
        .expect("default working directory download should succeed");

        assert_eq!(server.finish().await.len(), 2);
        assert_eq!(summary.ok, 2);
        assert_eq!(summary.skipped, 0);
        assert!(summary.failed.is_empty());
        assert!(!summary.interrupted);
        fx.assert_downloaded("top.bin", payload_top);
        fx.assert_downloaded("sub/nested.bin", payload_nested);
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
        let server = MockServer::start(vec![
            MockResponse::ok(payload_top),
            MockResponse::ok(payload_deep),
            MockResponse::ok(payload_sibling),
        ])
        .await;
        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();

        let files = vec![
            (
                server.url("/top.txt"),
                "top.txt".to_string(),
                Some(md5_hex(payload_top)),
            ),
            (
                server.url("/dir1/sub/file.bin"),
                "dir1/sub/file.bin".to_string(),
                Some(md5_hex(payload_deep)),
            ),
            (
                server.url("/dir2/data.bin"),
                "dir2/data.bin".to_string(),
                Some(md5_hex(payload_sibling)),
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

        assert_eq!(server.finish().await.len(), 3);
        assert!(output_dir.is_dir());
        assert_eq!(summary.ok, 3);
        assert_eq!(summary.skipped, 0);
        assert!(summary.failed.is_empty());
        assert!(!summary.interrupted);
        assert_downloaded_file(&output_dir, "top.txt", payload_top);
        assert_downloaded_file(&output_dir, "dir1/sub/file.bin", payload_deep);
        assert_downloaded_file(&output_dir, "dir2/data.bin", payload_sibling);

        let sandbox_entries: Vec<OsString> = fs::read_dir(sandbox.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(
            sandbox_entries,
            vec![OsString::from("downloads")],
            "nothing should be written outside the target output directory"
        );
    }

    #[tokio::test]
    async fn test_prepare_output_dir_and_download_reject_invalid_paths_with_zero_requests() {
        let sandbox = TempTestDir::new("invalid-output-paths");
        let existing_file = sandbox.write("existing.txt", b"untouched-regular-file");
        let file_ancestor = existing_file.join("sub");
        let empty_path = PathBuf::from("");

        let mut cases: Vec<(&str, PathBuf, Option<&str>)> = vec![
            ("existing-file", existing_file.clone(), None),
            ("file-ancestor", file_ancestor, None),
            ("empty-path", empty_path, None),
        ];

        #[cfg(unix)]
        let missing_symlink_target = {
            use std::os::unix::fs::symlink;
            let missing_target = sandbox.join("missing-target");
            let broken_link = sandbox.join("broken-out");
            symlink(&missing_target, &broken_link).unwrap();
            cases.push(("broken-symlink", broken_link, Some("broken symlink")));
            missing_target
        };

        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();

        for (label, bad_path, expected_substr) in cases {
            let prep_err = prepare_output_dir(&bad_path).unwrap_err();
            assert!(
                matches!(prep_err, IaGetError::FileSystem(_)),
                "case {label}: expected FileSystem error from prepare_output_dir, got {prep_err:?}"
            );
            if let Some(substr) = expected_substr {
                assert!(
                    prep_err.to_string().contains(substr),
                    "case {label}: expected '{substr}' in '{prep_err}'"
                );
            }

            let server =
                MockServer::start(vec![MockResponse::ok(b"must never be requested")]).await;
            let dl_err = download_files_in_dir(
                &client,
                &bad_path,
                vec![(server.url("/file.bin"), "file.bin".to_string(), None)],
                1,
                None,
                &RetryPolicy::default(),
                &signal,
            )
            .await
            .unwrap_err();
            assert!(
                matches!(dl_err, IaGetError::FileSystem(_)),
                "case {label}: expected FileSystem error from download_files_in_dir, got {dl_err:?}"
            );
            assert!(
                server.abort().await.is_empty(),
                "case {label}: expected 0 HTTP requests for invalid output path"
            );
        }

        assert_eq!(fs::read(&existing_file).unwrap(), b"untouched-regular-file");
        #[cfg(unix)]
        assert!(!missing_symlink_target.exists());
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
                Some(md5_hex(safe_payload)),
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
        assert_eq!(requests.len(), 1);
        assert!(requests[0].request_line.starts_with("GET /safe.bin "));
        assert_eq!(summary.ok, 1);
        assert_eq!(summary.skipped, 0);
        assert!(!summary.interrupted);
        assert_eq!(summary.failed.len(), 2);
        for (idx, path) in ["escape_dir/payload.bin", "escape_file.bin"]
            .into_iter()
            .enumerate()
        {
            assert_eq!(summary.failed[idx].file_path, path);
            assert!(summary.failed[idx]
                .error
                .starts_with("File operation failed:"));
        }

        assert!(!outside_dir.join("payload.bin").exists());
        assert!(!outside_dir.join("payload.bin.part").exists());
        assert_eq!(
            fs::read(&outside_victim).unwrap(),
            b"untouched-outside-victim"
        );
        assert_downloaded_file(&output_dir, "safe/sibling.bin", safe_payload);
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

        let mut limiter_100k = RateLimiter::with_start(100_000, t0);
        assert_eq!(
            limiter_100k.delay_for_chunk(25_000, t0),
            Duration::from_millis(250)
        );

        // Zero rate returns Duration::ZERO without dividing by zero or panicking.
        let mut zero_rate = RateLimiter::with_start(0, t0);
        assert_eq!(zero_rate.delay_for_chunk(0, t0), Duration::ZERO);
        assert_eq!(zero_rate.delay_for_chunk(1024, t0), Duration::ZERO);
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
        let fx = DownloadFixture::new("rate-limit-throttled");
        let options = DownloadOptions {
            retry_policy: RetryPolicy::default(),
            limit_rate: Some(parse_rate_limit("100K").unwrap()),
            verbose: false,
        };

        let started = Instant::now();
        let summary = download_files_in_dir_with_options(
            &fx.client,
            fx.path(),
            vec![(
                server.url("/throttled.bin"),
                "throttled.bin".to_string(),
                Some(expected_md5),
            )],
            1,
            None,
            &options,
            &fx.signal,
        )
        .await
        .expect("throttled download should succeed");
        let elapsed = started.elapsed();

        assert_eq!(server.finish().await.len(), 1);
        assert_eq!(summary.ok, 1);
        assert_eq!(summary.skipped, 0);
        assert!(summary.failed.is_empty());
        assert!(!summary.interrupted);
        assert!(
            elapsed >= Duration::from_millis(320),
            "40 KiB at 100 KiB/s (~400ms expected) completed too quickly: {elapsed:?}"
        );
        fx.assert_downloaded("throttled.bin", &payload);
    }

    #[tokio::test]
    async fn test_download_files_with_rate_limit_interrupts_immediately_on_shutdown_signal() {
        let payload: Vec<u8> = (0..(16 * 1024)).map(|i| (i % 239) as u8).collect();
        let server = MockServer::start(vec![MockResponse::ok(&payload)]).await;
        let fx = DownloadFixture::new("rate-limit-interrupt");
        let options = DownloadOptions {
            retry_policy: RetryPolicy::default(),
            limit_rate: Some(parse_rate_limit("1K").unwrap()),
            verbose: false,
        };

        let part_path = fx.part_path("interrupted.bin");
        let trigger_signal = fx.signal.clone();
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

        let started = Instant::now();
        let summary = download_files_in_dir_with_options(
            &fx.client,
            fx.path(),
            vec![(
                server.url("/interrupted.bin"),
                "interrupted.bin".to_string(),
                Some(md5_hex(&payload)),
            )],
            1,
            None,
            &options,
            &fx.signal,
        )
        .await
        .expect("interrupted throttled batch should return summary");
        let elapsed = started.elapsed();

        let _ = trigger_task.await;
        let _requests = server.abort().await;

        assert!(
            elapsed < Duration::from_secs(2),
            "Ctrl+C must interrupt ~16s throttled sleep promptly, took {elapsed:?}"
        );
        assert!(summary.interrupted);
        assert_eq!(summary.ok, 0);
        assert_eq!(summary.skipped, 0);
        assert!(summary.failed.is_empty());
        assert!(!fx.join("interrupted.bin").exists());

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

        let server = MockServer::start(vec![MockResponse::partial_content(
            prefix_len as u64,
            total_len as u64,
            &payload[prefix_len..],
        )])
        .await;

        let fx = DownloadFixture::new("rate-limit-resume");
        fx.write_part("resumed.bin", &payload[..prefix_len]);
        let options = DownloadOptions {
            retry_policy: RetryPolicy::default(),
            limit_rate: Some(parse_rate_limit("100K").unwrap()),
            verbose: false,
        };

        let started = Instant::now();
        let summary = download_files_in_dir_with_options(
            &fx.client,
            fx.path(),
            vec![(
                server.url("/resumed.bin"),
                "resumed.bin".to_string(),
                Some(md5_hex(&payload)),
            )],
            1,
            None,
            &options,
            &fx.signal,
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
        fx.assert_downloaded("resumed.bin", &payload);
    }

    #[test]
    fn proxy_config_env_precedence_and_empty_values() {
        struct ProxyEnvCase<'a> {
            label: &'a str,
            vars: &'a [(&'a str, &'a str)],
            http: Option<&'a str>,
            https: Option<&'a str>,
            all: Option<&'a str>,
            no_proxy: Option<&'a str>,
        }

        let cases = [
            ProxyEnvCase {
                label: "unset",
                vars: &[],
                http: None,
                https: None,
                all: None,
                no_proxy: None,
            },
            ProxyEnvCase {
                label: "whitespace-only",
                vars: &[
                    ("HTTP_PROXY", ""),
                    ("HTTPS_PROXY", "   "),
                    ("ALL_PROXY", "\t \n"),
                    ("NO_PROXY", "  \t "),
                ],
                http: None,
                https: None,
                all: None,
                no_proxy: None,
            },
            ProxyEnvCase {
                label: "empty-lowercase-blocks-uppercase-fallback",
                vars: &[
                    ("http_proxy", ""),
                    ("HTTP_PROXY", "http://upper-http.example.com:8080"),
                    ("https_proxy", "   "),
                    ("HTTPS_PROXY", "https://upper-https.example.com:8443"),
                    ("all_proxy", "  socks5h://127.0.0.1:1080  "),
                    ("ALL_PROXY", "socks5://upper-all.example.com:1080"),
                    ("no_proxy", ""),
                    ("NO_PROXY", "archive.org"),
                ],
                http: None,
                https: None,
                all: Some("socks5h://127.0.0.1:1080"),
                no_proxy: None,
            },
            ProxyEnvCase {
                label: "uppercase-fallback-when-lowercase-unset",
                vars: &[
                    ("HTTP_PROXY", "http://proxy.example.com:8080"),
                    ("HTTPS_PROXY", "https://secure.example.com:8443"),
                    ("ALL_PROXY", "socks5://127.0.0.1:1080"),
                    ("NO_PROXY", " localhost, .archive.org "),
                ],
                http: Some("http://proxy.example.com:8080"),
                https: Some("https://secure.example.com:8443"),
                all: Some("socks5://127.0.0.1:1080"),
                no_proxy: Some("localhost, .archive.org"),
            },
            ProxyEnvCase {
                label: "non-empty-lowercase-overrides-uppercase",
                vars: &[
                    ("http_proxy", "http://lower-http.example:8080"),
                    ("HTTP_PROXY", "http://upper-http.example:8080"),
                    ("https_proxy", "https://lower-https.example:8443"),
                    ("HTTPS_PROXY", "https://upper-https.example:8443"),
                    ("all_proxy", "socks5://lower-all.example:1080"),
                    ("ALL_PROXY", "socks5://upper-all.example:1080"),
                    ("no_proxy", "127.0.0.1,localhost"),
                    ("NO_PROXY", "archive.org"),
                ],
                http: Some("http://lower-http.example:8080"),
                https: Some("https://lower-https.example:8443"),
                all: Some("socks5://lower-all.example:1080"),
                no_proxy: Some("127.0.0.1,localhost"),
            },
        ];

        for case in cases {
            let config = proxy_config_from_vars(case.vars).unwrap();
            assert_eq!(
                config.http_proxy,
                case.http.map(|u| Url::parse(u).unwrap()),
                "{}",
                case.label
            );
            assert_eq!(
                config.https_proxy,
                case.https.map(|u| Url::parse(u).unwrap()),
                "{}",
                case.label
            );
            assert_eq!(
                config.all_proxy,
                case.all.map(|u| Url::parse(u).unwrap()),
                "{}",
                case.label
            );
            assert_eq!(config.no_proxy.as_deref(), case.no_proxy, "{}", case.label);
            if case.http.is_none()
                && case.https.is_none()
                && case.all.is_none()
                && case.no_proxy.is_none()
            {
                assert_eq!(config, ProxyConfig::default(), "{}", case.label);
                http_client_builder_with_proxy(&config)
                    .unwrap()
                    .build()
                    .unwrap();
            }
        }
    }

    #[test]
    fn proxy_config_accepts_supported_schemes_ipv6_credentials_and_omitted_ports() {
        let valid_cases = [
            ("http://127.0.0.1:8080", "http"),
            ("https://127.0.0.1:8443", "https"),
            ("socks4://127.0.0.1:1080", "socks4"),
            ("socks4a://127.0.0.1:1080", "socks4a"),
            ("socks5://127.0.0.1:1080", "socks5"),
            ("socks5h://user:pass@127.0.0.1:1080", "socks5h"),
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
            let config = proxy_config_from_vars(&[("ALL_PROXY", raw)])
                .unwrap_or_else(|err| panic!("expected '{raw}' to be valid, got {err}"));
            assert_eq!(
                config.all_proxy.as_ref().expect("all_proxy set").scheme(),
                expected_scheme
            );
            http_client_builder_with_proxy(&config)
                .unwrap_or_else(|err| panic!("builder failed for '{raw}': {err}"))
                .build()
                .unwrap_or_else(|err| panic!("client build failed for '{raw}': {err}"));
        }
    }

    #[test]
    fn proxy_config_rejects_invalid_urls_and_redacts_credentials() {
        let invalid_cases: [(&str, &str, Option<&str>, &[&str]); 27] = [
            ("HTTP_PROXY", "proxy.example.com:8080", None, &[]),
            (
                "HTTP_PROXY",
                "user:topsecret@proxy.example.com:8080",
                None,
                &["topsecret"],
            ),
            (
                "HTTPS_PROXY",
                "ftp://alice:s3cr3t@proxy.example.com:21",
                None,
                &["s3cr3t"],
            ),
            ("ALL_PROXY", "socks://127.0.0.1:1080", None, &[]),
            (
                "HTTP_PROXY",
                "http://user:pw@proxy.example.com:0",
                None,
                &["pw"],
            ),
            ("ALL_PROXY", "socks5://:1080", None, &[]),
            ("ALL_PROXY", "socks5:///path", None, &[]),
            (
                "HTTP_PROXY",
                "http://user:hunter2@proxy host:8080",
                None,
                &["hunter2"],
            ),
            (
                "HTTP_PROXY",
                "http://user:hidden@proxy.example.com:8080\0",
                None,
                &["hidden"],
            ),
            ("HTTP_PROXY", "http:example.com/a://b", None, &[]),
            ("HTTP_PROXY", "not-a-url", None, &[]),
            ("HTTP_PROXY", "proxy.local:8080", None, &[]),
            ("HTTP_PROXY", "http://", None, &[]),
            ("HTTP_PROXY", "http://:8080", None, &[]),
            ("ALL_PROXY", "socks5://", None, &[]),
            ("HTTP_PROXY", "http://127.0.0.1:99999", None, &[]),
            ("HTTPS_PROXY", "https://127.0.0.1:abc", None, &[]),
            ("ALL_PROXY", "socks5://127.0.0.1:0", None, &[]),
            ("HTTP_PROXY", "://bad", None, &[]),
            ("ALL_PROXY", "socks5://127.0.0.1\t:1080", None, &[]),
            (
                "HTTP_PROXY",
                "http://user:p@ssword@proxy.local:99999",
                None,
                &["p@ssword"],
            ),
            (
                "ALL_PROXY",
                "socks5://alice:supersecret@:1080",
                None,
                &["supersecret"],
            ),
            (
                "HTTP_PROXY",
                "http://alice:p/ass@proxy.local:8080",
                Some("http://<redacted>@proxy.local:8080"),
                &["alice", "p/ass"],
            ),
            (
                "HTTP_PROXY",
                "http://alice:p?w@proxy.local:8080",
                Some("http://<redacted>@proxy.local:8080"),
                &["alice", "p?w"],
            ),
            (
                "HTTP_PROXY",
                "http://alice:p#w@proxy.local:8080",
                Some("http://<redacted>@proxy.local:8080"),
                &["alice", "p#w"],
            ),
            (
                "HTTP_PROXY",
                "alice:p/ass@proxy.local:8080",
                Some("<redacted>@proxy.local:8080"),
                &["alice", "p/ass"],
            ),
            (
                "HTTP_PROXY",
                "http:alice:secret@proxy.local/a://b",
                Some("<redacted>@proxy.local/a://b"),
                &["alice", "secret"],
            ),
        ];

        for (var_name, raw, expected_redacted, secrets) in invalid_cases {
            let err = proxy_config_from_vars(&[(var_name, raw)])
                .expect_err(&format!("expected '{raw}' to fail"));
            assert_invalid_proxy(
                err,
                var_name,
                expected_redacted,
                secrets,
                !secrets.is_empty(),
                raw,
            );
        }

        // Direct builder validation also rejects unsupported schemes and redacts credentials.
        let direct_invalid = ProxyConfig {
            http_proxy: Some(Url::parse("ftp://alice:s3cr3t@proxy.local:21").unwrap()),
            ..ProxyConfig::default()
        };
        let err = http_client_builder_with_proxy(&direct_invalid)
            .expect_err("direct invalid scheme must be rejected");
        assert_invalid_proxy(err, "HTTP_PROXY", None, &["s3cr3t"], true, "direct-ftp");
    }

    #[cfg(unix)]
    #[test]
    fn proxy_config_rejects_non_utf8_os_values() {
        use std::os::unix::ffi::OsStringExt;

        let cases: [(&str, Vec<(&str, OsString)>); 3] = [
            (
                "http_proxy",
                vec![
                    ("http_proxy", OsString::from_vec(vec![0xff, 0xfe])),
                    ("HTTP_PROXY", OsString::from("http://127.0.0.1:8080")),
                ],
            ),
            (
                "ALL_PROXY",
                vec![("ALL_PROXY", OsString::from_vec(vec![0xff, 0x80]))],
            ),
            (
                "NO_PROXY",
                vec![("NO_PROXY", OsString::from_vec(vec![0xfe, 0xff]))],
            ),
        ];

        for (expected_var, vars) in cases {
            let err = ProxyConfig::from_os_lookup(|name| {
                vars.iter()
                    .find(|(k, _)| *k == name)
                    .map(|(_, v)| v.clone())
            })
            .expect_err(&format!("non-UTF-8 {expected_var} must fail"));
            assert_invalid_proxy(
                err,
                expected_var,
                Some("<non-UTF-8>"),
                &[],
                false,
                expected_var,
            );
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
        async fn start(bodies: Vec<Vec<u8>>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind mock SOCKS5 server");
            let addr = listener.local_addr().expect("mock SOCKS5 local_addr");
            let connections = Arc::new(Mutex::new(Vec::new()));
            let connections_clone = Arc::clone(&connections);

            let handle = tokio::spawn(async move {
                for body in bodies {
                    let Ok((mut socket, _)) = listener.accept().await else {
                        break;
                    };
                    let _ = socket.set_nodelay(true);

                    let Some(target) = socks5_handshake(&mut socket).await else {
                        continue;
                    };
                    let Some(request) = socks5_read_http_request(&mut socket).await else {
                        continue;
                    };
                    connections_clone
                        .lock()
                        .await
                        .push(CapturedSocks5Connection { target, request });
                    let mut wire = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .into_bytes();
                    wire.extend_from_slice(&body);
                    let _ = socket.write_all(&wire).await;
                    let _ = socket.flush().await;
                    let _ = socket.shutdown().await;
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

        async fn abort(mut self) -> Vec<CapturedSocks5Connection> {
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

    async fn socks5_read_http_request(socket: &mut TcpStream) -> Option<CapturedRequest> {
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
        let headers = lines
            .filter_map(|line| {
                let (name, value) = line.split_once(':')?;
                Some((name.trim().to_string(), value.trim().to_string()))
            })
            .collect();
        Some(CapturedRequest {
            request_line,
            headers,
        })
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
                let mut port = [0u8; 2];
                socket.read_exact(&mut ipv4).await.ok()?;
                socket.read_exact(&mut port).await.ok()?;
                MockSocks5Target::Ipv4(ipv4, u16::from_be_bytes(port))
            }
            0x03 => {
                let mut len = [0u8; 1];
                socket.read_exact(&mut len).await.ok()?;
                let mut domain = vec![0u8; usize::from(len[0])];
                let mut port = [0u8; 2];
                socket.read_exact(&mut domain).await.ok()?;
                socket.read_exact(&mut port).await.ok()?;
                MockSocks5Target::Domain(String::from_utf8(domain).ok()?, u16::from_be_bytes(port))
            }
            0x04 => {
                let mut ipv6 = [0u8; 16];
                let mut port = [0u8; 2];
                socket.read_exact(&mut ipv6).await.ok()?;
                socket.read_exact(&mut port).await.ok()?;
                MockSocks5Target::Ipv6(ipv6, u16::from_be_bytes(port))
            }
            _ => return None,
        };

        socket
            .write_all(&[0x05, 0x00, 0x00, 0x01, 127, 0, 0, 1, 0, 0])
            .await
            .ok()?;
        socket.flush().await.ok()?;
        Some(target)
    }

    #[tokio::test]
    async fn proxy_http_proxy_routing_and_precedence() {
        // 1. Direct HTTP_PROXY routing; 2. ALL_PROXY fallback; 3. HTTP_PROXY overrides ALL_PROXY.
        for (label, use_http_proxy, use_all_proxy, filename) in [
            ("http-proxy-route", true, false, "test.bin"),
            ("all-proxy-fallback", false, true, "fallback.bin"),
            ("http-proxy-precedence", true, true, "precedence.bin"),
        ] {
            let payload = format!("payload for {label}").into_bytes();
            let primary = MockServer::start(vec![MockResponse::ok(&payload)]).await;
            let primary_url = format!("http://127.0.0.1:{}", primary.port());
            let unused = if use_http_proxy && use_all_proxy {
                Some(MockServer::start(vec![MockResponse::ok(b"wrong-proxy")]).await)
            } else {
                None
            };
            let unused_url = unused
                .as_ref()
                .map(|s| format!("http://127.0.0.1:{}", s.port()));

            let mut vars = Vec::new();
            if use_http_proxy {
                vars.push(("HTTP_PROXY", primary_url.as_str()));
                if let Some(ref url) = unused_url {
                    vars.push(("ALL_PROXY", url.as_str()));
                }
            } else if use_all_proxy {
                vars.push(("ALL_PROXY", primary_url.as_str()));
            }

            let fx = DownloadFixture::with_client(label, proxy_client(&vars));
            let target_url = format!("http://archive.example/download/item/{filename}");
            let outcome = fx
                .download(&target_url, filename, Some(&md5_hex(&payload)), None)
                .await
                .unwrap_or_else(|err| panic!("case {label} failed: {err}"));

            assert_eq!(outcome, FileOutcome::Downloaded, "{label}");
            let requests = primary.finish().await;
            assert_eq!(requests.len(), 1, "{label}");
            assert_eq!(
                requests[0].request_line,
                format!("GET {target_url} HTTP/1.1"),
                "{label}"
            );
            if let Some(unused_server) = unused {
                assert!(
                    unused_server.abort().await.is_empty(),
                    "ALL_PROXY must not be contacted when HTTP_PROXY matches"
                );
            }
            fx.assert_downloaded(filename, &payload);
        }
    }

    #[tokio::test]
    async fn proxy_https_proxy_sends_connect_without_external_network() {
        let proxy_server =
            MockServer::start(vec![MockResponse::status("502 Bad Gateway", b"")]).await;
        let proxy_url = format!("http://127.0.0.1:{}", proxy_server.port());

        let fx = DownloadFixture::with_client(
            "https-proxy-connect",
            proxy_client(&[("HTTPS_PROXY", &proxy_url)]),
        );
        let policy = fast_retry_policy(0);

        let result = fx
            .download_with_policy(
                "https://archive.example/download/item/test.bin",
                "test.bin",
                None,
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
    async fn proxy_no_proxy_bypasses_http_and_socks5_proxies() {
        // 1. NO_PROXY bypasses HTTP_PROXY
        let http_payload = b"served directly from local mock origin".to_vec();
        let origin_http = MockServer::start(vec![MockResponse::ok(&http_payload)]).await;
        let http_proxy = MockServer::start(vec![MockResponse::ok(b"wrong-from-proxy")]).await;
        let http_proxy_url = format!("http://127.0.0.1:{}", http_proxy.port());
        let fx_http = DownloadFixture::with_client(
            "no-proxy-http-bypass",
            proxy_client(&[("HTTP_PROXY", &http_proxy_url), ("NO_PROXY", "127.0.0.1")]),
        );

        let outcome = fx_http
            .download(
                &origin_http.url("/direct.bin"),
                "direct.bin",
                Some(&md5_hex(&http_payload)),
                None,
            )
            .await
            .expect("NO_PROXY bypassed HTTP download should succeed");
        assert_eq!(outcome, FileOutcome::Downloaded);
        let origin_reqs = origin_http.finish().await;
        assert_eq!(origin_reqs.len(), 1);
        assert_eq!(origin_reqs[0].request_line, "GET /direct.bin HTTP/1.1");
        assert!(http_proxy.abort().await.is_empty());
        fx_http.assert_downloaded("direct.bin", &http_payload);

        // 2. NO_PROXY bypasses SOCKS5 ALL_PROXY
        let socks_payload = b"origin payload bypassing SOCKS5 proxy".to_vec();
        let origin_socks = MockServer::start(vec![MockResponse::ok(&socks_payload)]).await;
        let socks_proxy = MockSocks5Server::start(vec![b"wrong-from-socks5".to_vec()]).await;
        let socks_url = socks_proxy.proxy_url("socks5");
        let fx_socks = DownloadFixture::with_client(
            "no-proxy-socks5-bypass",
            proxy_client(&[("ALL_PROXY", &socks_url), ("NO_PROXY", "127.0.0.1")]),
        );

        let outcome = fx_socks
            .download(
                &origin_socks.url("/bypass-socks.bin"),
                "bypass-socks.bin",
                Some(&md5_hex(&socks_payload)),
                None,
            )
            .await
            .expect("NO_PROXY bypassed SOCKS5 download should succeed");
        assert_eq!(outcome, FileOutcome::Downloaded);
        let origin_reqs = origin_socks.finish().await;
        assert_eq!(origin_reqs.len(), 1);
        assert_eq!(
            origin_reqs[0].request_line,
            "GET /bypass-socks.bin HTTP/1.1"
        );
        assert!(socks_proxy.abort().await.is_empty());
        fx_socks.assert_downloaded("bypass-socks.bin", &socks_payload);
    }

    #[tokio::test]
    async fn proxy_all_proxy_socks5_and_socks5h_tunnel_downloads() {
        let cases = [
            (
                "socks5",
                "http://127.0.0.1:80/socks5.bin",
                "socks5.bin",
                MockSocks5Target::Ipv4([127, 0, 0, 1], 80),
                "GET /socks5.bin HTTP/1.1",
                None,
            ),
            (
                "socks5h",
                "http://archive.example/socks5h.bin",
                "socks5h.bin",
                MockSocks5Target::Domain("archive.example".to_string(), 80),
                "GET /socks5h.bin HTTP/1.1",
                Some("archive.example"),
            ),
        ];

        for (scheme, url, filename, expected_target, expected_line, expected_host) in cases {
            let payload = format!("tunneled through {scheme} proxy").into_bytes();
            let socks_server = MockSocks5Server::start(vec![payload.clone()]).await;
            let socks_url = socks_server.proxy_url(scheme);
            let fx = DownloadFixture::with_client(
                &format!("{scheme}-tunnel"),
                proxy_client(&[("ALL_PROXY", &socks_url)]),
            );

            let outcome = fx
                .download(url, filename, Some(&md5_hex(&payload)), None)
                .await
                .unwrap_or_else(|err| panic!("{scheme} tunneled download failed: {err}"));

            assert_eq!(outcome, FileOutcome::Downloaded, "{scheme}");
            let connections = socks_server.finish().await;
            assert_eq!(connections.len(), 1, "{scheme}");
            assert_eq!(connections[0].target, expected_target, "{scheme}");
            assert_eq!(
                connections[0].request.request_line, expected_line,
                "{scheme}"
            );
            if let Some(host) = expected_host {
                assert_eq!(
                    connections[0].request.header("Host"),
                    Some(host),
                    "{scheme}"
                );
            }
            fx.assert_downloaded(filename, &payload);
        }
    }

    #[tokio::test]
    async fn test_verbose_logging_redirect_and_cookie_redaction_with_sentinel_secret() {
        const SENTINEL_NAME: &str = "ia_sentinel_cookie_name";
        const SENTINEL_VALUE: &str = "SUPER_SECRET_SENTINEL_9f8e7d6c";

        let payload = b"redirected payload after protected hop".to_vec();
        let server = MockServer::start(vec![
            MockResponse::redirect("302 Found", "/public/final.bin"),
            MockResponse::ok(&payload),
        ])
        .await;

        let netscape =
            format!("127.0.0.1\tFALSE\t/protected/\tFALSE\t0\t{SENTINEL_NAME}\t{SENTINEL_VALUE}\n");
        let cookie_source = CookieSource::from_netscape_str(&netscape);

        let fx = DownloadFixture::new("verbose-redirect-cookie");
        let options = DownloadOptions {
            retry_policy: fast_retry_policy(0),
            limit_rate: None,
            verbose: true,
        };
        let captured = CapturedLogs::new();
        let logger = VerboseLogger::with_writer(captured.clone_writer());

        let start_url = server.url("/protected/start.bin");
        let final_url = server.url("/public/final.bin");

        let outcome = download_single_file_with_options(
            &fx.client,
            fx.path(),
            &start_url,
            "final.bin",
            Some(&md5_hex(&payload)),
            &fx.signal,
            Some(&cookie_source),
            &options,
            &logger,
        )
        .await
        .expect("redirected download with scoped cookie should succeed");
        assert_eq!(outcome, FileOutcome::Downloaded);
        fx.assert_downloaded("final.bin", &payload);

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

        assert!(
            !captured.as_bytes().contains(&0x1b),
            "verbose logs must contain zero ANSI escape bytes"
        );

        let logs = captured.text();
        assert!(
            !logs.contains(SENTINEL_NAME),
            "verbose logs must never leak cookie name:\n{logs}"
        );
        assert!(
            !logs.contains(SENTINEL_VALUE),
            "verbose logs must never leak cookie value:\n{logs}"
        );
        for expected in [
            format!("[verbose] http: -> GET {start_url} (cookies=yes)"),
            format!("[verbose] http: <- 302 Found GET {start_url} (cookies=yes)"),
            format!("[verbose] redirect: 302 Found {start_url} -> {final_url} (hop 1/"),
            format!("[verbose] http: -> GET {final_url} (cookies=no)"),
            format!("[verbose] http: <- 200 OK GET {final_url} (cookies=no)"),
        ] {
            assert!(
                logs.contains(&expected),
                "missing expected log '{expected}':\n{logs}"
            );
        }
    }

    #[tokio::test]
    async fn test_verbose_logging_retries_with_retry_after_and_backoff() {
        let payload = b"recovered after 429 and 503".to_vec();
        let server = MockServer::start(vec![
            MockResponse::retry_after("429 Too Many Requests", "0", b""),
            MockResponse::status("503 Service Unavailable", b""),
            MockResponse::ok(&payload),
        ])
        .await;

        let fx = DownloadFixture::new("verbose-retries");
        let options = DownloadOptions {
            retry_policy: fast_retry_policy(3),
            limit_rate: None,
            verbose: true,
        };
        let captured = CapturedLogs::new();
        let logger = VerboseLogger::with_writer(captured.clone_writer());
        let url = server.url("/flaky.bin");

        let outcome = download_single_file_with_options(
            &fx.client,
            fx.path(),
            &url,
            "flaky.bin",
            Some(&md5_hex(&payload)),
            &fx.signal,
            None,
            &options,
            &logger,
        )
        .await
        .expect("download should recover after 429 and 503");
        assert_eq!(outcome, FileOutcome::Downloaded);
        fx.assert_downloaded("flaky.bin", &payload);
        assert_eq!(server.finish().await.len(), 3);

        assert!(
            !captured.as_bytes().contains(&0x1b),
            "retry verbose logs must contain zero ANSI escape bytes"
        );

        let logs = captured.text();
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
        let fresh_payload = b"freshly fetched file".to_vec();

        let server = MockServer::start(vec![
            MockResponse::ok(&fresh_payload),
            MockResponse::status("404 Not Found", b""),
        ])
        .await;

        let fx = DownloadFixture::new("verbose-batch-mixed");
        fx.write("existing.bin", &existing_payload);

        let options = DownloadOptions {
            retry_policy: fast_retry_policy(0),
            limit_rate: None,
            verbose: true,
        };
        let captured = CapturedLogs::new();
        let logger = VerboseLogger::with_writer(captured.clone_writer());

        let batch = vec![
            (
                server.url("/existing.bin"),
                "existing.bin".to_string(),
                Some(md5_hex(&existing_payload)),
            ),
            (
                server.url("/fresh.bin"),
                "fresh.bin".to_string(),
                Some(md5_hex(&fresh_payload)),
            ),
            (server.url("/missing.bin"), "missing.bin".to_string(), None),
        ];

        let summary = download_files_in_dir_with_logger(
            &fx.client,
            fx.path(),
            batch,
            3,
            None,
            &options,
            &fx.signal,
            &logger,
        )
        .await
        .expect("mixed batch should return summary");

        assert_eq!(summary.ok, 1);
        assert_eq!(summary.skipped, 1);
        assert_eq!(summary.failed.len(), 1);
        assert_eq!(summary.failed[0].file_path, "missing.bin");
        assert!(!summary.interrupted);
        fx.assert_downloaded("fresh.bin", &fresh_payload);

        let requests = server.finish().await;
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].request_line, "GET /fresh.bin HTTP/1.1");
        assert_eq!(requests[1].request_line, "GET /missing.bin HTTP/1.1");

        assert!(
            !captured.as_bytes().contains(&0x1b),
            "batch verbose logs must contain zero ANSI escape bytes"
        );

        let logs = captured.text();
        for expected in [
            "[verbose] download: skipped 'existing.bin': already exists; MD5 verified",
            "[verbose] download: completed 'fresh.bin' (2/3)",
            "[verbose] summary: ok=1 skipped=1 failed=1 interrupted=false",
        ] {
            assert!(logs.contains(expected), "missing '{expected}':\n{logs}");
        }
        assert!(
            logs.lines().any(|line| {
                line.starts_with("[verbose] download: failed 'missing.bin': ")
                    && line.contains("404")
            }),
            "missing failure reason for missing.bin:\n{logs}"
        );
    }

    #[test]
    fn test_verbose_logging_disabled_by_default() {
        let default_options = DownloadOptions::default();
        assert!(
            !default_options.verbose,
            "DownloadOptions::default() must disable verbose logging"
        );
        assert!(!VerboseLogger::from_enabled(default_options.verbose).is_enabled());
        assert!(!VerboseLogger::disabled().is_enabled());
    }

    #[test]
    fn test_validate_download_paths_exact_and_case_sensitivity_modes() {
        // Exact mode: final duplicates, normalised slash duplicates, staging collisions in both orders
        for (label, first, second, expected_local, check_entries) in [
            (
                "dup-sanitised",
                ("foo?.txt", "foo_.txt"),
                ("foo*.txt", "foo_.txt"),
                "foo_.txt",
                true,
            ),
            (
                "normalised-slash",
                ("a/file.txt", "a/file.txt"),
                ("a//file.txt", "a//file.txt"),
                "a/file.txt",
                false,
            ),
            (
                "staging-after",
                ("foo.txt", "foo.txt"),
                ("foo.txt.part", "foo.txt.part"),
                "foo.txt.part",
                true,
            ),
            (
                "staging-before",
                ("foo.txt.part", "foo.txt.part"),
                ("foo.txt", "foo.txt"),
                "foo.txt.part",
                true,
            ),
        ] {
            let err = validate_download_paths([first, second]).unwrap_err();
            assert_path_collision(
                err,
                expected_local,
                check_entries.then_some(first.0),
                check_entries.then_some(second.0),
                None,
                label,
            );
        }

        assert!(validate_download_paths(same_paths(&[
            "foo.txt",
            "foo.txt.bak",
            "a/foo.txt",
            "b/foo.txt",
            "README.md",
            "readme.md",
        ]))
        .is_ok());

        // Case-insensitive mode: filename/dir/staging/Unicode/sanitised case collisions
        let ci_cases = [
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

        for (label, first, second, expected_local) in ci_cases {
            let err = validate_download_paths_with_case_sensitivity(
                [first, second],
                PathCaseSensitivity::CaseInsensitive,
            )
            .unwrap_err();
            assert_path_collision(
                err,
                expected_local,
                Some(first.0),
                Some(second.0),
                Some("case-insensitive"),
                label,
            );
        }

        // Exact duplicates still report the exact reason even in case-insensitive mode.
        let exact_err = validate_download_paths_with_case_sensitivity(
            [("foo?.txt", "foo_.txt"), ("foo*.txt", "foo_.txt")],
            PathCaseSensitivity::CaseInsensitive,
        )
        .unwrap_err();
        assert_path_collision(
            exact_err,
            "foo_.txt",
            None,
            None,
            Some("both entries map to the same destination path"),
            "exact-in-ci",
        );

        // Case-sensitive mode allows all case-only differences.
        assert!(validate_download_paths_with_case_sensitivity(
            same_paths(&[
                "Foo.txt",
                "foo.txt",
                "Dir/a.txt",
                "dir/A.txt",
                "a.txt",
                "A.TXT.part",
                "a.txt.PART",
                "Caf\u{00e9}.txt",
                "CAF\u{00c9}.txt",
            ]),
            PathCaseSensitivity::CaseSensitive,
        )
        .is_ok());
    }

    #[tokio::test]
    async fn test_download_files_in_dir_pre_write_validation_aborts_before_http_requests() {
        let client = http_client_builder().build().unwrap();
        let signal = ShutdownSignal::new();

        // 1. Colliding final/staging paths abort before creating output dir or sending HTTP.
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
            assert!(
                matches!(err, IaGetError::PathCollision { .. }),
                "case {label}: expected PathCollision, got {err:?}"
            );
            assert!(
                server.abort().await.is_empty(),
                "case {label}: expected 0 HTTP requests on collision"
            );
            assert!(
                !target_dir.exists(),
                "case {label}: colliding batch must fail before creating output directory"
            );
        }

        // 2. Fail-closed unprobable overlong Unicode candidates abort before any HTTP transfer.
        let sandbox = TempTestDir::new("failclosed-aborts-transfers");
        let server = MockServer::start(vec![
            MockResponse::ok(b"must never be requested"),
            MockResponse::ok(b"must never be requested"),
        ])
        .await;
        let long_nfc = format!("Caf\u{00e9}-{}", "a".repeat(300));
        let long_nfd = format!("Cafe\u{0301}-{}", "a".repeat(300));
        let err = download_files_in_dir(
            &client,
            sandbox.path(),
            vec![
                (server.url("/nfc"), long_nfc, None),
                (server.url("/nfd"), long_nfd, None),
            ],
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
        assert!(
            server.abort().await.is_empty(),
            "fail-closed validation error must abort before any HTTP transfer requests"
        );
        assert_no_probe_dirs_remaining(sandbox.path());
    }

    #[test]
    fn test_detect_path_case_sensitivity_and_validate_for_dir_match_host_filesystem() {
        let oracle_dir = TempTestDir::new("case-oracle");
        let expected = measure_host_dir_case_sensitivity(oracle_dir.path());

        // 1. Empty existing directory (exercises temp probe file + cleanup).
        let empty_dir = TempTestDir::new("case-empty");
        assert_eq!(detect_path_case_sensitivity(empty_dir.path()), expected);
        assert_eq!(fs::read_dir(empty_dir.path()).unwrap().count(), 0);

        // 2. Non-empty existing directory (exercises read-only child probe).
        let populated_dir = TempTestDir::new("case-populated");
        populated_dir.write("Existing.txt", b"data");
        assert_eq!(detect_path_case_sensitivity(populated_dir.path()), expected);
        assert_eq!(fs::read_dir(populated_dir.path()).unwrap().count(), 1);

        // 3. Not-yet-created nested directory resolves nearest existing ancestor without creating it.
        let missing_dir = empty_dir.join("not-created-yet/nested");
        assert_eq!(detect_path_case_sensitivity(&missing_dir), expected);
        assert!(!empty_dir.join("not-created-yet").exists());

        // 4. Read-only directory with an existing child still probes cleanly on Unix.
        #[cfg(unix)]
        {
            let ro_dir = TempTestDir::new("case-readonly");
            ro_dir.write("Sample.txt", b"ro");
            let detected = with_readonly_dir(ro_dir.path(), || {
                detect_path_case_sensitivity(ro_dir.path())
            });
            assert_eq!(detected, expected);
        }

        // 5. validate_download_paths_for_dir matches host sensitivity for case-only pairs.
        for pair in [["Foo.txt", "foo.txt"], ["a.txt", "A.TXT.part"]] {
            let result = validate_download_paths_for_dir(&missing_dir, same_paths(&pair));
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
        assert!(!empty_dir.join("not-created-yet").exists());
    }

    #[tokio::test]
    async fn test_download_files_in_dir_case_only_entries_follow_host_filesystem() {
        let sandbox = TempTestDir::new("case-download-host");
        let expected = measure_host_dir_case_sensitivity(sandbox.path());
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
                Some(md5_hex(payload_upper)),
            ),
            (
                server.url("/lower"),
                "foo.txt".to_string(),
                Some(md5_hex(payload_lower)),
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
                assert!(matches!(
                    result.unwrap_err(),
                    IaGetError::PathCollision { .. }
                ));
                assert!(server.abort().await.is_empty());
                assert!(!target_dir.exists());
            }
            PathCaseSensitivity::CaseSensitive => {
                let summary = result.expect("case-sensitive volume should download both files");
                assert_eq!(summary.ok, 2);
                assert!(summary.failed.is_empty());
                assert_downloaded_file(&target_dir, "Foo.txt", payload_upper);
                assert_downloaded_file(&target_dir, "foo.txt", payload_lower);
                assert_eq!(server.finish().await.len(), 2);
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
    fn test_per_parent_lookup_mixed_mounts_and_merged_parents() {
        // 1. Case-insensitive subdir under case-sensitive root
        let cs_root = MockDirectoryLookup::new([
            ("/root", MockDirRule::Exact),
            ("/root/sub", MockDirRule::CaseInsensitiveAscii),
        ]);
        let root = Path::new("/root");

        assert!(validate_download_paths_with_lookup(
            root,
            &same_paths(&["Foo.txt", "foo.txt"]),
            &cs_root
        )
        .is_ok());
        assert_eq!(cs_root.probed(), vec![PathBuf::from("/root")]);

        for (label, first, second, expected_local) in [
            (
                "final-vs-final",
                "sub/Foo.txt",
                "sub/foo.txt",
                "sub/foo.txt",
            ),
            (
                "staging-vs-final",
                "sub/a.txt",
                "sub/A.TXT.part",
                "sub/A.TXT.part",
            ),
            (
                "final-vs-staging",
                "sub/A.TXT.part",
                "sub/a.txt",
                "sub/a.txt.part",
            ),
            (
                "missing-nested-under-submount",
                "sub/newdir/Foo.txt",
                "sub/newdir/foo.txt",
                "sub/newdir/foo.txt",
            ),
        ] {
            cs_root.clear_probed();
            let err =
                validate_download_paths_with_lookup(root, &same_paths(&[first, second]), &cs_root)
                    .unwrap_err();
            assert_path_collision(
                err,
                expected_local,
                Some(first),
                Some(second),
                Some("filesystem lookup rules"),
                label,
            );
            assert_eq!(
                cs_root.probed(),
                vec![PathBuf::from("/root/sub")],
                "case {label}: expected probe inside /root/sub"
            );
        }

        // 2. Case-sensitive subdir under case-insensitive root (including merged parent spellings)
        let ci_root = MockDirectoryLookup::new([
            ("/root", MockDirRule::CaseInsensitiveAscii),
            ("/root/sub", MockDirRule::Exact),
        ]);

        let root_err = validate_download_paths_with_lookup(
            root,
            &same_paths(&["Foo.txt", "foo.txt"]),
            &ci_root,
        )
        .unwrap_err();
        assert_path_collision(root_err, "foo.txt", None, None, None, "ci-root");

        ci_root.clear_probed();
        assert!(validate_download_paths_with_lookup(
            root,
            &same_paths(&["sub/Foo.txt", "sub/foo.txt"]),
            &ci_root,
        )
        .is_ok());
        assert_eq!(ci_root.probed(), vec![PathBuf::from("/root/sub")]);

        // Equivalent parent spellings (`Sub` and `sub`) merge onto the existing case-sensitive `/root/sub`.
        ci_root.clear_probed();
        assert!(validate_download_paths_with_lookup(
            root,
            &same_paths(&["Sub/Foo.txt", "sub/foo.txt"]),
            &ci_root,
        )
        .is_ok());
        assert_eq!(
            ci_root.probed(),
            vec![PathBuf::from("/root"), PathBuf::from("/root/sub")]
        );

        let merged_leaf_err = validate_download_paths_with_lookup(
            root,
            &same_paths(&["Sub/Foo.txt", "sub/Foo.txt"]),
            &ci_root,
        )
        .unwrap_err();
        assert_path_collision(
            merged_leaf_err,
            "sub/Foo.txt",
            Some("Sub/Foo.txt"),
            Some("sub/Foo.txt"),
            Some("filesystem lookup rules"),
            "merged-parent-identical-leaf",
        );

        // 3. Merging case-equivalent parent subdirectories on a CI volume
        let ci = MockDirectoryLookup::new([("/ci", MockDirRule::CaseInsensitiveAscii)]);
        for (label, first, second, expected_local) in [
            (
                "dir-case-and-file-case",
                "Dir/a.txt",
                "dir/A.txt",
                "dir/A.txt",
            ),
            (
                "dir-case-and-staging-upper-part",
                "Dir/a.txt",
                "dir/a.txt.PART",
                "dir/a.txt.PART",
            ),
            (
                "dir-case-and-staging-before-final",
                "Dir/a.txt.PART",
                "dir/a.txt",
                "dir/a.txt.part",
            ),
        ] {
            let err = validate_download_paths_with_lookup(
                Path::new("/ci"),
                &same_paths(&[first, second]),
                &ci,
            )
            .unwrap_err();
            assert_path_collision(err, expected_local, Some(first), Some(second), None, label);
        }
        assert!(validate_download_paths_with_lookup(
            Path::new("/ci"),
            &same_paths(&["Dir/a.txt", "dir/b.txt"]),
            &ci,
        )
        .is_ok());
    }

    #[test]
    fn test_per_parent_lookup_unicode_nfc_nfd_and_folding_equivalence() {
        let lookup = MockDirectoryLookup::new([
            ("/apfs-cs", MockDirRule::NormalizationInsensitive),
            ("/ext4", MockDirRule::Exact),
            ("/apfs-ci", MockDirRule::CaseAndNormalizationInsensitive),
        ]);

        // Case-sensitive but normalisation-insensitive directory allows ASCII case differences.
        assert!(validate_download_paths_with_lookup(
            Path::new("/apfs-cs"),
            &same_paths(&["Foo.txt", "foo.txt"]),
            &lookup,
        )
        .is_ok());

        for (label, first, second, expected_local) in [
            (
                "nfc-vs-nfd-final",
                "Caf\u{00e9}.txt",
                "Cafe\u{0301}.txt",
                "Cafe\u{0301}.txt",
            ),
            (
                "nfc-vs-nfd-staging-after",
                "Caf\u{00e9}.txt",
                "Cafe\u{0301}.txt.part",
                "Cafe\u{0301}.txt.part",
            ),
            (
                "nfd-staging-before-nfc",
                "Cafe\u{0301}.txt.part",
                "Caf\u{00e9}.txt",
                "Caf\u{00e9}.txt.part",
            ),
            (
                "nfc-vs-nfd-parent-dir",
                "Dir_Caf\u{00e9}/a.txt",
                "Dir_Cafe\u{0301}/a.txt",
                "Dir_Cafe\u{0301}/a.txt",
            ),
        ] {
            let pair = same_paths(&[first, second]);
            let err = validate_download_paths_with_lookup(Path::new("/apfs-cs"), &pair, &lookup)
                .unwrap_err();
            assert_path_collision(
                err,
                expected_local,
                Some(first),
                Some(second),
                Some("filesystem lookup rules"),
                label,
            );
            assert!(
                validate_download_paths_with_lookup(Path::new("/ext4"), &pair, &lookup).is_ok(),
                "case {label}: normalisation-sensitive directory should allow {pair:?}"
            );
        }

        // Equivalent NFC/NFD parent directories with distinct child filenames are allowed.
        assert!(validate_download_paths_with_lookup(
            Path::new("/apfs-cs"),
            &same_paths(&["Dir_Caf\u{00e9}/a.txt", "Dir_Cafe\u{0301}/b.txt"]),
            &lookup,
        )
        .is_ok());

        // Non-ASCII entry colliding with a pure-ASCII entry under Unicode full case folding.
        let folding_err = validate_download_paths_with_lookup(
            Path::new("/apfs-ci"),
            &same_paths(&["Stra\u{00df}e.txt", "STRASSE.TXT"]),
            &lookup,
        )
        .unwrap_err();
        assert_path_collision(folding_err, "STRASSE.TXT", None, None, None, "eszett-fold");
    }

    #[test]
    fn test_per_parent_lookup_file_vs_directory_collisions() {
        let lookup = MockDirectoryLookup::new([
            ("/exact", MockDirRule::Exact),
            ("/ci", MockDirRule::CaseInsensitiveAscii),
        ]);

        // Exact file-vs-directory and staging-vs-directory collisions fail with zero probing.
        for (first, second, expected_local) in [
            ("foo", "foo/bar.txt", "foo"),
            ("foo/bar.txt", "foo", "foo"),
            ("foo", "foo.part/bar.txt", "foo.part"),
            ("foo.part/bar.txt", "foo", "foo.part"),
        ] {
            lookup.clear_probed();
            let err = validate_download_paths_with_lookup(
                Path::new("/exact"),
                &same_paths(&[first, second]),
                &lookup,
            )
            .unwrap_err();
            assert_path_collision(
                err,
                expected_local,
                Some(first),
                Some(second),
                None,
                "exact-fvd",
            );
            assert!(
                lookup.probed().is_empty(),
                "exact file-vs-dir collision must not require filesystem probing"
            );
        }

        // Case-only file-vs-directory collision is allowed on exact dir and rejected on CI dir.
        assert!(validate_download_paths_with_lookup(
            Path::new("/exact"),
            &same_paths(&["foo", "FOO/bar.txt"]),
            &lookup,
        )
        .is_ok());

        for (first, second, expected_local) in [
            ("foo", "FOO/bar.txt", "FOO"),
            ("FOO/bar.txt", "foo", "foo"),
            ("foo", "FOO.PART/bar.txt", "FOO.PART"),
        ] {
            let err = validate_download_paths_with_lookup(
                Path::new("/ci"),
                &same_paths(&[first, second]),
                &lookup,
            )
            .unwrap_err();
            assert_path_collision(
                err,
                expected_local,
                Some(first),
                Some(second),
                None,
                "ci-fvd",
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
            &same_paths(&["a.txt", "b.txt", "sub/c.txt", "sub/d.txt"]),
            &lookup,
        )
        .is_ok());
        assert!(lookup.probed().is_empty());

        // Single non-ASCII entry performs zero probing.
        assert!(validate_download_paths_with_lookup(
            Path::new("/ancestor"),
            &same_paths(&["Caf\u{00e9}.txt"]),
            &lookup,
        )
        .is_ok());
        assert!(lookup.probed().is_empty());

        // Missing output root (`/ancestor/missing`) must inherit `/ancestor`, not sibling `/ancestor/sub`.
        assert!(validate_download_paths_with_lookup(
            Path::new("/ancestor/missing"),
            &same_paths(&["sub/Foo.txt", "sub/foo.txt"]),
            &lookup,
        )
        .is_ok());
        assert_eq!(lookup.probed(), vec![PathBuf::from("/ancestor")]);
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

    fn assert_matches_host_case(result: &Result<()>, host_case: PathCaseSensitivity, ctx: &str) {
        match host_case {
            PathCaseSensitivity::CaseSensitive => {
                assert!(
                    result.is_ok(),
                    "{ctx}: CS host should allow, got {result:?}"
                )
            }
            PathCaseSensitivity::CaseInsensitive => assert!(
                matches!(result, Err(IaGetError::PathCollision { .. })),
                "{ctx}: CI host should reject, got {result:?}"
            ),
        }
    }

    fn assert_matches_host_nfc(result: &Result<()>, host_nfc_nfd: bool, ctx: &str) {
        if host_nfc_nfd {
            assert!(
                matches!(result, Err(IaGetError::PathCollision { .. })),
                "{ctx}: normalisation-insensitive host should reject, got {result:?}"
            );
        } else {
            assert!(
                result.is_ok(),
                "{ctx}: normalisation-sensitive host should allow, got {result:?}"
            );
        }
    }

    #[test]
    fn test_real_host_probe_unicode_cleanup_symlink_safety_readonly_and_overlong_fallback() {
        let sandbox = TempTestDir::new("real-host-per-parent-probe");
        let host_case = measure_host_dir_case_sensitivity(sandbox.path());
        let host_nfc_nfd = probe_host_dir_equivalent(
            sandbox.path(),
            "host-nfc-probe-caf\u{00e9}.txt",
            "host-nfc-probe-cafe\u{0301}.txt",
        );

        // 1. NFC vs NFD final and `.part` pairs match host semantics and leave no probe dirs.
        for pair in [
            ["Caf\u{00e9}.txt", "Cafe\u{0301}.txt"],
            ["Caf\u{00e9}.txt", "Cafe\u{0301}.txt.part"],
        ] {
            let result = validate_download_paths_for_dir(sandbox.path(), same_paths(&pair));
            assert_matches_host_nfc(&result, host_nfc_nfd, "real-host-nfc");
            assert_no_probe_dirs_remaining(sandbox.path());
        }

        // 2. Missing output dir is never created and leaves no probe dirs in its parent.
        let missing_dir = sandbox.join("never-created/nested");
        let case_res =
            validate_download_paths_for_dir(&missing_dir, same_paths(&["Foo.txt", "foo.txt"]));
        assert_matches_host_case(&case_res, host_case, "missing-dir");
        assert!(!sandbox.join("never-created").exists());
        assert_no_probe_dirs_remaining(sandbox.path());

        // 3. Overlong ASCII & Unicode names fail closed in both empty and populated dirs, and via prefixed probe.
        let long_lower = "a".repeat(300);
        let long_upper = "A".repeat(300);
        let long_nfc = format!("Caf\u{00e9}-{}", "a".repeat(300));
        let long_nfd = format!("Cafe\u{0301}-{}", "a".repeat(300));
        assert!(matches!(
            validate_download_paths_for_dir(
                sandbox.path(),
                same_paths(&[long_lower.as_str(), long_upper.as_str()])
            ),
            Err(IaGetError::FileSystem(_))
        ));
        assert_no_probe_dirs_remaining(sandbox.path());

        let populated = TempTestDir::new("overlong-populated-failclosed");
        populated.write("Sample.txt", b"existing");
        let ascii_pop = validate_download_paths_for_dir(
            populated.path(),
            same_paths(&[long_lower.as_str(), long_upper.as_str()]),
        );
        match host_case {
            PathCaseSensitivity::CaseSensitive => {
                assert!(matches!(ascii_pop, Err(IaGetError::FileSystem(_))));
            }
            PathCaseSensitivity::CaseInsensitive => {
                assert!(matches!(
                    ascii_pop,
                    Err(IaGetError::PathCollision { .. } | IaGetError::FileSystem(_))
                ));
            }
        }
        assert!(matches!(
            validate_download_paths_for_dir(
                populated.path(),
                same_paths(&[long_nfc.as_str(), long_nfd.as_str()])
            ),
            Err(IaGetError::FileSystem(_))
        ));
        let no_subdir_lookup = OsDirectoryLookup::without_temp_subdir_probe();
        assert!(matches!(
            validate_download_paths_with_lookup(
                populated.path(),
                &same_paths(&[long_nfc.as_str(), long_nfd.as_str()]),
                &no_subdir_lookup,
            ),
            Err(IaGetError::FileSystem(_))
        ));
        assert_no_probe_dirs_remaining(populated.path());

        // 4. Symlinks inside output_dir are never followed for probing; read-only dirs preserve
        // zero-I/O fast path and fail closed when candidates cannot be verified.
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            let external_target = TempTestDir::new("symlink-probe-external");
            symlink(external_target.path(), sandbox.join("linked_sub")).unwrap();
            let link_res = with_readonly_dir(external_target.path(), || {
                validate_download_paths_for_dir(
                    sandbox.path(),
                    same_paths(&["linked_sub/Foo.txt", "linked_sub/foo.txt"]),
                )
            });
            assert_matches_host_case(&link_res, host_case, "symlink-external");
            assert_eq!(fs::read_dir(external_target.path()).unwrap().count(), 0);
            assert_no_probe_dirs_remaining(sandbox.path());

            // Populated read-only dir: zero-I/O ASCII ok, case-only via child probe, Unicode & trailing-dot fail closed.
            let ro_populated = TempTestDir::new("ro-probe-populated");
            ro_populated.write("Sample.txt", b"ro");
            let (pop_distinct, pop_case, pop_unicode, pop_dot) =
                with_readonly_dir(ro_populated.path(), || {
                    (
                        validate_download_paths_for_dir(
                            ro_populated.path(),
                            same_paths(&["alpha.txt", "beta.txt"]),
                        ),
                        validate_download_paths_for_dir(
                            ro_populated.path(),
                            same_paths(&["Foo.txt", "foo.txt"]),
                        ),
                        validate_download_paths_for_dir(
                            ro_populated.path(),
                            same_paths(&["Caf\u{00e9}.txt", "Cafe\u{0301}.txt"]),
                        ),
                        validate_download_paths_for_dir(
                            ro_populated.path(),
                            same_paths(&["foo.", "foo"]),
                        ),
                    )
                });
            assert!(pop_distinct.is_ok());
            assert_matches_host_case(&pop_case, host_case, "ro-populated-case");
            assert!(matches!(pop_unicode, Err(IaGetError::FileSystem(_))));
            assert!(matches!(pop_dot, Err(IaGetError::FileSystem(_))));
            assert_no_probe_dirs_remaining(ro_populated.path());

            // Empty read-only dir: zero-I/O ASCII ok; case-only and Unicode fail closed.
            let ro_empty = TempTestDir::new("ro-probe-empty");
            let (empty_distinct, empty_case, empty_unicode) =
                with_readonly_dir(ro_empty.path(), || {
                    (
                        validate_download_paths_for_dir(
                            ro_empty.path(),
                            same_paths(&["alpha.txt", "beta.txt"]),
                        ),
                        validate_download_paths_for_dir(
                            ro_empty.path(),
                            same_paths(&["Foo.txt", "foo.txt"]),
                        ),
                        validate_download_paths_for_dir(
                            ro_empty.path(),
                            same_paths(&["Caf\u{00e9}.txt", "Cafe\u{0301}.txt"]),
                        ),
                    )
                });
            assert!(empty_distinct.is_ok());
            assert!(matches!(empty_case, Err(IaGetError::FileSystem(_))));
            assert!(matches!(empty_unicode, Err(IaGetError::FileSystem(_))));
            assert_no_probe_dirs_remaining(ro_empty.path());
        }
    }

    #[test]
    fn test_prefixed_file_probe_fallback_when_temp_subdir_unavailable() {
        let sandbox = TempTestDir::new("prefixed-probe-fallback");
        let host_case = measure_host_dir_case_sensitivity(sandbox.path());
        let host_nfc_nfd = probe_host_dir_equivalent(
            sandbox.path(),
            "host-nfc-probe-caf\u{00e9}.txt",
            "host-nfc-probe-cafe\u{0301}.txt",
        );
        let no_subdir_lookup = OsDirectoryLookup::without_temp_subdir_probe();

        // 1. ASCII case-only pair matches host semantics when temp subdir creation is unavailable.
        let case_res = validate_download_paths_with_lookup(
            sandbox.path(),
            &same_paths(&["Foo.txt", "foo.txt"]),
            &no_subdir_lookup,
        );
        assert_matches_host_case(&case_res, host_case, "prefixed-case");
        assert_no_probe_dirs_remaining(sandbox.path());

        // 2. Unicode NFC vs NFD final and `.part` pairs match host semantics via prefixed files.
        for pair in [
            ["Caf\u{00e9}.txt", "Cafe\u{0301}.txt"],
            ["Caf\u{00e9}.txt", "Cafe\u{0301}.txt.part"],
        ] {
            let result = validate_download_paths_with_lookup(
                sandbox.path(),
                &same_paths(&pair),
                &no_subdir_lookup,
            );
            assert_matches_host_nfc(&result, host_nfc_nfd, "prefixed-nfc");
            assert_no_probe_dirs_remaining(sandbox.path());
        }

        // 3. Direct prefixed probe verifies collision detection and distinctness with clean teardown.
        let colliding_names = vec!["same.txt".to_string(), "same.txt".to_string()];
        assert_eq!(
            probe_prefixed_names_in_dir(sandbox.path(), &colliding_names, &[0, 1]),
            PrefixedProbeOutcome::Resolved(vec![0, 0])
        );
        assert_no_probe_dirs_remaining(sandbox.path());

        let distinct_names = vec!["alpha.txt".to_string(), "beta.txt".to_string()];
        assert_eq!(
            probe_prefixed_names_in_dir(sandbox.path(), &distinct_names, &[0, 1]),
            PrefixedProbeOutcome::Resolved(vec![0, 1])
        );
        assert_no_probe_dirs_remaining(sandbox.path());

        // 4. Pre-existing foreign file matching the first probe filename is never removed or mistaken.
        let pid = std::process::id();
        let initial_seq: u64 = 900_000_000;
        let foreign_path = sandbox.write(
            format!(".ia-get-fs-probe-{pid}-{initial_seq}-beta.txt"),
            b"pre-existing-foreign-file",
        );

        let mut seq = initial_seq;
        let foreign_res =
            probe_prefixed_names_in_dir_with_seq(sandbox.path(), &distinct_names, &[0, 1], || {
                let current = seq;
                seq += 1;
                current
            });
        assert_eq!(foreign_res, PrefixedProbeOutcome::Resolved(vec![0, 1]));
        assert_eq!(seq, initial_seq + 2);
        assert_eq!(
            fs::read(&foreign_path).unwrap(),
            b"pre-existing-foreign-file"
        );
        fs::remove_file(&foreign_path).unwrap();
        assert_no_probe_dirs_remaining(sandbox.path());
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
            &same_paths(&["alpha.txt", "beta.txt"]),
            &lookup,
        )
        .is_ok());
        assert!(lookup.probed().is_empty());

        // Unicode and case-only candidates fail closed when the probe is inconclusive.
        for entries in [
            ["Caf\u{00e9}.txt", "Cafe\u{0301}.txt"],
            ["Foo.txt", "foo.txt"],
        ] {
            lookup.clear_probed();
            let err = validate_download_paths_with_lookup(
                Path::new("/unverifiable"),
                &same_paths(&entries),
                &lookup,
            )
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
            &same_paths(&[
                "unverifiable/Caf\u{00e9}.txt",
                "unverifiable/Cafe\u{0301}.txt",
            ]),
            &lookup,
        )
        .unwrap_err();
        assert!(
            matches!(subdir_err, IaGetError::FileSystem(ref msg) if msg.contains("/exact/unverifiable")),
            "inconclusive child directory probe must fail closed, got {subdir_err:?}"
        );
        assert_eq!(lookup.probed(), vec![PathBuf::from("/exact/unverifiable")]);
    }
}
