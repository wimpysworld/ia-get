//! Module for handling file downloads, verification, and related operations.

use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use colored::*;
use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::{Client, StatusCode};

use crate::error::IaGetError; // Import IaGetError for explicit error conversion
use crate::utils::{create_progress_bar, format_duration, format_size, format_transfer_rate};
use crate::Result; // Import utility functions

/// Buffer size for file operations (8KB)
const BUFFER_SIZE: usize = 8192;

/// File size threshold for showing hash progress bar (2MB)
const LARGE_FILE_THRESHOLD: u64 = 2 * 1024 * 1024;

/// Maximum number of retry attempts for failed downloads
const MAX_RETRIES: u32 = 3;

/// Initial delay between retries in milliseconds (doubles with each retry)
const INITIAL_RETRY_DELAY_MS: u64 = 1000;

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

/// Builds the temporary `.part` path for a target file without replacing existing extensions.
fn part_file_path(file_path: &str) -> PathBuf {
    let mut part_path = OsString::from(file_path);
    part_path.push(".part");
    PathBuf::from(part_path)
}

/// Returns the current byte length of a `.part` file, or `0` if it does not exist.
fn existing_part_size(part_path: &Path) -> Result<u64> {
    match fs::metadata(part_path) {
        Ok(metadata) => Ok(metadata.len()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(e) => Err(e.into()),
    }
}

/// Atomically promote a verified `.part` file to its final destination path.
fn promote_part_file(part_path: &Path, final_path: &Path) -> Result<()> {
    fs::rename(part_path, final_path)?;
    Ok(())
}

/// Sets up signal handling for graceful shutdown on Ctrl+C
///
/// Returns an Arc<AtomicBool> that can be checked to see if the process
/// should stop. When Ctrl+C is pressed, this will be set to false.
fn setup_signal_handler() -> Arc<AtomicBool> {
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();

    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
        println!(
            "\n{} Received Ctrl+C, finishing current operation...",
            "✘".red().bold()
        );
    })
    .expect("Error setting Ctrl+C handler");

    running
}

/// Calculates the MD5 hash of a file
fn calculate_md5(file_path: &Path, running: &Arc<AtomicBool>) -> Result<String> {
    let file = File::open(file_path)?;
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

/// Check if an existing file has the correct hash
fn check_existing_file(
    file_path: &str,
    expected_md5: Option<&str>,
    running: &Arc<AtomicBool>,
) -> Result<Option<bool>> {
    let path = Path::new(file_path);
    if !path.exists() {
        return Ok(None);
    }

    if expected_md5.is_none() {
        return Ok(Some(true));
    }

    let local_md5 = match calculate_md5(path, running) {
        Ok(hash) => hash,
        Err(e) => {
            if e.to_string().contains("interrupted by signal") {
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

    Ok(Some(local_md5 == expected_md5.unwrap()))
}

/// Ensure parent directories exist for a file
fn ensure_parent_directories(file_path: &str) -> Result<()> {
    if let Some(path) = Path::new(file_path).parent() {
        if path.file_name().is_some() && !path.exists() {
            fs::create_dir_all(path)?;
        }
    }
    Ok(())
}

/// Prepare a `.part` file for download by opening it for truncation or append.
fn prepare_file_for_download(part_path: &Path, truncate: bool) -> Result<File> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(truncate)
        .open(part_path)?;

    let seek_pos = if truncate {
        SeekFrom::Start(0)
    } else {
        SeekFrom::End(0)
    };
    file.seek(seek_pos)?;

    Ok(file)
}

/// Download file content into a `.part` file with progress reporting and automatic retry on failure
async fn download_file_content(
    client: &Client,
    url: &str,
    part_path: &Path,
    running: &Arc<AtomicBool>,
    cookie_header: Option<&str>,
    expected_md5: Option<&str>,
) -> Result<(u64, bool)> {
    let mut retry_count = 0;
    let mut has_reset_range = false;

    loop {
        // Re-check `.part` file size at start of each attempt (in case of retry)
        let current_file_size = existing_part_size(part_path)?;

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

        let mut response = match request.send().await {
            Ok(resp) => resp,
            Err(e) => {
                // Request failed before we even got a response
                retry_count += 1;

                if retry_count > MAX_RETRIES {
                    println!(
                        "{} {}      {} Maximum retries ({}) exceeded",
                        "├╼".cyan().dimmed(),
                        "Failed".red().bold(),
                        "✘".red().bold(),
                        MAX_RETRIES
                    );
                    return Err(e.into());
                }

                let delay = INITIAL_RETRY_DELAY_MS * 2u64.pow(retry_count - 1);
                println!(
                    "{} {}      {} Connection error (attempt {}/{}): {}",
                    "├╼".cyan().dimmed(),
                    "Retry".yellow().bold(),
                    "⟳".yellow().bold(),
                    retry_count,
                    MAX_RETRIES,
                    e
                );
                println!(
                    "{} {}      Waiting {:.1}s before retry...",
                    "├╼".cyan().dimmed(),
                    "Wait".white(),
                    delay as f64 / 1000.0
                );

                tokio::time::sleep(tokio::time::Duration::from_millis(delay)).await;

                continue; // Retry from the top of the loop
            }
        };

        let status = response.status();
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
                prepare_file_for_download(part_path, true)?,
                0,
                total_expected_size,
            ),
            ResponseAction::AppendFromOffset {
                total_expected_size,
            } => (
                prepare_file_for_download(part_path, false)?,
                current_file_size,
                total_expected_size,
            ),
            ResponseAction::VerifyCompletePart => {
                if expected_md5.is_some() {
                    if !running.load(Ordering::SeqCst) {
                        return Ok((current_file_size, false));
                    }
                    let is_valid = verify_downloaded_file(part_path, expected_md5, running)?;
                    if !running.load(Ordering::SeqCst) {
                        return Ok((current_file_size, false));
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
                    match fs::remove_file(part_path) {
                        Ok(()) => {}
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                        Err(e) => return Err(e.into()),
                    }
                    continue;
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
                match fs::remove_file(part_path) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.into()),
                }
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
            while let Some(chunk_result) = response.chunk().await.transpose() {
                if !running.load(Ordering::SeqCst) {
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

                // Check if this is a user interruption
                if !running.load(Ordering::SeqCst) || e.to_string().contains("interrupted") {
                    file.flush()?;
                    drop(file);
                    return Err(e);
                }

                retry_count += 1;

                if retry_count > MAX_RETRIES {
                    let _ = file.flush();
                    drop(file);
                    println!(
                        "{} {}      {} Maximum retries ({}) exceeded",
                        "├╼".cyan().dimmed(),
                        "Failed".red().bold(),
                        "✘".red().bold(),
                        MAX_RETRIES
                    );
                    return Err(e);
                }

                let delay = INITIAL_RETRY_DELAY_MS * 2u64.pow(retry_count - 1);
                println!(
                    "{} {}      {} Download error (attempt {}/{}): {}",
                    "├╼".cyan().dimmed(),
                    "Retry".yellow().bold(),
                    "⟳".yellow().bold(),
                    retry_count,
                    MAX_RETRIES,
                    e
                );
                println!(
                    "{} {}      Waiting {:.1}s before retry...",
                    "├╼".cyan().dimmed(),
                    "Wait".white(),
                    delay as f64 / 1000.0
                );

                // Ensure file is flushed and closed before sleeping and starting next attempt
                file.flush()?;
                drop(file);

                tokio::time::sleep(tokio::time::Duration::from_millis(delay)).await;
            }
        }
    }
}

/// Verify a downloaded file's hash against an expected value
fn verify_downloaded_file(
    file_path: &Path,
    expected_md5: Option<&str>,
    running: &Arc<AtomicBool>,
) -> Result<bool> {
    if expected_md5.is_none() {
        println!(
            "{} {}",
            "-".dimmed(),
            "No MD5 hash provided for verification.".dimmed()
        );
        return Ok(true); // No hash to check against, consider it verified
    }
    let expected_md5_str = expected_md5.unwrap();
    let local_md5 = calculate_md5(file_path, running)?;
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

/// Download a single file via `.part` staging, MD5 verification, and atomic promotion.
async fn download_single_file(
    client: &Client,
    url: &str,
    file_path: &str,
    expected_md5: Option<&str>,
    running: &Arc<AtomicBool>,
    cookie_header: Option<&str>,
) -> Result<()> {
    if let Some(is_valid) = check_existing_file(file_path, expected_md5, running)? {
        if is_valid {
            println!(
                "{} {}   {}",
                "╰╼".cyan().dimmed(),
                "Downloaded".white(),
                "✔".green().bold()
            );
            return Ok(());
        } else {
            println!(
                "{} {}      {}",
                "├╼".cyan().dimmed(),
                "Partial".white(),
                "▲".yellow().bold()
            );
        }
    }

    ensure_parent_directories(file_path)?;

    let part_path = part_file_path(file_path);
    let (_, already_verified) = download_file_content(
        client,
        url,
        &part_path,
        running,
        cookie_header,
        expected_md5,
    )
    .await?;

    if !running.load(Ordering::SeqCst) {
        return Ok(());
    }

    if already_verified || verify_downloaded_file(&part_path, expected_md5, running)? {
        promote_part_file(&part_path, Path::new(file_path))?;
        Ok(())
    } else {
        Err(IaGetError::Integrity(format!(
            "MD5 mismatch for {}",
            file_path
        )))
    }
}

/// Download multiple files with shared signal handling
///
/// This function sets up signal handling once for the entire download session
/// and allows for graceful interruption between files.
pub async fn download_files<I>(
    client: &Client,
    files: I,
    total_files: usize,
    cookie_header: Option<&str>,
) -> Result<()>
where
    I: IntoIterator<Item = (String, String, Option<String>)>, // (url, filename, md5)
{
    // Set up signal handling for the entire download session
    let running = setup_signal_handler();

    for (index, (url, file_path, expected_md5)) in files.into_iter().enumerate() {
        // Check if we should stop due to signal
        if !running.load(Ordering::SeqCst) {
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

        download_single_file(
            client,
            &url,
            &file_path,
            expected_md5.as_deref(),
            &running,
            cookie_header,
        )
        .await?;
    }

    Ok(())
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
    use std::time::Duration;
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

        fn file_path(&self, name: &str) -> String {
            self.path.join(name).to_string_lossy().into_owned()
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
            running: Arc<AtomicBool>,
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

        async fn finish(mut self) -> Vec<CapturedRequest> {
            if let Some(handle) = self.handle.take() {
                let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;
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
                running,
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

                running.store(false, Ordering::SeqCst);

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
        let file_path = dir.file_path("nested/item.bin");
        let part_path = part_file_path(&file_path);
        let client = Client::new();
        let running = Arc::new(AtomicBool::new(true));

        download_single_file(
            &client,
            &server.url("/item.bin"),
            &file_path,
            Some(&expected_md5),
            &running,
            None,
        )
        .await
        .expect("fresh 200 download should succeed");

        let requests = server.finish().await;
        assert_eq!(requests.len(), 1);
        assert!(requests[0].request_line.starts_with("GET /item.bin "));
        assert_eq!(requests[0].header("range"), None);
        assert_eq!(fs::read(&file_path).unwrap(), payload);
        assert!(!part_path.exists(), ".part file should be promoted away");
    }

    #[tokio::test]
    async fn test_download_interrupted_mid_stream_leaves_part_without_final_file() {
        let first_chunk = b"0123456789";
        let second_chunk = b"abcdefghij";
        let total_len = first_chunk.len() + second_chunk.len();
        let dir = TempTestDir::new("interrupt");
        let file_path = dir.file_path("interrupted.bin");
        let part_path = part_file_path(&file_path);
        let running = Arc::new(AtomicBool::new(true));

        let server = MockServer::start(vec![MockResponse::MidStreamInterrupt {
            status_line: "200 OK",
            headers: vec![("Content-Length".to_string(), total_len.to_string())],
            first_chunk: first_chunk.to_vec(),
            second_chunk: Some(second_chunk.to_vec()),
            watch_path: part_path.clone(),
            expected_part_len: first_chunk.len() as u64,
            running: Arc::clone(&running),
        }])
        .await;

        let client = Client::new();
        let err = download_single_file(
            &client,
            &server.url("/interrupted.bin"),
            &file_path,
            None,
            &running,
            None,
        )
        .await
        .unwrap_err();

        let requests = server.finish().await;
        assert_eq!(requests.len(), 1);
        assert!(matches!(err, IaGetError::FileSystem(_)));
        assert!(
            !Path::new(&file_path).exists(),
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
        let file_path = dir.file_path("resumed.bin");
        let part_path = part_file_path(&file_path);
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
        let running = Arc::new(AtomicBool::new(true));
        download_single_file(
            &client,
            &server.url("/resumed.bin"),
            &file_path,
            Some(&expected_md5),
            &running,
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
        assert_eq!(fs::read(&file_path).unwrap(), full);
        assert!(!part_path.exists());
    }

    #[tokio::test]
    async fn test_download_http_200_to_ranged_request_restarts_without_duplication() {
        let stale_prefix = b"stale-prefix-bytes";
        let full_payload = b"complete fresh payload from byte zero";
        let expected_md5 = md5_hex(full_payload);

        let dir = TempTestDir::new("ranged-200");
        let file_path = dir.file_path("restarted.bin");
        let part_path = part_file_path(&file_path);
        fs::write(&part_path, stale_prefix).unwrap();

        let server = MockServer::start(vec![MockResponse::ok(full_payload)]).await;
        let client = Client::new();
        let running = Arc::new(AtomicBool::new(true));

        download_single_file(
            &client,
            &server.url("/restarted.bin"),
            &file_path,
            Some(&expected_md5),
            &running,
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
        assert_eq!(fs::read(&file_path).unwrap(), full_payload);
        assert!(!part_path.exists());
    }

    #[tokio::test]
    async fn test_download_http_206_mismatched_content_range_discards_and_refetches_unranged() {
        let stale_prefix = b"stale-01";
        let full_payload = b"0123456789abcdefghij";
        let expected_md5 = md5_hex(full_payload);

        let dir = TempTestDir::new("mismatched-206");
        let file_path = dir.file_path("mismatched.bin");
        let part_path = part_file_path(&file_path);
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
        let running = Arc::new(AtomicBool::new(true));
        download_single_file(
            &client,
            &server.url("/mismatched.bin"),
            &file_path,
            Some(&expected_md5),
            &running,
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
        assert_eq!(fs::read(&file_path).unwrap(), full_payload);
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
            let file_path = dir.file_path("bad206.bin");
            let part_path = part_file_path(&file_path);
            fs::write(&part_path, stale_prefix).unwrap();

            let server = MockServer::start(vec![
                MockResponse::with_headers("206 Partial Content", bad_headers, b"junk"),
                MockResponse::ok(full_payload),
            ])
            .await;

            let client = Client::new();
            let running = Arc::new(AtomicBool::new(true));
            download_single_file(
                &client,
                &server.url("/bad206.bin"),
                &file_path,
                Some(&expected_md5),
                &running,
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
            assert_eq!(fs::read(&file_path).unwrap(), full_payload, "case {label}");
            assert!(!part_path.exists(), "case {label}");
        }
    }

    #[tokio::test]
    async fn test_download_http_206_repeated_on_unranged_retry_fails_with_network_error() {
        let stale_prefix = b"partial";
        let dir = TempTestDir::new("repeated-206");
        let file_path = dir.file_path("repeated206.bin");
        let part_path = part_file_path(&file_path);
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
        let running = Arc::new(AtomicBool::new(true));
        let err = download_single_file(
            &client,
            &server.url("/repeated206.bin"),
            &file_path,
            None,
            &running,
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
        assert!(!Path::new(&file_path).exists());
        assert!(!part_path.exists());
    }

    #[tokio::test]
    async fn test_download_http_416_matching_complete_part_verifies_in_place() {
        let full_payload = b"already complete part payload";
        let expected_md5 = md5_hex(full_payload);

        let dir = TempTestDir::new("416-complete");
        let file_path = dir.file_path("complete.bin");
        let part_path = part_file_path(&file_path);
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
        let running = Arc::new(AtomicBool::new(true));
        download_single_file(
            &client,
            &server.url("/complete.bin"),
            &file_path,
            Some(&expected_md5),
            &running,
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
        assert_eq!(fs::read(&file_path).unwrap(), full_payload);
        assert!(!part_path.exists());
    }

    #[tokio::test]
    async fn test_download_http_416_oversized_part_discards_and_refetches_unranged() {
        let full_payload = b"exact server bytes";
        let expected_md5 = md5_hex(full_payload);
        let oversized_part = b"exact server bytes PLUS extra stale trailing data";

        let dir = TempTestDir::new("416-oversized");
        let file_path = dir.file_path("oversized.bin");
        let part_path = part_file_path(&file_path);
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
        let running = Arc::new(AtomicBool::new(true));
        download_single_file(
            &client,
            &server.url("/oversized.bin"),
            &file_path,
            Some(&expected_md5),
            &running,
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
        assert_eq!(fs::read(&file_path).unwrap(), full_payload);
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
        let file_path = dir.file_path("corrupt-complete.bin");
        let part_path = part_file_path(&file_path);
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
        let running = Arc::new(AtomicBool::new(true));
        download_single_file(
            &client,
            &server.url("/corrupt-complete.bin"),
            &file_path,
            Some(&expected_md5),
            &running,
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
        assert_eq!(fs::read(&file_path).unwrap(), full_payload);
        assert!(!part_path.exists());
    }

    #[tokio::test]
    async fn test_download_error_responses_never_write_body_to_part_or_final_file() {
        let error_cases: [(&str, &str, &[u8]); 3] = [
            (
                "404",
                "404 Not Found",
                b"<html><body><h1>404 Not Found</h1></body></html>",
            ),
            (
                "500",
                "500 Internal Server Error",
                b"<html><body>Internal Server Error</body></html>",
            ),
            ("403", "403 Forbidden", b"Access Denied"),
        ];

        let client = Client::new();
        let running = Arc::new(AtomicBool::new(true));

        for (label, status_line, body) in error_cases {
            // Case 1: no prior .part file
            let dir_fresh = TempTestDir::new(&format!("err-fresh-{label}"));
            let file_path_fresh = dir_fresh.file_path("target.bin");
            let part_path_fresh = part_file_path(&file_path_fresh);

            let server_fresh = MockServer::start(vec![MockResponse::with_headers(
                status_line,
                vec![
                    ("Content-Type", "text/html".to_string()),
                    ("Content-Length", body.len().to_string()),
                ],
                body,
            )])
            .await;

            let err = download_single_file(
                &client,
                &server_fresh.url("/target.bin"),
                &file_path_fresh,
                None,
                &running,
                None,
            )
            .await
            .unwrap_err();

            let requests = server_fresh.finish().await;
            assert_eq!(requests.len(), 1, "fresh {label}");
            assert!(matches!(err, IaGetError::Network(_)), "fresh {label}");
            assert!(
                !part_path_fresh.exists(),
                "fresh {label} must not create .part"
            );
            assert!(
                !Path::new(&file_path_fresh).exists(),
                "fresh {label} must not create final file"
            );

            // Case 2: pre-existing .part prefix must stay untouched
            let existing_prefix = b"preserved-partial-prefix";
            let dir_resume = TempTestDir::new(&format!("err-resume-{label}"));
            let file_path_resume = dir_resume.file_path("target.bin");
            let part_path_resume = part_file_path(&file_path_resume);
            fs::write(&part_path_resume, existing_prefix).unwrap();

            let server_resume = MockServer::start(vec![MockResponse::with_headers(
                status_line,
                vec![
                    ("Content-Type", "text/html".to_string()),
                    ("Content-Length", body.len().to_string()),
                ],
                body,
            )])
            .await;

            let err = download_single_file(
                &client,
                &server_resume.url("/target.bin"),
                &file_path_resume,
                None,
                &running,
                None,
            )
            .await
            .unwrap_err();

            let requests = server_resume.finish().await;
            assert_eq!(requests.len(), 1, "resume {label}");
            assert_eq!(
                requests[0].header("range"),
                Some(format!("bytes={}-", existing_prefix.len()).as_str()),
                "resume {label}"
            );
            assert!(matches!(err, IaGetError::Network(_)), "resume {label}");
            assert_eq!(
                fs::read(&part_path_resume).unwrap(),
                existing_prefix,
                "resume {label} must not append or overwrite .part with error body"
            );
            assert!(
                !Path::new(&file_path_resume).exists(),
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
        let file_path = dir.file_path("corrupt.bin");
        let part_path = part_file_path(&file_path);
        let client = Client::new();
        let running = Arc::new(AtomicBool::new(true));

        let err = download_single_file(
            &client,
            &server.url("/corrupt.bin"),
            &file_path,
            Some(wrong_md5),
            &running,
            None,
        )
        .await
        .unwrap_err();

        let requests = server.finish().await;
        assert_eq!(requests.len(), 1);
        assert!(matches!(err, IaGetError::Integrity(_)));
        assert!(
            !Path::new(&file_path).exists(),
            "MD5 mismatch must never promote .part to final path"
        );
        assert_eq!(fs::read(&part_path).unwrap(), payload);
    }

    #[tokio::test]
    async fn test_download_http_206_huge_content_length_does_not_overflow_or_panic() {
        let prefix = b"hello";
        let chunk = b"world";
        let dir = TempTestDir::new("u64-max-cl");
        let file_path = dir.file_path("huge-cl.bin");
        let part_path = part_file_path(&file_path);
        fs::write(&part_path, prefix).unwrap();

        // hyper reserves u64::MAX and u64::MAX - 1 as DecodedLength sentinels, so
        // u64::MAX - 2 is the largest Content-Length it passes to reqwest; with a
        // 5-byte prefix it still overflows current_offset + content_length without
        // saturating arithmetic. (u64::MAX itself is also covered by unit tests.)
        let huge_content_length = u64::MAX - 2;
        assert!(prefix.len() as u64 > u64::MAX - huge_content_length);

        let running = Arc::new(AtomicBool::new(true));
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
            running: Arc::clone(&running),
        }])
        .await;

        let client = Client::new();
        let result = download_single_file(
            &client,
            &server.url("/huge-cl.bin"),
            &file_path,
            None,
            &running,
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
        assert!(!Path::new(&file_path).exists());
        assert_eq!(fs::read(&part_path).unwrap(), b"helloworld");
    }
}
