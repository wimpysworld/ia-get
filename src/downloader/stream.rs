//! Streaming an HTTP response body into a local file, with resume and
//! retry, plus the helpers that decide how a response's body is to be used.

use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

use colored::*;
use indicatif::ProgressBar;
use reqwest::header::{CONTENT_RANGE, HeaderMap, HeaderValue, RANGE, RETRY_AFTER};
use reqwest::{Client, Method, Response, StatusCode};

use crate::Result;
use crate::cookie::{CookieSource, cookie_header_for, with_cookie};
use crate::display::{branch_glyph, create_progress_bar, last_glyph, print_downloaded_line};
use crate::downloader::mtime::parse_last_modified;
use crate::downloader::rate::RateLimiter;
use crate::downloader::retry::{INTERRUPT_CHECK_INTERVAL, RetryTracker, parse_retry_after};
use crate::error::IaGetError;
use crate::verbose;
use url::Url;

/// Resolves once the batch has been asked to stop, so a stalled body read
/// can be aborted at the next check instead of waiting for the next chunk
/// (or the read timeout).
async fn wait_for_stop(running: &Arc<AtomicBool>) {
    while running.load(Ordering::SeqCst) {
        tokio::time::sleep(INTERRUPT_CHECK_INTERVAL).await;
    }
}

/// Streams the response body into `file`, updating the progress bar as data
/// arrives. Returns the number of new bytes written. A user interruption
/// aborts with `IaGetError::Interrupted`; partial data stays in the file so
/// the next attempt can resume.
///
/// The stop flag is raced against every chunk read: a body that stalls
/// mid-transfer no longer forces a wait for the next chunk or the read
/// timeout after a Ctrl+C.
async fn stream_response_body(
    response: &mut Response,
    file: &mut File,
    base_size: u64,
    pb: &ProgressBar,
    running: &Arc<AtomicBool>,
    rate: &mut RateLimiter,
) -> Result<u64> {
    let mut downloaded_bytes: u64 = 0;

    loop {
        tokio::select! {
            chunk = response.chunk() => match chunk {
                Ok(Some(chunk)) => {
                    file.write_all(&chunk)?;
                    let chunk_len = chunk.len() as u64;
                    downloaded_bytes += chunk_len;
                    pb.set_position(base_size + downloaded_bytes);
                    // Pace the transfer to the configured rate, if any, so a
                    // limited download does not saturate the connection.
                    // The pacing sleep is raced against the stop flag: with a
                    // low --limit-rate a chunk's budget can be minutes long,
                    // and a Ctrl+C must not wait it out.
                    tokio::select! {
                        _ = rate.pace(chunk_len) => {}
                        _ = wait_for_stop(running) => {
                            pb.finish_and_clear();
                            return Err(IaGetError::Interrupted);
                        }
                    }
                }
                Ok(None) => break,
                Err(e) => return Err(e.into()),
            },
            _ = wait_for_stop(running) => {
                pb.finish_and_clear();
                return Err(IaGetError::Interrupted);
            }
        }
    }

    Ok(downloaded_bytes)
}

/// Truncates a local file and moves its write cursor to the start, so a
/// fresh body is streamed over it instead of appended to stale data.
fn reset_local_file(file: &mut File) -> Result<()> {
    file.set_len(0)?;
    file.seek(SeekFrom::Start(0))?;
    Ok(())
}

/// A transfer's body could not be used: clear the progress bar, record the
/// retry, and leave the file positioned at its end so the next attempt
/// resumes from where this one stopped.
async fn retry_open_file(
    file: &mut File,
    pb: &ProgressBar,
    retry: &mut RetryTracker,
    kind: &str,
    detail: &str,
    retry_after_secs: Option<u64>,
    running: &Arc<AtomicBool>,
) -> Result<()> {
    pb.finish_and_clear();
    retry
        .record(kind, detail, retry_after_secs, running)
        .await?;
    file.seek(SeekFrom::End(0))?;
    Ok(())
}

/// Progress bar label for a download attempt: "Resuming" when a Range
/// request continues an existing `.part` file, "Downloading" otherwise.
fn download_action_label(resuming: bool) -> String {
    if resuming {
        format!("{} {}     ", last_glyph(), "Resuming".white())
    } else {
        format!("{} {}  ", last_glyph(), "Downloading".white())
    }
}

/// True when the error is a disk-full / no-space condition that retrying
/// cannot fix (ENOSPC on Linux, ERROR_DISK_FULL on Windows).
fn is_disk_full_error(err: &IaGetError) -> bool {
    matches!(
        err,
        IaGetError::FileSystem { source: Some(src), .. }
            if src
                .downcast_ref::<std::io::Error>()
                .is_some_and(|io| io.kind() == std::io::ErrorKind::StorageFull)
    )
}

/// The redirect cap a manually followed chain may reach (reqwest's default
/// automatic policy cap)
const MAX_REDIRECTS: u32 = 10;

/// Sends a request and follows up to `MAX_REDIRECTS` redirects manually,
/// re-resolving the `Cookie` header against every target URL: reqwest's
/// automatic policy would carry the original request's fixed header into
/// each hop, bypassing the path/domain scoping of parsed cookies — and, on
/// a redirect that leaves archive.org, leaking them to the foreign host.
/// The caller's client must have automatic following disabled
/// (`ClientBuilder::redirect(Policy::none())`, see build_client in
/// main.rs); this function is the only place a 3xx is consumed. The
/// `Range` header (when any) rides along: redirects here name the same
/// resource. A `timeout`, when given, applies per hop.
pub(crate) async fn send_following_redirects(
    client: &Client,
    method: Method,
    url: &Url,
    range: Option<&HeaderValue>,
    cookie_source: Option<&CookieSource>,
    timeout: Option<Duration>,
) -> Result<Response> {
    let mut url = url.clone();
    let mut redirects = 0u32;
    loop {
        let cookie_header = cookie_header_for(cookie_source, &url)?;
        let mut request = with_cookie(
            client.request(method.clone(), url.clone()),
            cookie_header.as_ref(),
        );
        if let Some(range) = range {
            request = request.header(RANGE, range.clone());
        }
        if let Some(timeout) = timeout {
            request = request.timeout(timeout);
        }
        let response = request.send().await.map_err(IaGetError::from)?;

        let status = response.status();
        if !matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308) {
            return Ok(response);
        }
        redirects += 1;
        if redirects > MAX_REDIRECTS {
            return Err(IaGetError::Network {
                detail: format!("redirect limit exceeded ({MAX_REDIRECTS} redirects)"),
                source: None,
            });
        }
        let location = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .ok_or(IaGetError::Network {
                detail: format!("HTTP {status} without a usable Location header"),
                source: None,
            })?;
        url = url.join(location).map_err(|e| IaGetError::Network {
            detail: format!("invalid redirect target {location:?}: {e}"),
            source: None,
        })?;
        verbose::log(&format!("  -> {status} redirect to {url}"));
    }
}

/// Handles a non-success response.
///
/// A 416 on a resume is fatal: the local prefix is not a valid one, so the
/// caller must re-download from scratch. A retryable status is recorded for
/// another attempt. Any other status is a fatal network error.
async fn handle_failed_status(
    status: StatusCode,
    resuming: bool,
    retry_after: Option<u64>,
    retry: &mut RetryTracker,
    running: &Arc<AtomicBool>,
) -> Result<()> {
    if resuming && status == StatusCode::RANGE_NOT_SATISFIABLE {
        return Err(IaGetError::RangeNotSatisfiable);
    }

    if !is_retryable_status(status) {
        return Err(IaGetError::Network {
            detail: format!(
                "Server responded with HTTP {} {}",
                status,
                status.canonical_reason().unwrap_or("unknown status")
            ),
            source: None,
        });
    }

    let reason = status
        .canonical_reason()
        .unwrap_or("unknown status")
        .to_string();
    retry
        .record(&format!("HTTP {status}"), &reason, retry_after, running)
        .await
}

/// Whether the local prefix may be appended to the server's successful
/// body: only a 206 whose `Content-Range` starts exactly where the file
/// ends. A full-body 200 replaces the file. (Only 200/206 reach this
/// point: every other status is rejected before streaming.)
fn prefix_trusted(status: StatusCode, headers: &HeaderMap, local_size: u64) -> bool {
    match status {
        StatusCode::OK => false,
        StatusCode::PARTIAL_CONTENT => partial_content_offset(headers, local_size) == Some(true),
        _ => false,
    }
}

/// The progress bar's total: the server-announced length (the bytes this
/// response will send, added to the local prefix) when present, else the
/// metadata size, else the current size (an unknown-size file has no usable
/// total, matching the previous behaviour).
///
/// The add is saturating: a hostile or broken `Content-Length` of
/// `u64::MAX` must not overflow a non-empty local prefix (a debug-build
/// panic or a release wrap would both misread the transfer).
fn progress_total(content_length: Option<u64>, base_size: u64, expected_size: Option<u64>) -> u64 {
    content_length
        .map(|remaining| base_size.saturating_add(remaining))
        .or(expected_size)
        .unwrap_or(base_size)
}

/// How to treat a streamed successful body
enum StreamedBody {
    /// The body is complete; the download finished
    Complete,
    /// The server returned no data at all; retry
    Empty,
    /// The body ended before the metadata size; resume where it stopped
    Incomplete { received: u64, expected: u64 },
}

/// Decides whether a streamed body is complete.
///
/// A zero-byte body is a server malfunction only when the metadata expects
/// data: a zero-byte file with no `<size>` is indistinguishable from a
/// dropped body, so the unknown-size case is trusted (an MD5, when present,
/// still verifies the result). A body shorter than the metadata size was
/// truncated.
fn assess_streamed_body(
    downloaded_bytes: u64,
    base_size: u64,
    expected_size: Option<u64>,
) -> StreamedBody {
    if downloaded_bytes == 0 && base_size == 0 && expected_size.is_some_and(|expected| expected > 0)
    {
        return StreamedBody::Empty;
    }
    if let Some(expected) = expected_size
        && base_size + downloaded_bytes < expected
    {
        return StreamedBody::Incomplete {
            received: base_size + downloaded_bytes,
            expected,
        };
    }
    StreamedBody::Complete
}

/// Download file content with progress reporting and automatic retry on failure.
///
/// `retry_delay` computes the delay for a given 1-based retry attempt; tests
/// substitute near-instant delays for `backoff_delay`.
///
/// Only a successful (2xx) response body is ever written to `file`. Error
/// pages are discarded, empty and truncated bodies are retried, and a 200
/// response to a ranged request resets the file instead of appending to it.
///
/// Returns, when the server sent one on the final successful response, the
/// parsed `Last-Modified` header value.
///
/// `rate_limit` (bytes/second, from `--limit-rate`) throttles the streamed
/// body when `Some(_ > 0)`; `None` or `Some(0)` leaves it unlimited.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn download_file_content(
    client: &Client,
    url: &str,
    file: &mut File,
    running: &Arc<AtomicBool>,
    cookie_source: Option<&CookieSource>,
    expected_size: Option<u64>,
    retry_delay: fn(u32) -> Duration,
    rate_limit: Option<u64>,
) -> Result<Option<SystemTime>> {
    // The session's cookies may be path-scoped (a parsed cookies.txt): the
    // sender resolves the header against every request URL (and every
    // redirect hop) — a cookies.txt entry can expire while a retry wait is
    // out (up to the 15-minute Retry-After cap), and a stale Cookie header
    // must not outlive it
    let url = Url::parse(url).map_err(|e| IaGetError::Network {
        detail: format!("invalid URL {url:?}: {e}"),
        source: None,
    })?;

    let mut retry = RetryTracker::new(retry_delay);
    let mut rate = RateLimiter::new(rate_limit);

    loop {
        // Re-check file size at start of each attempt (in case of retry)
        let current_file_size = file.metadata()?.len();
        let resuming = current_file_size > 0;
        let mut download_action = download_action_label(resuming);

        verbose::log(&match resuming {
            true => format!("GET {url} (Range: bytes={current_file_size}-)"),
            false => format!("GET {url}"),
        });

        // A plain GET for a fresh file, or a `Range: bytes=<size>-` resume
        // request when the local prefix is non-empty
        let range = if resuming {
            Some(
                HeaderValue::from_str(&format!("bytes={current_file_size}-")).map_err(|e| {
                    IaGetError::Network {
                        detail: format!("Invalid range header value: {e}"),
                        source: Some(Box::new(e)),
                    }
                })?,
            )
        } else {
            None
        };

        let mut response = match send_following_redirects(
            client,
            Method::GET,
            &url,
            range.as_ref(),
            cookie_source,
            None,
        )
        .await
        {
            Ok(response) => response,
            Err(e) => {
                verbose::log(&format!("  connection error: {e}"));
                // Request failed before we even got a response
                retry
                    .record("Connection error", &e.to_string(), None, running)
                    .await?;
                continue;
            }
        };

        let status = response.status();
        verbose::log(&format!("  -> HTTP {status}"));
        let retry_after = response
            .headers()
            .get(RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(parse_retry_after);
        // Captured on every successful response so the value returned to the
        // caller always comes from the attempt that completed the download.
        let last_modified = parse_last_modified(response.headers());

        if !status.is_success() {
            // Error pages must never be written to the file; drop the body.
            drop(response);
            // Ok: the failure was recorded and another attempt follows;
            // Err: a fatal status returned instead.
            handle_failed_status(status, resuming, retry_after, &mut retry, running).await?;
            continue;
        }

        // Only a full body (200) or a verified partial (206) may be
        // streamed: any other success status (a 204 or 304 to a ranged
        // request) carries no file data — accepting it would let the
        // `.part` "complete" on data it does not hold
        if !matches!(status, StatusCode::OK | StatusCode::PARTIAL_CONTENT) {
            drop(response);
            return Err(IaGetError::Network {
                detail: format!(
                    "Server responded with HTTP {} {}: not a downloadable body",
                    status,
                    status.canonical_reason().unwrap_or("unknown status")
                ),
                source: None,
            });
        }

        if resuming && !prefix_trusted(status, response.headers(), current_file_size) {
            match status {
                // The server ignored the Range request and is sending the
                // full body: the local prefix is untrusted, so the file is
                // reset before the body is streamed.
                StatusCode::OK => {
                    reset_local_file(file)?;
                    download_action = download_action_label(false);
                }
                // A 206 that does not continue the local prefix: its body
                // is a partial range of the file, not the file — streaming
                // it (into a reset file or not) can only yield a corrupt
                // result. Reset the file, discard the response, and retry
                // with an un-ranged GET.
                _ => {
                    reset_local_file(file)?;
                    drop(response);
                    retry
                        .record(
                            "Bad 206",
                            &format!(
                                "Content-Range does not continue the local prefix ({current_file_size} bytes); re-requesting from scratch"
                            ),
                            retry_after,
                            running,
                        )
                        .await?;
                    continue;
                }
            }
        }
        let base_size = file.metadata()?.len();

        let pb = create_progress_bar(
            progress_total(response.content_length(), base_size, expected_size),
            &download_action,
            "green/green",
            true,
        );
        // Set initial progress to current file size for resumed downloads
        pb.set_position(base_size);

        let start_time = Instant::now();
        match stream_response_body(&mut response, file, base_size, &pb, running, &mut rate).await {
            Ok(downloaded_bytes) => {
                match assess_streamed_body(downloaded_bytes, base_size, expected_size) {
                    StreamedBody::Complete => {
                        pb.finish_and_clear();
                        print_downloaded_line(
                            branch_glyph(),
                            downloaded_bytes,
                            Some(start_time.elapsed()),
                        );
                        return Ok(last_modified);
                    }
                    StreamedBody::Empty => {
                        retry_open_file(
                            file,
                            &pb,
                            &mut retry,
                            "Empty response",
                            "server returned no data",
                            retry_after,
                            running,
                        )
                        .await?;
                    }
                    StreamedBody::Incomplete { received, expected } => {
                        // The body ended before the announced size: the
                        // transfer was truncated, so resume from where it
                        // stopped — honoring this response's Retry-After
                        // when the server asked for one.
                        retry_open_file(
                            file,
                            &pb,
                            &mut retry,
                            "Incomplete body",
                            &format!("received {received} of {expected} bytes"),
                            retry_after,
                            running,
                        )
                        .await?;
                    }
                }
                continue;
            }
            Err(e) => {
                // A user interruption aborts the run; a disk-full condition
                // cannot be fixed by retrying (the same ENOSPC /
                // ERROR_DISK_FULL would recur).
                if matches!(e, IaGetError::Interrupted) || is_disk_full_error(&e) {
                    pb.finish_and_clear();
                    return Err(e);
                }

                // Mid-stream failure: keep the partial data and resume later.
                retry_open_file(
                    file,
                    &pb,
                    &mut retry,
                    "Download error",
                    &e.to_string(),
                    None,
                    running,
                )
                .await?;
            }
        }
    }
}

/// Returns true for HTTP statuses that are worth retrying (transient server
/// or client throttling problems). Other 4xx/5xx statuses are fatal.
fn is_retryable_status(status: StatusCode) -> bool {
    matches!(status.as_u16(), 408 | 425 | 429 | 500 | 502 | 503 | 504)
}

/// Parses and validates a `Content-Range` header of the form
/// `bytes <start>-<end>/<total>` (or `bytes <start>-<end>/*`), which a 206
/// response must carry.
///
/// Returns `Some(true)` when the offset equals `expected_start`,
/// `Some(false)` when it differs, and `None` when the header is absent or
/// malformed in ANY field — the end must parse and reach at least the
/// start, and a numeric total must exceed the (inclusive) end, since a
/// range `start-end/total` cannot end at or past the file's last index
/// (`total - 1`) — so a mangled range can never certify the offset. In the
/// `None` case the caller treats the body as untrusted.
fn partial_content_offset(headers: &HeaderMap, expected_start: u64) -> Option<bool> {
    headers
        .get(CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| {
            let rest = value.strip_prefix("bytes ")?;
            let (range, total) = rest.split_once('/')?;
            let (start, end) = range.split_once('-')?;
            let start: u64 = start.parse().ok()?;
            let end: u64 = end.parse().ok()?;
            if end < start {
                return None;
            }
            // A known total names the file's last valid index as total - 1,
            // so the (inclusive) end must stay below it: total == end (or
            // worse) is a malformed range that must not certify the offset.
            if total != "*" && total.parse::<u64>().ok()? <= end {
                return None;
            }
            Some(start == expected_start)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::downloader::retry::MAX_RETRIES;
    use crate::test_support::{
        MockBody, MockResponse, MockServer, file_server, ok_empty, run_download,
        run_download_with_rate, test_running,
    };
    use std::collections::VecDeque;
    use std::fs;
    use std::time::{Duration, Instant};

    #[tokio::test]
    async fn http_500_body_is_not_written_to_file() {
        let content = b"0123456789abcdef";
        let nginx_error =
            b"<html><head><title>500 Internal Server Error</title></head><body>nginx</body></html>";

        let (server, dir) = file_server(
            "http_500",
            VecDeque::from(vec![
                MockResponse::new(500, MockBody::Full(nginx_error.to_vec())),
                MockResponse::new(500, MockBody::Full(nginx_error.to_vec())),
                MockResponse::new(200, MockBody::Full(content.to_vec())),
            ]),
            ok_empty(),
        );
        let part = dir.join("file.bin.part");
        let result = run_download(
            &server.url("/file.bin"),
            part.to_str().unwrap(),
            Some(content.len() as u64),
            &test_running(),
        )
        .await;

        assert!(result.is_ok(), "expected success, got {:?}", result.err());
        let data = fs::read(&part).unwrap();
        assert_eq!(data, content, "error body must not be written to the file");
        assert_eq!(server.request_count(), 3);
    }

    #[tokio::test]
    async fn empty_response_retries_then_fails() {
        // Fallback also returns an empty 200 so every retry sees the same
        let (server, dir) = file_server(
            "empty_response",
            VecDeque::from(vec![MockResponse::new(200, MockBody::Full(vec![]))]),
            ok_empty(),
        );
        let part = dir.join("file.bin.part");
        let result = run_download(
            &server.url("/file.bin"),
            part.to_str().unwrap(),
            Some(10),
            &test_running(),
        )
        .await;

        assert!(
            matches!(result, Err(IaGetError::Network { .. })),
            "expected a Network error, got {:?}",
            result.ok()
        );
        assert_eq!(fs::metadata(&part).unwrap().len(), 0);
        assert_eq!(server.request_count(), 1 + MAX_RETRIES as usize);
    }

    #[tokio::test]
    async fn empty_response_with_unknown_size_is_accepted() {
        // A zero-byte file whose metadata carries no <size> must not loop on
        // "Empty response": with no size to compare against, the empty 200
        // is the only signal we have, so it is trusted (an MD5, when
        // present, still verifies the result).
        let (server, dir) = file_server(
            "empty_unknown_size",
            VecDeque::from(vec![MockResponse::new(200, MockBody::Full(vec![]))]),
            ok_empty(),
        );
        let part = dir.join("file.bin.part");
        let result = run_download(
            &server.url("/file.bin"),
            part.to_str().unwrap(),
            None,
            &test_running(),
        )
        .await;

        assert!(
            result.is_ok(),
            "an empty body with unknown size must succeed, got {:?}",
            result.err()
        );
        assert_eq!(
            server.request_count(),
            1,
            "a trusted empty body must not be retried"
        );
        assert_eq!(fs::metadata(&part).unwrap().len(), 0);
    }

    #[tokio::test]
    async fn resume_after_mid_stream_disconnect() {
        let full = b"01234567890123456789"; // 20 bytes

        let (server, dir) = file_server(
            "mid_stream",
            VecDeque::from(vec![
                MockResponse::new(
                    200,
                    MockBody::Truncated {
                        announced_len: full.len() as u64,
                        partial: full[..8].to_vec(),
                    },
                ),
                // Range-aware: the 206 tail and Content-Range are derived from
                // the client's Range request, so the resume offset is verified
                // against behaviour rather than script order.
                MockResponse::ranged(full.to_vec()),
            ]),
            MockResponse::new(206, MockBody::Full(vec![])),
        );
        let part = dir.join("file.bin.part");
        let result = run_download(
            &server.url("/file.bin"),
            part.to_str().unwrap(),
            Some(full.len() as u64),
            &test_running(),
        )
        .await;

        result.expect("download should succeed");
        assert_eq!(fs::read(&part).unwrap(), full);
        assert_eq!(server.ranges(), vec![None, Some(8)]);
    }

    #[tokio::test]
    async fn resume_sends_correct_offset_to_range_aware_server() {
        // The mock honors the Range header like a real origin: a wrong resume
        // offset would yield a mismatched 206 that resets the file rather than
        // a silently corrupted tail, so the exact offset the client requests
        // is what is verified here.
        let full = b"01234567890123456789"; // 20 bytes
        let prefix = &full[..8]; // a .part that already holds 8 bytes

        let (server, dir) = file_server(
            "range_aware_resume",
            VecDeque::from(vec![MockResponse::ranged(full.to_vec())]),
            ok_empty(),
        );
        let part = dir.join("file.bin.part");
        fs::write(&part, prefix).unwrap();

        let result = run_download(
            &server.url("/file.bin"),
            part.to_str().unwrap(),
            Some(full.len() as u64),
            &test_running(),
        )
        .await;

        result.expect("resume must succeed against a Range-aware server");
        assert_eq!(
            fs::read(&part).unwrap(),
            full,
            "the resume must append the correct tail, not a wrong offset"
        );
        assert_eq!(
            server.ranges(),
            vec![Some(8)],
            "the resume must request bytes=8-, exactly the .part size"
        );
    }

    #[tokio::test]
    async fn partial_content_offset_mismatch_resets_file() {
        // A 206 whose Content-Range does not continue our prefix must not
        // be appended: the file is reset and re-downloaded from scratch.
        let full = b"0123456789"; // 10 bytes

        let (server, dir) = file_server(
            "cr_mismatch",
            VecDeque::from(vec![
                MockResponse::new(206, MockBody::Full(full.to_vec()))
                    .with_header("Content-Range", "bytes 0-9/10"),
            ]),
            // The bad 206 is discarded: the retry is an un-ranged GET,
            // which the fallback serves in full
            MockResponse::new(200, MockBody::Full(full.to_vec())),
        );
        let part = dir.join("file.bin.part");
        fs::write(&part, b"XXXX").unwrap(); // 4-byte "partial" file
        let result = run_download(
            &server.url("/file.bin"),
            part.to_str().unwrap(),
            Some(full.len() as u64),
            &test_running(),
        )
        .await;

        result.expect("download should succeed");
        assert_eq!(
            fs::read(&part).unwrap(),
            full,
            "a mismatched 206 must be discarded and re-fetched, not appended"
        );
        assert_eq!(server.ranges(), vec![Some(4), None]);
    }

    #[tokio::test]
    async fn partial_content_without_range_header_resets_file() {
        // A 206 without a Content-Range header is malformed (RFC 7233 makes
        // it mandatory): the body is untrusted, the response is discarded,
        // and the next attempt is an un-ranged GET.
        let full = b"0123456789"; // 10 bytes

        let (server, dir) = file_server(
            "cr_missing",
            VecDeque::from(vec![MockResponse::new(206, MockBody::Full(full.to_vec()))]),
            MockResponse::new(200, MockBody::Full(full.to_vec())),
        );
        let part = dir.join("file.bin.part");
        fs::write(&part, b"XXXX").unwrap(); // 4-byte "partial" file
        let result = run_download(
            &server.url("/file.bin"),
            part.to_str().unwrap(),
            Some(full.len() as u64),
            &test_running(),
        )
        .await;

        result.expect("download should succeed");
        assert_eq!(
            fs::read(&part).unwrap(),
            full,
            "a 206 without Content-Range must be discarded and re-fetched"
        );
        assert_eq!(server.ranges(), vec![Some(4), None]);
    }

    #[tokio::test]
    async fn http_204_on_a_resume_is_fatal_not_complete() {
        // A 204 (no content) to a ranged request carries no body: it must
        // fail, not "complete" the download on the stale .part prefix — in
        // the unknown-size case nothing would ever contradict it
        let (server, dir) = file_server(
            "http_204",
            VecDeque::from(vec![MockResponse::new(204, MockBody::Full(vec![]))]),
            ok_empty(),
        );
        let part = dir.join("file.bin.part");
        fs::write(&part, b"XXXX").unwrap();
        let result = run_download(
            &server.url("/file.bin"),
            part.to_str().unwrap(),
            None,
            &test_running(),
        )
        .await;

        assert!(
            matches!(result, Err(IaGetError::Network { .. })),
            "expected a Network error, got {:?}",
            result.ok()
        );
        assert_eq!(server.request_count(), 1, "a 204 must not be retried");
    }

    #[tokio::test]
    async fn redirect_target_gets_the_cookie_header() {
        // The origin redirects; the target request must carry the cookie
        // header (re-resolved for the target URL), not lose it on the hop
        let content = b"redirected-content";
        let (target_server, _target_url) = MockServer::scripted(
            "/file.bin",
            vec![MockResponse::new(200, MockBody::Full(content.to_vec()))],
        );
        let (_origin_server, origin_url) = MockServer::scripted(
            "/file.bin",
            vec![
                MockResponse::new(302, MockBody::Full(vec![]))
                    .with_header("Location", target_server.url("/file.bin").as_str()),
                MockResponse::new(200, MockBody::Full(content.to_vec())),
            ],
        );
        let source = crate::cookie::CookieSource::Raw(HeaderValue::from_static("session=abc"));

        // Automatic following must be off: only then does the helper's
        // manual loop (and its per-hop cookie re-resolution) run
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("client build");
        send_following_redirects(&client, Method::GET, &origin_url, None, Some(&source), None)
            .await
            .expect("the redirect must be followed");

        assert_eq!(
            target_server.cookies(),
            vec![Some("session=abc".to_string())],
            "the redirect target must see the cookie header"
        );
    }

    #[tokio::test]
    async fn full_200_response_resets_untrusted_prefix() {
        let full = b"0123456789"; // 10 bytes

        let (server, dir) = file_server(
            "range_ignored",
            VecDeque::from(vec![MockResponse::new(200, MockBody::Full(full.to_vec()))]),
            ok_empty(),
        );
        let part = dir.join("file.bin.part");
        fs::write(&part, b"XXXXXX").unwrap(); // 6-byte "partial" file
        let result = run_download(
            &server.url("/file.bin"),
            part.to_str().unwrap(),
            Some(full.len() as u64),
            &test_running(),
        )
        .await;

        result.expect("download should succeed");
        assert_eq!(
            fs::read(&part).unwrap(),
            full,
            "200 response must replace the local prefix, not append to it"
        );
        assert_eq!(server.ranges(), vec![Some(6)]);
    }

    #[tokio::test]
    async fn http_416_yields_range_not_satisfiable() {
        let (server, dir) = file_server(
            "http_416",
            VecDeque::from(vec![MockResponse::new(
                416,
                MockBody::Full(b"Range not satisfiable".to_vec()),
            )]),
            MockResponse::new(416, MockBody::Full(vec![])),
        );
        let part = dir.join("file.bin.part");
        fs::write(&part, b"XXXX").unwrap();
        let result = run_download(
            &server.url("/file.bin"),
            part.to_str().unwrap(),
            Some(100),
            &test_running(),
        )
        .await;

        assert!(
            matches!(result, Err(IaGetError::RangeNotSatisfiable)),
            "expected RangeNotSatisfiable, got {:?}",
            result
        );
        assert_eq!(server.request_count(), 1, "416 must not be retried");
        assert_eq!(server.ranges(), vec![Some(4)]);
    }

    #[tokio::test]
    async fn http_404_fails_immediately() {
        let (server, dir) = file_server(
            "http_404",
            VecDeque::from(vec![MockResponse::new(
                404,
                MockBody::Full(b"not found".to_vec()),
            )]),
            MockResponse::new(404, MockBody::Full(vec![])),
        );
        let part = dir.join("file.bin.part");
        let result = run_download(
            &server.url("/file.bin"),
            part.to_str().unwrap(),
            Some(10),
            &test_running(),
        )
        .await;

        assert!(
            matches!(result, Err(IaGetError::Network { .. })),
            "expected a Network error, got {:?}",
            result.ok()
        );
        assert_eq!(server.request_count(), 1, "404 must not be retried");
        assert_eq!(
            fs::metadata(&part).unwrap().len(),
            0,
            "error body must not be written"
        );
    }

    #[tokio::test]
    async fn retry_after_header_is_respected() {
        let content = b"retry-after-content";

        let (server, dir) = file_server(
            "retry_after",
            VecDeque::from(vec![
                MockResponse::new(429, MockBody::Full(vec![])).with_header("Retry-After", "1"),
                MockResponse::new(200, MockBody::Full(content.to_vec())),
            ]),
            MockResponse::new(429, MockBody::Full(vec![])),
        );
        let part = dir.join("file.bin.part");
        let start = Instant::now();
        let result = run_download(
            &server.url("/file.bin"),
            part.to_str().unwrap(),
            Some(content.len() as u64),
            &test_running(),
        )
        .await;

        assert!(result.is_ok(), "expected success, got {:?}", result.err());
        // A loaded CI host may deliver the 1s sleep late, but it cannot
        // deliver it *early*: the threshold only needs to rule out skipping
        // the wait.
        assert!(
            start.elapsed() >= Duration::from_millis(500),
            "Retry-After: 1 was not honored (elapsed {:?})",
            start.elapsed()
        );
        assert_eq!(fs::read(&part).unwrap(), content);
    }

    #[tokio::test]
    async fn last_modified_header_is_captured() {
        let content = b"0123456789abcdef";

        let (server, dir) = file_server(
            "last_modified",
            VecDeque::from(vec![
                MockResponse::new(200, MockBody::Full(content.to_vec()))
                    .with_header("Last-Modified", "Wed, 21 Oct 2015 07:28:00 GMT"),
            ]),
            MockResponse::new(200, MockBody::Full(vec![])),
        );
        let part = dir.join("file.bin.part");
        let result = run_download(
            &server.url("/file.bin"),
            part.to_str().unwrap(),
            Some(content.len() as u64),
            &test_running(),
        )
        .await;

        let mtime = result.expect("download should succeed");
        assert_eq!(fs::read(&part).unwrap(), content);
        assert_eq!(
            mtime,
            httpdate::parse_http_date("Wed, 21 Oct 2015 07:28:00 GMT").ok()
        );
    }

    #[tokio::test]
    async fn interrupt_during_backoff_stops_retries() {
        // The server throttles forever; a Ctrl+C during the backoff wait must
        // end the retry loop instead of sending another request after it.
        let (server, dir) = file_server(
            "interrupt_backoff",
            VecDeque::from(vec![MockResponse::new(429, MockBody::Full(vec![]))]),
            MockResponse::new(429, MockBody::Full(vec![])),
        );
        let running = Arc::new(AtomicBool::new(true));
        let flag = running.clone();
        // Simulate the Ctrl+C right after the first 429 has been served (a
        // fixed timer would race the retry loop on fast machines): the
        // flag is then guaranteed to be observed inside a retry wait.
        let watcher = server.clone();
        tokio::spawn(async move {
            while watcher.request_count() < 1 {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            flag.store(false, Ordering::SeqCst);
        });

        let part = dir.join("file.bin.part");
        let result = run_download(
            &server.url("/file.bin"),
            part.to_str().unwrap(),
            Some(10),
            &running,
        )
        .await;

        assert!(
            matches!(result, Err(IaGetError::Interrupted)),
            "an interrupt during the backoff wait must abort the retries, got {result:?}"
        );
        assert!(
            (1..=3).contains(&server.request_count()),
            "at most one extra retry may follow the interrupt, got {}",
            server.request_count()
        );
    }

    #[tokio::test]
    async fn interrupt_during_stalled_body_stops_waiting_for_chunks() {
        // A connection that never delivers its body must not force a
        // Ctrl+C to wait out the read timeout: the stop flag is raced
        // against the chunk read.
        let (server, dir) = file_server(
            "stalled_body",
            VecDeque::from(vec![MockResponse::stalled(100)]),
            ok_empty(),
        );
        let running = Arc::new(AtomicBool::new(true));
        let flag = running.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            flag.store(false, Ordering::SeqCst);
        });

        let part = dir.join("file.bin.part");
        let start = Instant::now();
        let result = run_download(
            &server.url("/file.bin"),
            part.to_str().unwrap(),
            Some(10),
            &running,
        )
        .await;

        assert!(
            matches!(result, Err(IaGetError::Interrupted)),
            "a Ctrl+C during a stalled body must abort the download, got {:?}",
            result.ok()
        );
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "the abort must land within one interrupt check, not the read timeout: {:?}",
            start.elapsed()
        );
    }

    #[tokio::test]
    async fn interrupt_during_rate_pacing_stops_waiting() {
        // At 1 byte/s a 10-byte body paces ~10s: a Ctrl+C must cut the
        // pacing sleep, not wait the chunk's budget out.
        let content = b"0123456789";
        let (server, dir) = file_server(
            "interrupt_pacing",
            VecDeque::from(vec![MockResponse::new(
                200,
                MockBody::Full(content.to_vec()),
            )]),
            ok_empty(),
        );
        let part = dir.join("file.bin.part");
        let running = Arc::new(AtomicBool::new(true));
        let flag = running.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            flag.store(false, Ordering::SeqCst);
        });

        let start = Instant::now();
        let result = run_download_with_rate(
            &server.url("/file.bin"),
            part.to_str().unwrap(),
            Some(content.len() as u64),
            &running,
            Some(1),
        )
        .await;

        assert!(
            matches!(result, Err(IaGetError::Interrupted)),
            "a Ctrl+C during pacing must abort the download, got {result:?}"
        );
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "the pacing delay must not hold the interrupt: {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn partial_content_offset_parses_matching_and_mismatching_ranges() {
        fn headers(range: Option<&str>) -> HeaderMap {
            let mut headers = HeaderMap::new();
            if let Some(range) = range {
                headers.insert(
                    CONTENT_RANGE,
                    HeaderValue::from_str(range).expect("a static test string is a valid header"),
                );
            }
            headers
        }

        assert_eq!(
            partial_content_offset(&headers(Some("bytes 8-19/20")), 8),
            Some(true)
        );
        assert_eq!(
            partial_content_offset(&headers(Some("bytes 0-9/20")), 8),
            Some(false)
        );
        assert_eq!(
            partial_content_offset(&headers(Some("bytes 8-19/*")), 9),
            Some(false)
        );
        assert_eq!(partial_content_offset(&headers(None), 8), None);
        // A suffix-form range has no usable start offset: untrusted.
        assert_eq!(
            partial_content_offset(&headers(Some("bytes */20")), 8),
            None
        );
        // A range without a total is not what a 206 carries: untrusted.
        assert_eq!(
            partial_content_offset(&headers(Some("bytes 8-19")), 8),
            None
        );
        // A malformed END field must not certify the start offset: a
        // mangled range reads as untrusted, not as "starts at 8"
        assert_eq!(
            partial_content_offset(&headers(Some("bytes 8-x/20")), 8),
            None,
            "an unparsable end is malformed"
        );
        assert_eq!(
            partial_content_offset(&headers(Some("bytes 9-8/20")), 8),
            None,
            "an end before the start is malformed"
        );
        assert_eq!(
            partial_content_offset(&headers(Some("bytes 8-19/10")), 8),
            None,
            "a total shorter than the end is malformed"
        );
        assert_eq!(
            partial_content_offset(&headers(Some("bytes 8-20/20")), 8),
            None,
            "a total equal to the (inclusive) end is malformed"
        );
    }

    #[test]
    fn is_retryable_status_covers_server_errors() {
        for code in [408u16, 425, 429, 500, 502, 503, 504] {
            assert!(
                is_retryable_status(StatusCode::from_u16(code).unwrap()),
                "{} must be retryable",
                code
            );
        }
        for code in [200u16, 206, 301, 400, 401, 403, 404, 416] {
            assert!(
                !is_retryable_status(StatusCode::from_u16(code).unwrap()),
                "{} must not be retryable",
                code
            );
        }
    }

    #[test]
    fn progress_total_saturates_on_hostile_content_length() {
        // A response announcing u64::MAX on top of a non-empty local prefix
        // must saturate, not overflow (debug panic) or wrap (release)
        assert_eq!(progress_total(Some(u64::MAX), 10, None), u64::MAX);
        // The announced length extends the local prefix
        assert_eq!(progress_total(Some(90), 10, None), 100);
        // Without an announcement the metadata size is used
        assert_eq!(progress_total(None, 10, Some(100)), 100);
        // Without either, the current size stands in
        assert_eq!(progress_total(None, 10, None), 10);
    }
}
