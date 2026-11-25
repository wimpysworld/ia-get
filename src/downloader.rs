//! Module for handling file downloads, verification, and related operations.

use std::collections::hash_map::RandomState;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::future::Future;
use std::hash::BuildHasher;
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use colored::*;
use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::{Client, RequestBuilder, Response, StatusCode};
use tokio::sync::Notify;

use crate::constants::{
    INITIAL_RETRY_DELAY_MS, MAX_RETRIES, MAX_RETRY_AFTER_SECS, MAX_RETRY_DELAY_SECS,
};
use crate::error::IaGetError; // Import IaGetError for explicit error conversion
use crate::utils::{create_progress_bar, format_duration, format_size, format_transfer_rate};
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

/// Computes the retry delay for a 1-based `attempt`, preferring a valid `Retry-After` header
/// and falling back to jittered exponential backoff otherwise.
pub fn compute_retry_delay(
    attempt: u32,
    retry_after_header: Option<&str>,
    now: SystemTime,
    policy: &RetryPolicy,
) -> Duration {
    if let Some(delay) =
        retry_after_header.and_then(|value| parse_retry_after(value, now, policy.max_retry_after))
    {
        return delay;
    }

    compute_backoff_delay_with_sample(attempt, policy, random_u64())
}

enum AttemptOutcome {
    Ready(Response),
    Retry { delay: Duration, reason: String },
}

async fn send_attempt<F>(
    build_request: &mut F,
    retry_count: u32,
    policy: &RetryPolicy,
    signal: &ShutdownSignal,
) -> Result<AttemptOutcome>
where
    F: FnMut() -> RequestBuilder,
{
    let send_result = await_interruptible(build_request().send(), signal).await?;

    match send_result {
        Ok(response) => {
            let status = response.status();
            if is_retryable_status(status) {
                if retry_count >= policy.max_retries {
                    return match response.error_for_status() {
                        Ok(resp) => Ok(AttemptOutcome::Ready(resp)),
                        Err(err) => Err(err.into()),
                    };
                }

                let retry_after = response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|value| value.to_str().ok());
                let delay =
                    compute_retry_delay(retry_count + 1, retry_after, SystemTime::now(), policy);
                let reason = format!("HTTP {}", status);
                drop(response);
                Ok(AttemptOutcome::Retry { delay, reason })
            } else {
                let response = response.error_for_status()?;
                Ok(AttemptOutcome::Ready(response))
            }
        }
        Err(err) => {
            if !is_retryable_reqwest_error(&err) || retry_count >= policy.max_retries {
                return Err(err.into());
            }

            let delay = compute_retry_delay(retry_count + 1, None, SystemTime::now(), policy);
            let reason = err.to_string();
            Ok(AttemptOutcome::Retry { delay, reason })
        }
    }
}

/// Sends an HTTP request with retries on transient transport errors and retryable HTTP statuses,
/// honouring `Retry-After` headers and immediate cancellation via `ShutdownSignal`.
pub async fn send_with_retry<F, H>(
    policy: &RetryPolicy,
    signal: &ShutdownSignal,
    mut build_request: F,
    mut on_retry: H,
) -> Result<Response>
where
    F: FnMut() -> RequestBuilder,
    H: FnMut(&RetryEvent<'_>),
{
    let mut retry_count: u32 = 0;

    loop {
        match send_attempt(&mut build_request, retry_count, policy, signal).await? {
            AttemptOutcome::Ready(response) => return Ok(response),
            AttemptOutcome::Retry { delay, reason } => {
                retry_count += 1;
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
/// statuses, and transient body read errors, honouring `Retry-After` and `ShutdownSignal`.
pub async fn fetch_text_with_retry<F, H>(
    policy: &RetryPolicy,
    signal: &ShutdownSignal,
    mut build_request: F,
    mut on_retry: H,
) -> Result<String>
where
    F: FnMut() -> RequestBuilder,
    H: FnMut(&RetryEvent<'_>),
{
    let mut retry_count: u32 = 0;

    loop {
        match send_attempt(&mut build_request, retry_count, policy, signal).await? {
            AttemptOutcome::Retry { delay, reason } => {
                retry_count += 1;
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
                        if !is_retryable_reqwest_error(&err) || retry_count >= policy.max_retries {
                            return Err(err.into());
                        }
                        retry_count += 1;
                        let delay =
                            compute_retry_delay(retry_count, None, SystemTime::now(), policy);
                        let reason = err.to_string();
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
fn resolve_destination_root(root: &Path) -> Result<PathBuf> {
    let canonical = fs::canonicalize(root)?;
    if !fs::metadata(&canonical)?.is_dir() {
        return Err(IaGetError::FileSystem(format!(
            "Destination root '{}' is not a directory",
            root.display()
        )));
    }
    Ok(canonical)
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
fn validate_contained_path(
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
fn open_regular_file_for_read(root: &Path, rel_path: &Path) -> Result<File> {
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
fn part_file_path(file_path: &str) -> PathBuf {
    let mut part_path = OsString::from(file_path);
    part_path.push(".part");
    PathBuf::from(part_path)
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

/// Calculates the MD5 hash of a verified regular file inside `root`.
fn calculate_md5(root: &Path, rel_path: &Path, running: &Arc<AtomicBool>) -> Result<String> {
    let file = open_regular_file_for_read(root, rel_path)?;
    let file_size = file.metadata()?.len();
    let is_large_file = file_size > LARGE_FILE_THRESHOLD;

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

/// Check if an existing file inside `root` has the correct hash.
fn check_existing_file(
    root: &Path,
    file_path: &str,
    expected_md5: Option<&str>,
    running: &Arc<AtomicBool>,
) -> Result<Option<bool>> {
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
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    }

    let Some(expected) = expected_md5 else {
        return Ok(Some(true));
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
            return Ok(Some(false));
        }
    };

    Ok(Some(local_md5 == expected))
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
    cookie_header: Option<&str>,
    expected_md5: Option<&str>,
    policy: &RetryPolicy,
) -> Result<(u64, bool)> {
    let mut retry_count: u32 = 0;
    let mut has_reset_range = false;

    loop {
        if !signal.is_running() {
            return Err(interrupted_error());
        }

        // Re-check `.part` file size at start of each attempt (in case of retry)
        let current_file_size = existing_part_size(root, part_path)?;

        let mut headers = HeaderMap::new();
        if let Some(cookie_header) = cookie_header {
            headers.insert(
                reqwest::header::COOKIE,
                HeaderValue::from_str(cookie_header).map_err(|e| {
                    IaGetError::Network(format!("Invalid cookie header value: {}", e))
                })?,
            );
        }
        if current_file_size > 0 {
            // Use IaGetError::Network for header parsing errors
            headers.insert(
                reqwest::header::RANGE,
                HeaderValue::from_str(&format!("bytes={}-", current_file_size)).map_err(|e| {
                    IaGetError::Network(format!("Invalid range header value: {}", e))
                })?,
            );
        }

        let mut request = client.get(url);
        if !headers.is_empty() {
            request = request.headers(headers);
        }

        let mut response = match await_interruptible(request.send(), signal).await? {
            Ok(resp) => resp,
            Err(e) => {
                // Request failed before we even got a response
                if !is_retryable_reqwest_error(&e) {
                    return Err(e.into());
                }

                if retry_count >= policy.max_retries {
                    println!(
                        "{} {}      {} Maximum retries ({}) exceeded",
                        "├╼".cyan().dimmed(),
                        "Failed".red().bold(),
                        "✘".red().bold(),
                        policy.max_retries
                    );
                    return Err(e.into());
                }

                retry_count += 1;
                let delay = compute_retry_delay(retry_count, None, SystemTime::now(), policy);
                println!(
                    "{} {}      {} Connection error (attempt {}/{}): {}",
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
                let delay = compute_retry_delay(
                    retry_count,
                    retry_after.as_deref(),
                    SystemTime::now(),
                    policy,
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
                        return Ok((current_file_size, true));
                    }
                    if has_reset_range {
                        return Err(IaGetError::Integrity(format!(
                            "MD5 mismatch for {}",
                            part_path.display()
                        )));
                    }
                    has_reset_range = true;
                    remove_part_file_safely(root, part_path)?;
                    continue;
                }
                if !signal.is_running() {
                    return Err(interrupted_error());
                }
                return Ok((current_file_size, false));
            }
            ResponseAction::DiscardAndRefetchUnranged => {
                if has_reset_range {
                    return Err(IaGetError::Network(format!(
                        "HTTP error {}: repeated invalid range response after reset",
                        status
                    )));
                }
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

        let start_time = std::time::Instant::now();
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

                let chunk = chunk_result?;
                file.write_all(&chunk)?;
                downloaded_bytes = downloaded_bytes.saturating_add(chunk.len() as u64);
                total_bytes = total_bytes.saturating_add(chunk.len() as u64);
                pb.set_position(total_bytes);
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
                let delay = compute_retry_delay(retry_count, None, SystemTime::now(), policy);
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
/// using an explicit `RetryPolicy`.
#[allow(clippy::too_many_arguments)]
async fn download_single_file_with_policy(
    client: &Client,
    root: &Path,
    url: &str,
    file_path: &str,
    expected_md5: Option<&str>,
    signal: &ShutdownSignal,
    cookie_header: Option<&str>,
    policy: &RetryPolicy,
) -> Result<FileOutcome> {
    if !signal.is_running() {
        return Err(interrupted_error());
    }

    if let Some(is_valid) =
        check_existing_file(root, file_path, expected_md5, signal.running_flag())?
    {
        if is_valid {
            println!(
                "{} {}   {}",
                "╰╼".cyan().dimmed(),
                "Downloaded".white(),
                "✔".green().bold()
            );
            return Ok(FileOutcome::Skipped);
        } else {
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
        cookie_header,
        expected_md5,
        policy,
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

/// Download a single file inside `root` via `.part` staging, MD5 verification, and atomic promotion.
#[cfg(test)]
async fn download_single_file(
    client: &Client,
    root: &Path,
    url: &str,
    file_path: &str,
    expected_md5: Option<&str>,
    signal: &ShutdownSignal,
    cookie_header: Option<&str>,
) -> Result<FileOutcome> {
    download_single_file_with_policy(
        client,
        root,
        url,
        file_path,
        expected_md5,
        signal,
        cookie_header,
        &RetryPolicy::default(),
    )
    .await
}

/// Downloads multiple files into destination `root` with an explicit retry policy and shutdown signal.
///
/// Continues past per-file failures after printing a warning, stops promptly on
/// shutdown signal interruption, prints an end-of-batch summary, and returns the
/// collected [`BatchSummary`].
pub async fn download_files_in_dir<I>(
    client: &Client,
    root: &Path,
    files: I,
    total_files: usize,
    cookie_header: Option<&str>,
    policy: &RetryPolicy,
    signal: &ShutdownSignal,
) -> BatchSummary
where
    I: IntoIterator<Item = (String, String, Option<String>)>, // (url, filename, md5)
{
    let mut summary = BatchSummary::default();

    for (index, (url, file_path, expected_md5)) in files.into_iter().enumerate() {
        if !signal.is_running() {
            summary.interrupted = true;
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

        match download_single_file_with_policy(
            client,
            root,
            &url,
            &file_path,
            expected_md5.as_deref(),
            signal,
            cookie_header,
            policy,
        )
        .await
        {
            Ok(FileOutcome::Downloaded) => summary.ok += 1,
            Ok(FileOutcome::Skipped) => summary.skipped += 1,
            Err(err) => {
                if !signal.is_running() {
                    summary.interrupted = true;
                    println!(
                        "\n{} Download interrupted. Run the command again to resume remaining files.",
                        "✘".red().bold()
                    );
                    break;
                }

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

    print_batch_summary(&summary);
    summary
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
    cookie_header: Option<&str>,
    policy: &RetryPolicy,
    signal: &ShutdownSignal,
) -> BatchSummary
where
    I: IntoIterator<Item = (String, String, Option<String>)>, // (url, filename, md5)
{
    download_files_in_dir(
        client,
        Path::new("."),
        files,
        total_files,
        cookie_header,
        policy,
        signal,
    )
    .await
}

/// Download multiple files with shared signal handling.
///
/// Sets up signal handling once for the entire download session, continues past
/// individual file failures, prints an end-of-batch summary, and returns the
/// collected [`BatchSummary`].
pub async fn download_files<I>(
    client: &Client,
    files: I,
    total_files: usize,
    cookie_header: Option<&str>,
) -> BatchSummary
where
    I: IntoIterator<Item = (String, String, Option<String>)>, // (url, filename, md5)
{
    let signal = install_signal_handler();
    download_files_with_policy_and_signal(
        client,
        files,
        total_files,
        cookie_header,
        &RetryPolicy::default(),
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
        addr: std::net::SocketAddr,
        requests: Arc<Mutex<Vec<CapturedRequest>>>,
        handle: Option<tokio::task::JoinHandle<()>>,
    }

    impl MockServer {
        async fn start(responses: Vec<MockResponse>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind mock server");
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
                addr,
                requests,
                handle: Some(handle),
            }
        }

        fn url(&self, path: &str) -> String {
            format!("http://{}{}", self.addr, path)
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
        let client = Client::new();
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

        let client = Client::new();
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

        let client = Client::new();
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
        let client = Client::new();
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

        let client = Client::new();
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

            let client = Client::new();
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

        let client = Client::new();
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

        let client = Client::new();
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

        let client = Client::new();
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

        let client = Client::new();
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

        let client = Client::new();
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
        let client = Client::new();
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

        let client = Client::new();
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
        let client = Client::new();

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
        let client = Client::new();
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
        let body = fetch_text_with_retry(
            &policy,
            &signal,
            || client.get(server.url("/meta.xml")),
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
        let err = send_with_retry(
            &policy,
            &signal,
            || client.get(server_404.url("/missing")),
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
        let err = send_with_retry(
            &small_policy,
            &signal,
            || client.get(server_503.url("/down")),
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
        let err = send_with_retry(
            &RetryPolicy {
                max_retries: 2,
                max_retry_after: Duration::from_secs(60),
                ..policy
            },
            &cancel_signal,
            || client.get(server_cancel.url("/rate-limited")),
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
        let client = Client::new();
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
        let client = Client::new();
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
        let client = Client::new();
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
        let client = Client::new();
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
        let client = Client::new();
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
        let client = Client::new();
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
        let client = Client::new();
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
        let client = Client::new();
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

        let client = Client::new();
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

        let summary =
            download_files_in_dir(&client, &dir.path, files, 3, None, &policy, &signal).await;

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

        let client = Client::new();
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
            download_files_in_dir(&client, &dir.path, files, 2, None, &fast_policy, &signal).await;

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

        let client = Client::new();
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

        let summary =
            download_files_in_dir(&client, &dir.path, files, 2, None, &policy, &signal).await;

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

        let client = Client::new();
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

        let summary =
            download_files_in_dir(&client, &dir.path, files, 4, None, &policy, &signal).await;

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

        let client = Client::new();
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

        let summary =
            download_files_in_dir(&client, &dir.path, files, 3, None, &policy, &signal).await;

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

        let client = Client::new();
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

        let summary =
            download_files_in_dir(&client, &dir.path, files, 2, None, &policy, &signal).await;

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
        let client = Client::new();
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
        let client = Client::new();
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
        let client = Client::new();
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
        let client = Client::new();
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

        let client = Client::new();
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

        let client = Client::new();
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
        let client = Client::new();
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
        let client = Client::new();
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
}
