//! Module for handling file downloads, verification, and related operations.

mod collision;
mod fs_safety;
mod proxy;
mod redirect;
mod retry;
mod scheduler;

use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use colored::*;
use indicatif::{MultiProgress, ProgressBar};
use reqwest::header::HeaderValue;
use reqwest::{Client, StatusCode, Url};
use tokio::sync::Mutex as AsyncMutex;
use tokio::task::JoinSet;

use crate::constants::MAX_REDIRECTS;
use crate::cookie::CookieSource;
use crate::disk_space::PlannedDownload;
use crate::error::IaGetError;
use crate::utils::{
    create_attached_progress_bar, create_progress_bar, format_duration, format_size,
    format_transfer_rate, print_run_summary, progress_draw_target, sanitize_url_for_log,
    VerboseLogger,
};
use crate::Result;

pub use collision::{validate_download_paths_for_dir, PathCaseSensitivity};
pub use fs_safety::prepare_output_dir;
pub(crate) use fs_safety::{
    inspect_contained_regular_file, open_regular_file_for_read, part_file_path,
    resolve_destination_root, validate_or_prepare_dir,
};
pub use proxy::{http_client_builder, http_client_builder_with_proxy, ProxyConfig};
pub use redirect::attach_cookie_header;
pub use retry::{
    compute_backoff_delay_with_sample, fetch_text_with_retry_with_logger, install_signal_handler,
    is_retryable_reqwest_error, is_retryable_status, parse_retry_after,
    send_with_retry_with_logger, sleep_interruptible, RetryEvent, RetryPolicy, ShutdownSignal,
};
pub use scheduler::{
    base_permit_cost, download_sort_key, sort_planned_downloads, ElasticScheduler, SchedulerPermit,
    BACKPRESSURE_RECOVERY_SUCCESSES, MAX_CONCURRENT_PERMITS, MEDIUM_FILE_MAX_BYTES,
    SMALL_FILE_MAX_BYTES,
};

use fs_safety::{
    ensure_parent_directories, existing_part_size,
    open_regular_file_for_read_in_canonical_root_typed, prepare_file_for_download,
    promote_part_file, remove_part_file_safely, ContainedFileError,
};
use proxy::redact_proxy_value;
use redirect::{send_following_redirects, RedirectSendOutcome};
use retry::{
    await_interruptible, compute_retry_delay_with_source, interrupted_error, is_throttle_status,
    parse_ascii_u64,
};

/// Buffer size for file operations (8KB)
const BUFFER_SIZE: usize = 8192;

/// File size threshold for showing hash progress bar (2MB)
const LARGE_FILE_THRESHOLD: u64 = 2 * 1024 * 1024;

/// Idle stall threshold beyond which [`RateLimiter`] resets its pacing window
/// so network stalls cannot accumulate multi-second unthrottled bursts.
const RATE_LIMIT_STALL_RESET_WINDOW: Duration = Duration::from_millis(100);

/// Configuration controlling download retries and bandwidth throttling.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DownloadOptions {
    pub retry_policy: RetryPolicy,
    pub limit_rate: Option<u64>,
}

/// A single file scheduled for download within a batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadItem {
    /// Absolute download URL for the file.
    pub url: String,
    /// Sanitised relative destination path inside the output directory.
    pub file_path: String,
    /// Optional expected MD5 digest from archive metadata.
    pub expected_md5: Option<String>,
    /// Optional expected file size in bytes from archive metadata.
    pub expected_size: Option<u64>,
}

impl From<PlannedDownload> for DownloadItem {
    fn from(item: PlannedDownload) -> Self {
        Self {
            url: item.url,
            file_path: item.file_path,
            expected_md5: item.expected_md5,
            expected_size: item.expected_size,
        }
    }
}

impl From<(String, String, Option<String>)> for DownloadItem {
    fn from((url, file_path, expected_md5): (String, String, Option<String>)) -> Self {
        Self {
            url,
            file_path,
            expected_md5,
            expected_size: None,
        }
    }
}

impl From<(String, String, Option<String>, Option<u64>)> for DownloadItem {
    fn from(
        (url, file_path, expected_md5, expected_size): (
            String,
            String,
            Option<String>,
            Option<u64>,
        ),
    ) -> Self {
        Self {
            url,
            file_path,
            expected_md5,
            expected_size,
        }
    }
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
            signal.sleep_or_shutdown(delay).await?;
        }
        Ok(())
    }
}

/// Paces `chunk_bytes` against a shared batch [`RateLimiter`] without holding the mutex across sleep.
async fn throttle_shared_rate_limiter(
    limiter: &AsyncMutex<RateLimiter>,
    chunk_bytes: u64,
    signal: &ShutdownSignal,
) -> Result<()> {
    let delay = {
        let mut guard = limiter.lock().await;
        guard.delay_for_chunk(chunk_bytes, Instant::now())
    };
    if delay > Duration::ZERO {
        signal.sleep_or_shutdown(delay).await?;
    }
    Ok(())
}

/// RAII guard that finishes, clears, and detaches a progress bar on every exit path.
struct ProgressBarGuard {
    pb: ProgressBar,
    multi: Option<MultiProgress>,
}

impl ProgressBarGuard {
    fn new(pb: ProgressBar, multi: Option<MultiProgress>) -> Self {
        Self { pb, multi }
    }

    fn set_position(&self, pos: u64) {
        self.pb.set_position(pos);
    }
}

impl Drop for ProgressBarGuard {
    fn drop(&mut self) {
        self.pb.finish_and_clear();
        if let Some(ref multi) = self.multi {
            multi.remove(&self.pb);
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

fn calculate_md5_in_canonical_root_typed(
    canonical_root: &Path,
    rel_path: &Path,
    running: &Arc<AtomicBool>,
    show_progress: bool,
    multi_progress: Option<&MultiProgress>,
    progress_label: Option<&str>,
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
        let action = format!("{} {}    ", "╰╼".cyan().dimmed(), "Verifying".white());
        let bar = match multi_progress {
            Some(multi) => {
                let label = progress_label
                    .map(str::to_owned)
                    .unwrap_or_else(|| rel_path.display().to_string());
                create_attached_progress_bar(
                    multi,
                    file_size,
                    &action,
                    &label,
                    Some("blue/blue"),
                    false,
                )
            }
            None => create_progress_bar(file_size, &action, Some("blue/blue"), false),
        };
        Some(ProgressBarGuard::new(bar, multi_progress.cloned()))
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

    drop(pb);

    let hash = context.finalize();
    Ok(format!("{:x}", hash))
}

/// Offloads blocking MD5 computation onto Tokio's blocking pool so concurrent async downloads
/// are never stalled while hashing large files.
async fn calculate_md5_in_canonical_root_async(
    canonical_root: PathBuf,
    rel_path: PathBuf,
    running: Arc<AtomicBool>,
    show_progress: bool,
    multi_progress: Option<MultiProgress>,
    progress_label: Option<String>,
) -> std::result::Result<String, ContainedFileError> {
    tokio::task::spawn_blocking(move || {
        calculate_md5_in_canonical_root_typed(
            &canonical_root,
            &rel_path,
            &running,
            show_progress,
            multi_progress.as_ref(),
            progress_label.as_deref(),
        )
    })
    .await
    .map_err(|err| {
        ContainedFileError::Io(IaGetError::FileSystem(format!(
            "MD5 hashing task failed: {err}"
        )))
    })?
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
    calculate_md5_in_canonical_root_typed(
        &canonical_root,
        rel_path,
        running,
        show_progress,
        None,
        None,
    )
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

/// Check if an existing file inside `root` has the correct hash, offloading MD5 I/O to a
/// blocking worker thread.
async fn check_existing_file(
    root: &Path,
    file_path: &str,
    expected_md5: Option<&str>,
    running: &Arc<AtomicBool>,
    multi_progress: Option<&MultiProgress>,
    detail_lines: &mut Vec<String>,
) -> Result<ExistingFileStatus> {
    let canonical_root = resolve_destination_root(root)?;
    let rel_path = PathBuf::from(file_path);

    if inspect_contained_regular_file(&canonical_root, &rel_path)?.is_none() {
        return Ok(ExistingFileStatus::Missing);
    }

    let Some(expected) = expected_md5 else {
        return Ok(ExistingFileStatus::CompleteNoMd5);
    };

    match calculate_md5_in_canonical_root_async(
        canonical_root,
        rel_path,
        Arc::clone(running),
        true,
        multi_progress.cloned(),
        Some(file_path.to_string()),
    )
    .await
    {
        Ok(local_md5) if local_md5 == expected => Ok(ExistingFileStatus::CompleteMd5Verified),
        Ok(_) => Ok(ExistingFileStatus::NeedsRedownload),
        Err(ContainedFileError::Containment(err) | ContainedFileError::Interrupted(err)) => {
            Err(err)
        }
        Err(ContainedFileError::Io(err)) => {
            detail_lines.push(format!(
                "{} {} to calculate MD5 hash: {}",
                "├╼".cyan().dimmed(),
                "Failed".red().bold(),
                err
            ));
            Ok(ExistingFileStatus::NeedsRedownload)
        }
    }
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

/// Logs, buffers tree status lines, notifies shared backpressure on 429/503, and waits for a
/// retryable download failure, returning `Ok(false)` when retries are exhausted or `Ok(true)`
/// once ready for the next attempt.
#[allow(clippy::too_many_arguments)]
async fn wait_for_download_retry(
    retry_count: &mut u32,
    policy: &RetryPolicy,
    safe_initial_url: &str,
    kind: DownloadRetryKind<'_>,
    signal: &ShutdownSignal,
    scheduler: Option<&ElasticScheduler>,
    logger: &VerboseLogger,
    detail_lines: &mut Vec<String>,
) -> Result<bool> {
    if *retry_count >= policy.max_retries {
        if let DownloadRetryKind::HttpStatus {
            status,
            retry_after,
        } = kind
        {
            if is_throttle_status(status) {
                if let Some(scheduler) = scheduler {
                    let (delay, delay_source) = compute_retry_delay_with_source(
                        *retry_count + 1,
                        retry_after,
                        SystemTime::now(),
                        policy,
                    );
                    let (prev_ceiling, new_ceiling) = scheduler.record_throttle(delay);
                    logger.log(
                        "scheduler",
                        format_args!(
                            "backpressure after HTTP {status} (retries exhausted): ceiling {prev_ceiling}->{new_ceiling}, cooldown {:.2}s ({delay_source})",
                            delay.as_secs_f64()
                        ),
                    );
                }
            }
        }

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
        detail_lines.push(format!(
            "{} {}      {} Maximum retries ({}) exceeded",
            "├╼".cyan().dimmed(),
            "Failed".red().bold(),
            "✘".red().bold(),
            policy.max_retries
        ));
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

    if let DownloadRetryKind::HttpStatus { status, .. } = kind {
        if is_throttle_status(status) {
            if let Some(scheduler) = scheduler {
                let (prev_ceiling, new_ceiling) = scheduler.record_throttle(delay);
                logger.log(
                    "scheduler",
                    format_args!(
                        "backpressure after HTTP {status}: ceiling {prev_ceiling}->{new_ceiling}, cooldown {wait_secs:.2}s ({delay_source})"
                    ),
                );
            }
        }
    }

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
    detail_lines.push(format!(
        "{} {}      {} {status_line}",
        "├╼".cyan().dimmed(),
        "Retry".yellow().bold(),
        "⟳".yellow().bold()
    ));
    detail_lines.push(format!(
        "{} {}      Waiting {wait_secs:.1}s before retry...",
        "├╼".cyan().dimmed(),
        "Wait".white()
    ));

    signal.sleep_or_shutdown(delay).await?;
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

/// Shared batch runtime collaborators passed into per-file transfers.
#[derive(Clone, Default)]
struct BatchTransferContext {
    rate_limiter: Option<Arc<AsyncMutex<RateLimiter>>>,
    scheduler: Option<ElasticScheduler>,
    multi_progress: Option<MultiProgress>,
}

/// Download file content into a `.part` file with progress reporting and automatic retry on failure
#[allow(clippy::too_many_arguments)]
async fn download_file_content(
    client: &Client,
    root: &Path,
    url: &str,
    file_path: &str,
    part_path: &Path,
    signal: &ShutdownSignal,
    cookie_source: Option<&CookieSource>,
    expected_md5: Option<&str>,
    options: &DownloadOptions,
    context: &BatchTransferContext,
    logger: &VerboseLogger,
    detail_lines: &mut Vec<String>,
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

        if let Some(ref scheduler) = context.scheduler {
            scheduler.wait_for_cooldown(signal).await?;
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
                    context.scheduler.as_ref(),
                    logger,
                    detail_lines,
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
                context.scheduler.as_ref(),
                logger,
                detail_lines,
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
                        context.multi_progress.as_ref(),
                        Some(file_path),
                        detail_lines,
                    )
                    .await?;
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

        let pb_bar = match context.multi_progress.as_ref() {
            Some(multi) => create_attached_progress_bar(
                multi,
                total_expected_size,
                &download_action,
                file_path,
                Some("green/green"),
                true,
            ),
            None => create_progress_bar(
                total_expected_size,
                &download_action,
                Some("green/green"),
                true,
            ),
        };
        let pb = ProgressBarGuard::new(pb_bar, context.multi_progress.clone());

        // Set initial progress to starting offset for resumed downloads
        pb.set_position(start_offset);

        let start_time = Instant::now();
        let mut local_rate_limiter = if context.rate_limiter.is_none() {
            options.limit_rate.map(RateLimiter::new)
        } else {
            None
        };
        let mut total_bytes: u64 = start_offset;
        let mut downloaded_bytes: u64 = 0;

        // Attempt the download
        let download_result: Result<()> = async {
            while let Some(chunk_result) = await_interruptible(response.chunk(), signal)
                .await?
                .transpose()
            {
                if !signal.is_running() {
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

                if let Some(ref shared_limiter) = context.rate_limiter {
                    throttle_shared_rate_limiter(shared_limiter, chunk_len, signal).await?;
                } else if let Some(limiter) = local_rate_limiter.as_mut() {
                    limiter.throttle(chunk_len, signal).await?;
                }
            }
            Ok(())
        }
        .await;

        drop(pb);

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

                detail_lines.push(format!(
                    "{} {}   {} {} in {} ({:.2} {}/s)",
                    "├╼".cyan().dimmed(),
                    "Downloaded".white(),
                    "↓".green().bold(),
                    format_size(downloaded_bytes).bold(),
                    format_duration(elapsed).bold(),
                    rate,
                    unit
                ));

                return Ok((total_bytes, false));
            }
            Err(e) => {
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
                    context.scheduler.as_ref(),
                    logger,
                    detail_lines,
                )
                .await?
                {
                    return Err(e);
                }
            }
        }
    }
}

/// Verify a downloaded file's hash against an expected value inside `root`,
/// offloading blocking MD5 computation onto Tokio's blocking pool.
async fn verify_downloaded_file(
    root: &Path,
    rel_path: &Path,
    expected_md5: Option<&str>,
    running: &Arc<AtomicBool>,
    multi_progress: Option<&MultiProgress>,
    progress_label: Option<&str>,
    detail_lines: &mut Vec<String>,
) -> Result<bool> {
    let Some(expected_md5_str) = expected_md5 else {
        detail_lines.push(format!(
            "{} {}",
            "-".dimmed(),
            "No MD5 hash provided for verification.".dimmed()
        ));
        return Ok(true); // No hash to check against, consider it verified
    };
    let canonical_root = resolve_destination_root(root)?;
    let local_md5 = calculate_md5_in_canonical_root_async(
        canonical_root,
        rel_path.to_path_buf(),
        Arc::clone(running),
        true,
        multi_progress.cloned(),
        progress_label.map(str::to_owned),
    )
    .await
    .map_err(ContainedFileError::into_ia_get_error)?;

    if local_md5 == expected_md5_str {
        detail_lines.push(format!(
            "{} {}         {} {}",
            "╰╼".cyan().dimmed(),
            "Hash".white(),
            "✔".green().bold(),
            format!("({})", local_md5).dimmed()
        ));
        Ok(true)
    } else {
        detail_lines.push(format!(
            "{} {}         {} ({}) Expected ({})",
            "╰╼".cyan().dimmed(),
            "Hash".white(),
            "✘".red().bold(),
            local_md5.red(),
            expected_md5_str.dimmed()
        ));
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

/// Formats a complete per-file tree block (`▣` / `├╼` / `╰╼`) for atomic printing.
fn format_file_completion_block(
    file_path: &str,
    completed_index: usize,
    total_files: usize,
    detail_lines: &[String],
) -> String {
    let mut block = String::new();
    block.push_str(" \n");
    block.push_str(&format!(
        "{}  {}     {}\n",
        "▣".bright_cyan().bold(),
        "Filename".white(),
        file_path.bold()
    ));
    block.push_str(&format!(
        "{} {}        {} {} of {}",
        "├╼".cyan().dimmed(),
        "Count".white(),
        "#".blue().bold(),
        completed_index.to_string().bold(),
        total_files.to_string().bold()
    ));
    for line in detail_lines {
        block.push('\n');
        block.push_str(line);
    }
    block
}

/// Prints a multi-line completion block atomically on both TTY and non-TTY stdout.
fn emit_multi_progress_block(multi: &MultiProgress, block: &str) {
    if multi.is_hidden() {
        multi.suspend(|| {
            println!("{block}");
        });
    } else {
        let _ = multi.println(block);
    }
}

/// Download a single file inside `root` via `.part` staging, MD5 verification, and atomic promotion
/// with shared batch context, returning collected tree lines on both success and failure.
#[allow(clippy::too_many_arguments)]
async fn download_single_file_with_context(
    client: &Client,
    root: &Path,
    url: &str,
    file_path: &str,
    expected_md5: Option<&str>,
    signal: &ShutdownSignal,
    cookie_source: Option<&CookieSource>,
    options: &DownloadOptions,
    context: &BatchTransferContext,
    batch_logger: &VerboseLogger,
    transfer_logger: &VerboseLogger,
) -> std::result::Result<(FileOutcome, Vec<String>), (IaGetError, Vec<String>)> {
    let mut detail_lines = Vec::new();
    if !signal.is_running() {
        return Err((interrupted_error(), detail_lines));
    }

    let existing_status = match check_existing_file(
        root,
        file_path,
        expected_md5,
        signal.running_flag(),
        context.multi_progress.as_ref(),
        &mut detail_lines,
    )
    .await
    {
        Ok(status) => status,
        Err(err) => return Err((err, detail_lines)),
    };

    match existing_status {
        ExistingFileStatus::Missing => {}
        ExistingFileStatus::CompleteMd5Verified => {
            batch_logger.log(
                "download",
                format_args!("skipped '{file_path}': already exists; MD5 verified"),
            );
            detail_lines.push(format!(
                "{} {}   {}",
                "╰╼".cyan().dimmed(),
                "Downloaded".white(),
                "✔".green().bold()
            ));
            return Ok((FileOutcome::Skipped, detail_lines));
        }
        ExistingFileStatus::CompleteNoMd5 => {
            batch_logger.log(
                "download",
                format_args!("skipped '{file_path}': already exists; no MD5 in metadata"),
            );
            detail_lines.push(format!(
                "{} {}   {}",
                "╰╼".cyan().dimmed(),
                "Downloaded".white(),
                "✔".green().bold()
            ));
            return Ok((FileOutcome::Skipped, detail_lines));
        }
        ExistingFileStatus::NeedsRedownload => {
            batch_logger.log(
                "download",
                format_args!("redownloading '{file_path}': existing file hash mismatch"),
            );
            detail_lines.push(format!(
                "{} {}      {}",
                "├╼".cyan().dimmed(),
                "Partial".white(),
                "▲".yellow().bold()
            ));
        }
    }

    if !signal.is_running() {
        return Err((interrupted_error(), detail_lines));
    }

    if let Err(err) = ensure_parent_directories(root, Path::new(file_path)) {
        return Err((err, detail_lines));
    }

    let part_path = part_file_path(file_path);
    let (_, already_verified) = match download_file_content(
        client,
        root,
        url,
        file_path,
        &part_path,
        signal,
        cookie_source,
        expected_md5,
        options,
        context,
        transfer_logger,
        &mut detail_lines,
    )
    .await
    {
        Ok(res) => res,
        Err(err) => return Err((err, detail_lines)),
    };

    if !signal.is_running() {
        return Err((interrupted_error(), detail_lines));
    }

    let verified = if already_verified {
        true
    } else {
        match verify_downloaded_file(
            root,
            &part_path,
            expected_md5,
            signal.running_flag(),
            context.multi_progress.as_ref(),
            Some(file_path),
            &mut detail_lines,
        )
        .await
        {
            Ok(valid) => valid,
            Err(err) => return Err((err, detail_lines)),
        }
    };

    if verified {
        if !signal.is_running() {
            return Err((interrupted_error(), detail_lines));
        }
        if let Err(err) = promote_part_file(root, &part_path, Path::new(file_path)) {
            return Err((err, detail_lines));
        }
        Ok((FileOutcome::Downloaded, detail_lines))
    } else {
        Err((
            IaGetError::Integrity(format!("MD5 mismatch for {}", file_path)),
            detail_lines,
        ))
    }
}

/// Download a single file inside `root` via `.part` staging, MD5 verification, and atomic promotion
/// using explicit `DownloadOptions`.
#[cfg(test)]
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
    let context = BatchTransferContext::default();
    download_single_file_with_context(
        client,
        root,
        url,
        file_path,
        expected_md5,
        signal,
        cookie_source,
        options,
        &context,
        logger,
        logger,
    )
    .await
    .map(|(outcome, _)| outcome)
    .map_err(|(err, _)| err)
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

enum TaskOutcome {
    Downloaded,
    Skipped,
    Failed { error: String },
    Interrupted,
}

struct TaskCompletion {
    original_index: usize,
    file_path: String,
    outcome: TaskOutcome,
}

fn apply_task_completion(
    completion: TaskCompletion,
    summary: &mut BatchSummary,
    failed_with_index: &mut Vec<(usize, FailedFile)>,
    interrupted: &mut bool,
    interrupt_reason: &mut Option<String>,
) {
    match completion.outcome {
        TaskOutcome::Downloaded => summary.ok += 1,
        TaskOutcome::Skipped => summary.skipped += 1,
        TaskOutcome::Failed { error } => {
            failed_with_index.push((
                completion.original_index,
                FailedFile {
                    file_path: completion.file_path,
                    error,
                },
            ));
        }
        TaskOutcome::Interrupted => {
            *interrupted = true;
            if interrupt_reason.is_none() {
                *interrupt_reason = Some(format!(
                    "interrupted while downloading '{}'",
                    completion.file_path
                ));
            }
        }
    }
}

/// Downloads multiple files into destination `root` with default disabled verbose logging.
#[allow(clippy::too_many_arguments)]
pub async fn download_files_in_dir<I>(
    client: &Client,
    root: &Path,
    files: I,
    total_files: usize,
    cookie_source: Option<&CookieSource>,
    options: &DownloadOptions,
    signal: &ShutdownSignal,
) -> Result<BatchSummary>
where
    I: IntoIterator,
    I::Item: Into<DownloadItem>,
{
    download_files_in_dir_with_logger(
        client,
        root,
        files,
        total_files,
        cookie_source,
        options,
        signal,
        &VerboseLogger::disabled(),
    )
    .await
}

/// Downloads multiple files into destination `root` with explicit [`DownloadOptions`],
/// shutdown signal, and [`VerboseLogger`].
///
/// Validates and prepares `root` before downloading, schedules transfers through the
/// 4-permit [`ElasticScheduler`] sorted by ascending `expected_size` (`None` last),
/// continues past per-file failures after printing an atomic tree block, preserves
/// original metadata order in [`BatchSummary::failed`], stops promptly on shutdown
/// signal interruption, prints an end-of-batch summary, and returns [`BatchSummary`].
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
    I: IntoIterator,
    I::Item: Into<DownloadItem>,
{
    let items: Vec<DownloadItem> = files.into_iter().map(Into::into).collect();
    validate_download_paths_for_dir(
        root,
        items
            .iter()
            .map(|item| (item.file_path.as_str(), item.file_path.as_str())),
    )?;

    let canonical_root = prepare_output_dir(root)?;
    let files_count = items.len();
    let mut indexed_items: Vec<(usize, DownloadItem)> = items.into_iter().enumerate().collect();
    sort_planned_downloads(&mut indexed_items, |item| item.expected_size);

    let multi_progress = MultiProgress::with_draw_target(progress_draw_target());
    let multi_guard = signal.enter_multi_progress();
    let scheduler = ElasticScheduler::new();
    let shared_rate_limiter = options
        .limit_rate
        .map(|bytes_per_sec| Arc::new(AsyncMutex::new(RateLimiter::new(bytes_per_sec))));
    let shared_cookies = cookie_source.cloned().map(Arc::new);
    let completed_count = Arc::new(AtomicUsize::new(0));
    let prefix_concurrent_logs = files_count > 1;

    let mut summary = BatchSummary::default();
    let mut failed_with_index: Vec<(usize, FailedFile)> = Vec::new();
    let mut interrupted = false;
    let mut interrupt_reason: Option<String> = None;
    let mut join_set: JoinSet<TaskCompletion> = JoinSet::new();

    for (dispatch_idx, (original_index, item)) in indexed_items.into_iter().enumerate() {
        while let Some(join_result) = join_set.try_join_next() {
            match join_result {
                Ok(completion) => apply_task_completion(
                    completion,
                    &mut summary,
                    &mut failed_with_index,
                    &mut interrupted,
                    &mut interrupt_reason,
                ),
                Err(_) if !signal.is_running() => {
                    interrupted = true;
                }
                Err(join_err) => {
                    join_set.abort_all();
                    let _ = multi_progress.clear();
                    drop(multi_guard);
                    return Err(IaGetError::Network(format!(
                        "Download worker task failed: {join_err}"
                    )));
                }
            }
        }

        if !signal.is_running() {
            interrupted = true;
            if interrupt_reason.is_none() {
                interrupt_reason = Some(format!(
                    "interrupted before file {}/{total_files} ('{}')",
                    dispatch_idx + 1,
                    item.file_path
                ));
            }
            break;
        }

        let permit = match scheduler.acquire(item.expected_size, signal).await {
            Ok(permit) => permit,
            Err(_) if !signal.is_running() => {
                interrupted = true;
                if interrupt_reason.is_none() {
                    interrupt_reason = Some(format!(
                        "interrupted before file {}/{total_files} ('{}')",
                        dispatch_idx + 1,
                        item.file_path
                    ));
                }
                break;
            }
            Err(err) => {
                join_set.abort_all();
                let _ = multi_progress.clear();
                drop(multi_guard);
                return Err(err);
            }
        };

        let task_client = client.clone();
        let task_root = canonical_root.clone();
        let task_signal = signal.clone();
        let task_cookies = shared_cookies.clone();
        let task_options = *options;
        let task_batch_logger = logger.clone();
        let task_transfer_logger = if prefix_concurrent_logs {
            logger.with_prefix(&item.file_path)
        } else {
            logger.clone()
        };
        let task_completed_count = Arc::clone(&completed_count);
        let task_scheduler = scheduler.clone();
        let task_multi = multi_progress.clone();
        let task_context = BatchTransferContext {
            rate_limiter: shared_rate_limiter.clone(),
            scheduler: Some(scheduler.clone()),
            multi_progress: Some(multi_progress.clone()),
        };

        join_set.spawn(async move {
            let _permit = permit;
            let file_path = item.file_path;

            let outcome = match download_single_file_with_context(
                &task_client,
                &task_root,
                &item.url,
                &file_path,
                item.expected_md5.as_deref(),
                &task_signal,
                task_cookies.as_deref(),
                &task_options,
                &task_context,
                &task_batch_logger,
                &task_transfer_logger,
            )
            .await
            {
                Ok((FileOutcome::Downloaded, detail_lines)) => {
                    let (prev_ceiling, new_ceiling) = task_scheduler.record_network_success();
                    if new_ceiling > prev_ceiling {
                        task_transfer_logger.log(
                            "scheduler",
                            format_args!(
                                "recovered concurrency ceiling {prev_ceiling}->{new_ceiling}"
                            ),
                        );
                    }
                    let finished_index = task_completed_count.fetch_add(1, Ordering::SeqCst) + 1;
                    let block = format_file_completion_block(
                        &file_path,
                        finished_index,
                        total_files,
                        &detail_lines,
                    );
                    emit_multi_progress_block(&task_multi, &block);
                    task_batch_logger.log(
                        "download",
                        format_args!("completed '{file_path}' ({finished_index}/{total_files})"),
                    );
                    TaskOutcome::Downloaded
                }
                Ok((FileOutcome::Skipped, detail_lines)) => {
                    let finished_index = task_completed_count.fetch_add(1, Ordering::SeqCst) + 1;
                    let block = format_file_completion_block(
                        &file_path,
                        finished_index,
                        total_files,
                        &detail_lines,
                    );
                    emit_multi_progress_block(&task_multi, &block);
                    TaskOutcome::Skipped
                }
                Err((err, mut detail_lines)) => {
                    if !task_signal.is_running() {
                        TaskOutcome::Interrupted
                    } else {
                        let finished_index =
                            task_completed_count.fetch_add(1, Ordering::SeqCst) + 1;
                        task_batch_logger
                            .log("download", format_args!("failed '{file_path}': {err}"));
                        detail_lines.push(format!(
                            "{} {}      {} {}: {}",
                            "╰╼".cyan().dimmed(),
                            "Warning".yellow().bold(),
                            "⚠".yellow().bold(),
                            file_path.bold(),
                            err
                        ));
                        let block = format_file_completion_block(
                            &file_path,
                            finished_index,
                            total_files,
                            &detail_lines,
                        );
                        emit_multi_progress_block(&task_multi, &block);
                        TaskOutcome::Failed {
                            error: err.to_string(),
                        }
                    }
                }
            };

            TaskCompletion {
                original_index,
                file_path,
                outcome,
            }
        });
    }

    while let Some(join_result) = join_set.join_next().await {
        match join_result {
            Ok(completion) => apply_task_completion(
                completion,
                &mut summary,
                &mut failed_with_index,
                &mut interrupted,
                &mut interrupt_reason,
            ),
            Err(_) if !signal.is_running() => {
                interrupted = true;
            }
            Err(join_err) => {
                join_set.abort_all();
                let _ = multi_progress.clear();
                drop(multi_guard);
                return Err(IaGetError::Network(format!(
                    "Download worker task failed: {join_err}"
                )));
            }
        }
    }

    let _ = multi_progress.clear();
    drop(multi_guard);

    if interrupted {
        let reason = interrupt_reason.unwrap_or_else(|| "interrupted by user signal".to_string());
        record_batch_interrupted(&mut summary, logger, format_args!("{reason}"));
    }

    failed_with_index.sort_by_key(|(original_index, _)| *original_index);
    summary.failed = failed_with_index
        .into_iter()
        .map(|(_, failed)| failed)
        .collect();

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

#[cfg(test)]
pub(super) mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::*;
    use crate::test_support::{
        fast_retry_policy, md5_hex, CapturedLogs, ConcurrentMockServer, MockResponse, MockServer,
        TempTestDir,
    };

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn download_single_file_with_policy(
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

    pub(super) async fn run_batch_in_dir<I>(
        client: &Client,
        root: &Path,
        files: I,
        total_files: usize,
        policy: &RetryPolicy,
        signal: &ShutdownSignal,
    ) -> Result<BatchSummary>
    where
        I: IntoIterator,
        I::Item: Into<DownloadItem>,
    {
        let options = DownloadOptions {
            retry_policy: *policy,
            limit_rate: None,
        };
        download_files_in_dir_with_logger(
            client,
            root,
            files,
            total_files,
            None,
            &options,
            signal,
            &VerboseLogger::disabled(),
        )
        .await
    }

    pub(super) async fn download_single_file(
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

    pub(super) struct DownloadFixture {
        dir: TempTestDir,
        pub(super) client: reqwest::Client,
        pub(super) signal: ShutdownSignal,
    }

    impl DownloadFixture {
        pub(super) fn new(prefix: &str) -> Self {
            Self::with_client(prefix, http_client_builder().build().unwrap())
        }

        pub(super) fn with_client(prefix: &str, client: reqwest::Client) -> Self {
            Self {
                dir: TempTestDir::new(prefix),
                client,
                signal: ShutdownSignal::new(),
            }
        }

        pub(super) fn path(&self) -> &Path {
            self.dir.path()
        }

        pub(super) fn join(&self, rel: impl AsRef<Path>) -> PathBuf {
            self.dir.join(rel)
        }

        pub(super) fn part_path(&self, rel: &str) -> PathBuf {
            self.dir.part_path(rel)
        }

        pub(super) fn write(&self, rel: impl AsRef<Path>, data: impl AsRef<[u8]>) -> PathBuf {
            self.dir.write(rel, data)
        }

        pub(super) fn write_part(&self, rel: &str, data: impl AsRef<[u8]>) -> PathBuf {
            let path = self.part_path(rel);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent dirs for .part fixture");
            }
            fs::write(&path, data).expect("write .part fixture");
            path
        }

        pub(super) async fn download(
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

        pub(super) async fn download_with_policy(
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

        pub(super) fn assert_downloaded(&self, rel_path: &str, expected: &[u8]) {
            assert_downloaded_file(self.path(), rel_path, expected);
        }
    }

    pub(super) fn assert_downloaded_file(dir: &Path, rel_path: &str, expected: &[u8]) {
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

        let summary = run_batch_in_dir(&fx.client, fx.path(), files, 6, &policy, &fx.signal)
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

        let summary = run_batch_in_dir(&fx.client, fx.path(), files, 3, &policy, &fx.signal)
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

        let summary = run_batch_in_dir(&fx.client, fx.path(), files, 2, &policy, &fx.signal)
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
        };

        let started = Instant::now();
        let summary = download_files_in_dir_with_logger(
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
            &VerboseLogger::disabled(),
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
        let summary = download_files_in_dir_with_logger(
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
            &VerboseLogger::disabled(),
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
        };

        let started = Instant::now();
        let summary = download_files_in_dir_with_logger(
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
            &VerboseLogger::disabled(),
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

    #[tokio::test]
    async fn test_elastic_parallel_four_small_files_overlap_and_cap_at_four() {
        let payloads: Vec<(String, Vec<u8>)> = (0..6)
            .map(|idx| {
                (
                    format!("small_{idx}.bin"),
                    format!("payload-for-small-file-{idx}").into_bytes(),
                )
            })
            .collect();
        let payload_map: Arc<std::collections::HashMap<String, Vec<u8>>> = Arc::new(
            payloads
                .iter()
                .map(|(name, bytes)| (format!("/{name}"), bytes.clone()))
                .collect(),
        );
        let first_wave_barrier = Arc::new(tokio::sync::Barrier::new(4));
        let first_wave_count = Arc::new(AtomicUsize::new(0));

        let server = ConcurrentMockServer::start(move |req| {
            let map = Arc::clone(&payload_map);
            let barrier = Arc::clone(&first_wave_barrier);
            let counter = Arc::clone(&first_wave_count);
            async move {
                let path = req
                    .request_line
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("/")
                    .to_string();
                if counter.fetch_add(1, Ordering::SeqCst) < 4 {
                    let _ = tokio::time::timeout(Duration::from_secs(2), barrier.wait()).await;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
                match map.get(&path) {
                    Some(bytes) => MockResponse::ok(bytes),
                    None => MockResponse::status("404 Not Found", b""),
                }
            }
        })
        .await;

        let fx = DownloadFixture::new("elastic-4-small");
        let items: Vec<DownloadItem> = payloads
            .iter()
            .map(|(name, bytes)| DownloadItem {
                url: server.url(&format!("/{name}")),
                file_path: name.clone(),
                expected_md5: Some(md5_hex(bytes)),
                expected_size: Some(bytes.len() as u64),
            })
            .collect();

        let summary = run_batch_in_dir(
            &fx.client,
            fx.path(),
            items,
            payloads.len(),
            &fast_retry_policy(0),
            &fx.signal,
        )
        .await
        .expect("6 small parallel downloads should succeed");

        let peak = server.max_concurrent();
        let requests = server.stop().await;
        assert_eq!(requests.len(), 6);
        assert_eq!(
            peak, MAX_CONCURRENT_PERMITS,
            "6 small files should overlap at exactly 4 concurrent requests"
        );
        assert_eq!(summary.ok, 6);
        assert_eq!(summary.skipped, 0);
        assert!(summary.failed.is_empty());
        assert!(!summary.interrupted);
        for (name, bytes) in &payloads {
            fx.assert_downloaded(name, bytes);
        }
    }

    #[tokio::test]
    async fn test_elastic_funnel_drains_small_and_runs_large_sequentially() {
        let small_barrier = Arc::new(tokio::sync::Barrier::new(4));
        let active_total = Arc::new(AtomicUsize::new(0));
        let small_peak = Arc::new(AtomicUsize::new(0));
        let large_overlaps = Arc::new(std::sync::Mutex::new(Vec::<usize>::new()));
        let started_paths = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));

        let entries: Vec<(&str, u64, Vec<u8>)> = vec![
            ("large_1.bin", MEDIUM_FILE_MAX_BYTES, b"large-one".to_vec()),
            ("small_1.bin", 100, b"small-one".to_vec()),
            (
                "large_2.bin",
                MEDIUM_FILE_MAX_BYTES + 16 * 1024 * 1024,
                b"large-two".to_vec(),
            ),
            ("small_2.bin", 200, b"small-two".to_vec()),
            ("small_3.bin", 300, b"small-three".to_vec()),
            ("small_4.bin", 400, b"small-four".to_vec()),
        ];
        let body_map: Arc<std::collections::HashMap<String, Vec<u8>>> = Arc::new(
            entries
                .iter()
                .map(|(name, _, bytes)| (format!("/{name}"), bytes.clone()))
                .collect(),
        );

        let server = ConcurrentMockServer::start({
            let body_map = Arc::clone(&body_map);
            let small_barrier = Arc::clone(&small_barrier);
            let active_total = Arc::clone(&active_total);
            let small_peak = Arc::clone(&small_peak);
            let large_overlaps = Arc::clone(&large_overlaps);
            let started_paths = Arc::clone(&started_paths);
            move |req| {
                let body_map = Arc::clone(&body_map);
                let small_barrier = Arc::clone(&small_barrier);
                let active_total = Arc::clone(&active_total);
                let small_peak = Arc::clone(&small_peak);
                let large_overlaps = Arc::clone(&large_overlaps);
                let started_paths = Arc::clone(&started_paths);
                async move {
                    let path = req
                        .request_line
                        .split_whitespace()
                        .nth(1)
                        .unwrap_or("/")
                        .to_string();
                    started_paths.lock().unwrap().push(path.clone());
                    let cur = active_total.fetch_add(1, Ordering::SeqCst) + 1;

                    if path.starts_with("/small_") {
                        small_peak.fetch_max(cur, Ordering::SeqCst);
                        let _ = tokio::time::timeout(Duration::from_secs(2), small_barrier.wait())
                            .await;
                        small_peak.fetch_max(active_total.load(Ordering::SeqCst), Ordering::SeqCst);
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    } else {
                        let at_start = active_total.load(Ordering::SeqCst);
                        tokio::time::sleep(Duration::from_millis(30)).await;
                        let after_hold = active_total.load(Ordering::SeqCst);
                        large_overlaps
                            .lock()
                            .unwrap()
                            .push(at_start.max(after_hold));
                    }

                    active_total.fetch_sub(1, Ordering::SeqCst);
                    MockResponse::ok(body_map.get(&path).expect("known path"))
                }
            }
        })
        .await;

        let fx = DownloadFixture::new("elastic-funnel-drain");
        let items: Vec<DownloadItem> = entries
            .iter()
            .map(|(name, declared_size, bytes)| DownloadItem {
                url: server.url(&format!("/{name}")),
                file_path: (*name).to_string(),
                expected_md5: Some(md5_hex(bytes)),
                expected_size: Some(*declared_size),
            })
            .collect();

        let summary = run_batch_in_dir(
            &fx.client,
            fx.path(),
            items,
            entries.len(),
            &fast_retry_policy(0),
            &fx.signal,
        )
        .await
        .expect("mixed small+large funnel batch should succeed");

        let _ = server.stop().await;
        assert_eq!(summary.ok, 6);
        assert!(summary.failed.is_empty());
        assert_eq!(
            small_peak.load(Ordering::SeqCst),
            4,
            "all 4 small files should run concurrently"
        );
        assert_eq!(
            *large_overlaps.lock().unwrap(),
            vec![1, 1],
            "large files must run strictly sequentially with zero overlap"
        );

        let order = started_paths.lock().unwrap().clone();
        assert_eq!(order.len(), 6);
        assert!(
            order[..4].iter().all(|p| p.starts_with("/small_")),
            "all small files must start before large files: {order:?}"
        );
        assert_eq!(
            &order[4..],
            &["/large_1.bin", "/large_2.bin"],
            "large files must follow in ascending size order: {order:?}"
        );

        for (name, _, bytes) in &entries {
            fx.assert_downloaded(name, bytes);
        }
    }

    #[tokio::test]
    async fn test_elastic_shared_rate_limiter_caps_aggregate_bandwidth() {
        let chunk_size = 10 * 1024usize;
        let payloads: Vec<(String, Vec<u8>)> = (0..4)
            .map(|idx| {
                let bytes: Vec<u8> = (0..chunk_size)
                    .map(|i| ((i + idx * 17) % 251) as u8)
                    .collect();
                (format!("paced_{idx}.bin"), bytes)
            })
            .collect();
        let body_map: Arc<std::collections::HashMap<String, Vec<u8>>> = Arc::new(
            payloads
                .iter()
                .map(|(name, bytes)| (format!("/{name}"), bytes.clone()))
                .collect(),
        );

        let server = ConcurrentMockServer::start(move |req| {
            let body_map = Arc::clone(&body_map);
            async move {
                let path = req
                    .request_line
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("/")
                    .to_string();
                MockResponse::ok(body_map.get(&path).expect("known path"))
            }
        })
        .await;

        let fx = DownloadFixture::new("elastic-shared-rate-limit");
        let options = DownloadOptions {
            retry_policy: fast_retry_policy(0),
            limit_rate: Some(parse_rate_limit("100K").unwrap()),
        };
        let items: Vec<DownloadItem> = payloads
            .iter()
            .map(|(name, bytes)| DownloadItem {
                url: server.url(&format!("/{name}")),
                file_path: name.clone(),
                expected_md5: Some(md5_hex(bytes)),
                expected_size: Some(bytes.len() as u64),
            })
            .collect();

        let started = Instant::now();
        let summary = download_files_in_dir(
            &fx.client,
            fx.path(),
            items,
            payloads.len(),
            None,
            &options,
            &fx.signal,
        )
        .await
        .expect("rate-limited concurrent batch should succeed");
        let elapsed = started.elapsed();

        assert_eq!(server.stop().await.len(), 4);
        assert_eq!(summary.ok, 4);
        assert!(summary.failed.is_empty());
        assert!(
            elapsed >= Duration::from_millis(320),
            "4x10 KiB concurrent batch at 100 KiB/s aggregate (~400ms expected) finished too fast: {elapsed:?}"
        );
        for (name, bytes) in &payloads {
            fx.assert_downloaded(name, bytes);
        }
    }

    #[tokio::test]
    async fn test_elastic_http_429_retry_after_shared_backpressure_and_prefixed_logs() {
        let throttle_sent_at = Arc::new(std::sync::Mutex::new(None::<Instant>));
        let late_arrivals = Arc::new(std::sync::Mutex::new(Vec::<(String, Instant)>::new()));
        let f1_attempts = Arc::new(AtomicUsize::new(0));

        let payloads: Vec<(String, Vec<u8>)> = (1..=6)
            .map(|i| {
                (
                    format!("f{i}.bin"),
                    format!("body-for-file-{i}").into_bytes(),
                )
            })
            .collect();
        let body_map: Arc<std::collections::HashMap<String, Vec<u8>>> = Arc::new(
            payloads
                .iter()
                .map(|(name, bytes)| (format!("/{name}"), bytes.clone()))
                .collect(),
        );

        let server = ConcurrentMockServer::start({
            let body_map = Arc::clone(&body_map);
            let throttle_sent_at = Arc::clone(&throttle_sent_at);
            let late_arrivals = Arc::clone(&late_arrivals);
            let f1_attempts = Arc::clone(&f1_attempts);
            move |req| {
                let body_map = Arc::clone(&body_map);
                let throttle_sent_at = Arc::clone(&throttle_sent_at);
                let late_arrivals = Arc::clone(&late_arrivals);
                let f1_attempts = Arc::clone(&f1_attempts);
                async move {
                    let path = req
                        .request_line
                        .split_whitespace()
                        .nth(1)
                        .unwrap_or("/")
                        .to_string();
                    if path == "/f1.bin" && f1_attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                        *throttle_sent_at.lock().unwrap() = Some(Instant::now());
                        return MockResponse::retry_after(
                            "429 Too Many Requests",
                            "1",
                            b"slow down",
                        );
                    }
                    if path == "/f5.bin" || path == "/f6.bin" {
                        late_arrivals
                            .lock()
                            .unwrap()
                            .push((path.clone(), Instant::now()));
                    } else if path != "/f1.bin" {
                        // Hold initial peers slightly so f1's 429 lands before f5/f6 dispatch.
                        tokio::time::sleep(Duration::from_millis(40)).await;
                    }
                    MockResponse::ok(body_map.get(&path).expect("known path"))
                }
            }
        })
        .await;

        let fx = DownloadFixture::new("elastic-429-backpressure");
        let options = DownloadOptions {
            retry_policy: RetryPolicy {
                max_retries: 2,
                initial_delay: Duration::from_millis(20),
                max_delay: Duration::from_millis(40),
                max_retry_after: Duration::from_millis(180),
            },
            limit_rate: None,
        };
        let captured = CapturedLogs::new();
        let logger = VerboseLogger::with_writer(captured.clone_writer());

        let items: Vec<DownloadItem> = payloads
            .iter()
            .enumerate()
            .map(|(idx, (name, bytes))| DownloadItem {
                url: server.url(&format!("/{name}")),
                file_path: name.clone(),
                expected_md5: Some(md5_hex(bytes)),
                // Increasing small sizes ensure f1..f4 dispatch first and f5..f6 follow.
                expected_size: Some(64 + idx as u64),
            })
            .collect();

        let summary = download_files_in_dir_with_logger(
            &fx.client,
            fx.path(),
            items,
            payloads.len(),
            None,
            &options,
            &fx.signal,
            &logger,
        )
        .await
        .expect("batch should recover from 429 backpressure");

        let _ = server.stop().await;
        assert_eq!(summary.ok, 6);
        assert!(summary.failed.is_empty());
        for (name, bytes) in &payloads {
            fx.assert_downloaded(name, bytes);
        }

        let throttled_at = throttle_sent_at
            .lock()
            .unwrap()
            .expect("429 should have been sent");
        let arrivals = late_arrivals.lock().unwrap().clone();
        assert_eq!(arrivals.len(), 2);
        for (path, arrived_at) in arrivals {
            let delta = arrived_at.saturating_duration_since(throttled_at);
            assert!(
                delta >= Duration::from_millis(140),
                "later file {path} should wait for shared 180ms cooldown after 429, delta={delta:?}"
            );
        }

        assert!(
            !captured.as_bytes().contains(&0x1b),
            "verbose logs must remain plain text"
        );
        let logs = captured.text();
        assert!(
            logs.contains("[verbose] http: [f1.bin] -> GET "),
            "concurrent HTTP logs should be prefixed with file path:\n{logs}"
        );
        assert!(
            logs.contains("[verbose] retry: [f1.bin] attempt 1/2"),
            "concurrent retry logs should be prefixed with file path:\n{logs}"
        );
        assert!(
            logs.contains("[verbose] scheduler: [f1.bin] backpressure after HTTP 429"),
            "scheduler backpressure logs should be prefixed with file path:\n{logs}"
        );
    }

    #[tokio::test]
    async fn test_elastic_local_skips_do_not_recover_throttled_ceiling() {
        let s0_attempts = Arc::new(AtomicUsize::new(0));
        let s1_attempts = Arc::new(AtomicUsize::new(0));
        let throttle_barrier = Arc::new(tokio::sync::Barrier::new(2));
        let after_active = Arc::new(AtomicUsize::new(0));
        let after_peak = Arc::new(AtomicUsize::new(0));

        let server = ConcurrentMockServer::start({
            let s0_attempts = Arc::clone(&s0_attempts);
            let s1_attempts = Arc::clone(&s1_attempts);
            let throttle_barrier = Arc::clone(&throttle_barrier);
            let after_active = Arc::clone(&after_active);
            let after_peak = Arc::clone(&after_peak);
            move |req| {
                let s0_attempts = Arc::clone(&s0_attempts);
                let s1_attempts = Arc::clone(&s1_attempts);
                let throttle_barrier = Arc::clone(&throttle_barrier);
                let after_active = Arc::clone(&after_active);
                let after_peak = Arc::clone(&after_peak);
                async move {
                    let path = req
                        .request_line
                        .split_whitespace()
                        .nth(1)
                        .unwrap_or("/")
                        .to_string();
                    if path == "/s0.bin" {
                        if s0_attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                            let _ = tokio::time::timeout(
                                Duration::from_secs(2),
                                throttle_barrier.wait(),
                            )
                            .await;
                            return MockResponse::retry_after("429 Too Many Requests", "0", b"");
                        }
                        return MockResponse::ok(b"s0-ok");
                    }
                    if path == "/s1.bin" {
                        if s1_attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                            let _ = tokio::time::timeout(
                                Duration::from_secs(2),
                                throttle_barrier.wait(),
                            )
                            .await;
                            return MockResponse::retry_after("429 Too Many Requests", "0", b"");
                        }
                        return MockResponse::ok(b"s1-ok");
                    }
                    if path.starts_with("/after_") {
                        let cur = after_active.fetch_add(1, Ordering::SeqCst) + 1;
                        after_peak.fetch_max(cur, Ordering::SeqCst);
                        tokio::time::sleep(Duration::from_millis(35)).await;
                        after_active.fetch_sub(1, Ordering::SeqCst);
                        return MockResponse::ok(b"after-ok");
                    }
                    MockResponse::status("404 Not Found", b"")
                }
            }
        })
        .await;

        let fx = DownloadFixture::new("elastic-skips-no-recovery");
        for idx in 1..=4 {
            fx.write(format!("skip_{idx}.bin"), b"already-complete");
        }

        // s0 and s1 (8 MiB, cost 2 each) fill all 4 permits and both hit 429 -> ceiling 4 -> 2 -> 1.
        // Once they succeed (clean_successes = 2), skip_1..skip_4 (16 MiB) run as local skips.
        // Finally after_1 and after_2 (24 MiB, base cost 2) run: they overlap only if skips wrongly
        // recovered ceiling to 2, and stay sequential (peak == 1) when ceiling stays 1.
        let mut items = vec![
            DownloadItem {
                url: server.url("/s0.bin"),
                file_path: "s0.bin".to_string(),
                expected_md5: Some(md5_hex(b"s0-ok")),
                expected_size: Some(SMALL_FILE_MAX_BYTES),
            },
            DownloadItem {
                url: server.url("/s1.bin"),
                file_path: "s1.bin".to_string(),
                expected_md5: Some(md5_hex(b"s1-ok")),
                expected_size: Some(SMALL_FILE_MAX_BYTES),
            },
        ];
        for idx in 1..=4 {
            items.push(DownloadItem {
                url: server.url(&format!("/skip_{idx}.bin")),
                file_path: format!("skip_{idx}.bin"),
                expected_md5: Some(md5_hex(b"already-complete")),
                expected_size: Some(2 * SMALL_FILE_MAX_BYTES + idx as u64),
            });
        }
        for idx in 1..=2 {
            items.push(DownloadItem {
                url: server.url(&format!("/after_{idx}.bin")),
                file_path: format!("after_{idx}.bin"),
                expected_md5: Some(md5_hex(b"after-ok")),
                expected_size: Some(3 * SMALL_FILE_MAX_BYTES + idx as u64),
            });
        }

        let summary = run_batch_in_dir(
            &fx.client,
            fx.path(),
            items,
            8,
            &fast_retry_policy(3),
            &fx.signal,
        )
        .await
        .expect("batch should complete");

        let _ = server.stop().await;
        assert_eq!(summary.ok, 4);
        assert_eq!(summary.skipped, 4);
        assert!(summary.failed.is_empty());
        assert_eq!(
            after_peak.load(Ordering::SeqCst),
            1,
            "local skips must not recover effective_ceiling from 1"
        );
    }

    #[tokio::test]
    async fn test_elastic_batch_summary_failed_preserves_original_metadata_order() {
        let server = ConcurrentMockServer::from_routes(vec![
            (
                "/z_large_fail.bin",
                vec![MockResponse::status("404 Not Found", b"")],
            ),
            ("/m_medium_ok.bin", vec![MockResponse::ok(b"medium-ok")]),
            (
                "/a_small_fail.bin",
                vec![MockResponse::status("404 Not Found", b"")],
            ),
            ("/b_small_ok.bin", vec![MockResponse::ok(b"small-ok")]),
            (
                "/u_unknown_fail.bin",
                vec![MockResponse::status("404 Not Found", b"")],
            ),
        ])
        .await;

        let fx = DownloadFixture::new("elastic-failed-order");
        let items = vec![
            DownloadItem {
                url: server.url("/z_large_fail.bin"),
                file_path: "z_large_fail.bin".to_string(),
                expected_md5: None,
                expected_size: Some(20 * 1024 * 1024),
            },
            DownloadItem {
                url: server.url("/m_medium_ok.bin"),
                file_path: "m_medium_ok.bin".to_string(),
                expected_md5: Some(md5_hex(b"medium-ok")),
                expected_size: Some(2 * 1024 * 1024),
            },
            DownloadItem {
                url: server.url("/a_small_fail.bin"),
                file_path: "a_small_fail.bin".to_string(),
                expected_md5: None,
                expected_size: Some(64),
            },
            DownloadItem {
                url: server.url("/b_small_ok.bin"),
                file_path: "b_small_ok.bin".to_string(),
                expected_md5: Some(md5_hex(b"small-ok")),
                expected_size: Some(128),
            },
            DownloadItem {
                url: server.url("/u_unknown_fail.bin"),
                file_path: "u_unknown_fail.bin".to_string(),
                expected_md5: None,
                expected_size: None,
            },
        ];

        let summary = run_batch_in_dir(
            &fx.client,
            fx.path(),
            items,
            5,
            &fast_retry_policy(0),
            &fx.signal,
        )
        .await
        .expect("batch should complete and report failures");

        assert_eq!(server.stop().await.len(), 5);
        assert_eq!(summary.ok, 2);
        assert_eq!(summary.skipped, 0);
        assert!(!summary.interrupted);
        let failed_paths: Vec<&str> = summary
            .failed
            .iter()
            .map(|failed| failed.file_path.as_str())
            .collect();
        assert_eq!(
            failed_paths,
            vec!["z_large_fail.bin", "a_small_fail.bin", "u_unknown_fail.bin"]
        );
    }

    #[tokio::test]
    async fn test_elastic_concurrent_shutdown_signal_cancels_cleanly_without_dispatching_remaining()
    {
        let first_chunk = b"partial-concurrent-bytes";
        let second_chunk = b"-never-finished";
        let total_len = first_chunk.len() + second_chunk.len();

        let fx = DownloadFixture::new("elastic-concurrent-ctrl-c");
        let part_paths: Vec<PathBuf> = (0..4)
            .map(|idx| fx.part_path(&format!("file_{idx}.bin")))
            .collect();
        let all_parts = Arc::new(part_paths.clone());
        let signal_for_trigger = fx.signal.clone();

        // Wait until all 4 concurrent transfers have written their partial chunks, then trigger Ctrl+C.
        let trigger_task = tokio::spawn(async move {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
            while tokio::time::Instant::now() < deadline {
                if all_parts.iter().all(|path| {
                    fs::metadata(path)
                        .map(|m| m.len() >= first_chunk.len() as u64)
                        .unwrap_or(false)
                }) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            signal_for_trigger.trigger();
        });

        let server = ConcurrentMockServer::start({
            let root = fx.path().to_path_buf();
            let signal = fx.signal.clone();
            move |req| {
                let root = root.clone();
                let signal = signal.clone();
                async move {
                    let path = req
                        .request_line
                        .split_whitespace()
                        .nth(1)
                        .unwrap_or("/")
                        .trim_start_matches('/')
                        .to_string();
                    MockResponse::MidStreamInterrupt {
                        status_line: "200 OK",
                        headers: vec![("Content-Length".to_string(), total_len.to_string())],
                        first_chunk: first_chunk.to_vec(),
                        second_chunk: Some(second_chunk.to_vec()),
                        watch_path: root.join(format!("{path}.part")),
                        expected_part_len: u64::MAX, // Wait until external trigger_task fires shutdown
                        signal,
                    }
                }
            }
        })
        .await;

        let items: Vec<DownloadItem> = (0..6)
            .map(|idx| DownloadItem {
                url: server.url(&format!("/file_{idx}.bin")),
                file_path: format!("file_{idx}.bin"),
                expected_md5: None,
                expected_size: Some(total_len as u64 + idx as u64),
            })
            .collect();

        let started = Instant::now();
        let summary = run_batch_in_dir(
            &fx.client,
            fx.path(),
            items,
            6,
            &fast_retry_policy(0),
            &fx.signal,
        )
        .await
        .expect("interrupted concurrent batch should return summary");
        let elapsed = started.elapsed();

        trigger_task.await.unwrap();
        let requests = server.stop().await;

        assert!(
            elapsed < Duration::from_secs(3),
            "concurrent shutdown should finish promptly, took {elapsed:?}"
        );
        assert_eq!(
            requests.len(),
            4,
            "remaining 2 queued files must not be dispatched after shutdown"
        );
        assert!(summary.interrupted);
        assert_eq!(summary.ok, 0);
        assert!(summary.failed.is_empty());
        assert!(!fx.signal.has_active_multi_progress());

        for (idx, part_path) in part_paths.iter().enumerate() {
            assert!(!fx.join(format!("file_{idx}.bin")).exists());
            assert_eq!(fs::read(part_path).unwrap(), first_chunk);
        }
        for idx in 4..6 {
            assert!(!fx.join(format!("file_{idx}.bin")).exists());
            assert!(!fx.part_path(&format!("file_{idx}.bin")).exists());
        }
    }
}
