use std::fs::{self, File};
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::Local;
use colored::*;
use reqwest::Client;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

use crate::Result;
use crate::error::IaGetError;

/// Buffer size for file operations (8KB)
const BUFFER_SIZE: usize = 8192;

/// Calculates the MD5 hash of a file silently (no separate progress bar)
fn calculate_md5(file_path: &str, running: &Arc<AtomicBool>) -> Result<String> {
    let file = File::open(file_path)?;
    let mut reader = BufReader::with_capacity(BUFFER_SIZE, file);
    let mut context = md5::Context::new();
    let mut buffer = [0; BUFFER_SIZE];

    loop {
        let bytes_read = reader.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }

        if !running.load(Ordering::SeqCst) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "Hash calculation interrupted by signal",
            )
            .into());
        }

        context.consume(&buffer[..bytes_read]);
    }

    let hash = context.finalize();
    Ok(format!("{:x}", hash))
}

/// Check if an existing file has the correct hash
/// For XML files, uses alternative validation due to frequent hash mismatches
fn check_existing_file(
    file_path: &str,
    expected_md5: Option<&str>,
    running: &Arc<AtomicBool>,
) -> Result<Option<bool>> {
    if !Path::new(file_path).exists() {
        return Ok(None);
    }

    if expected_md5.is_none() {
        return Ok(Some(true));
    }

    // Special handling for XML files
    if file_path.to_lowercase().ends_with(".xml") {
        let is_valid = verify_xml_file_alternative(file_path, running)?;
        return Ok(Some(is_valid));
    }

    let local_md5 = match calculate_md5(file_path, running) {
        Ok(hash) => hash,
        Err(e) => {
            if e.to_string().contains("interrupted by signal") {
                return Err(e);
            }
            // Silently return false for hash calculation errors to avoid creating new lines
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

/// Prepare a file for download
fn prepare_file_for_download(file_path: &str) -> Result<File> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(file_path)?;

    // Seek to the end of the file for resume capability
    file.seek(SeekFrom::End(0))?;

    Ok(file)
}

/// Download file content with progress updates on the main progress bar
async fn download_file_content_simple(
    client: &Client,
    url: &str,
    file_size: u64,
    file: &mut File,
    running: &Arc<AtomicBool>,
    progress_bar: &indicatif::ProgressBar,
    file_name: &str,
) -> Result<u64> {
    let mut headers = HeaderMap::new();
    if file_size > 0 {
        headers.insert(
            reqwest::header::RANGE,
            HeaderValue::from_str(&format!("bytes={}-", file_size))
                .map_err(|e| IaGetError::Network(format!("Invalid range header value: {}", e)))?,
        );
    }

    let mut response = if file_size > 0 {
        client
            .get(url)
            .headers(headers)
            .send()
            .await
            .map_err(|e| IaGetError::Network(format!("Download failed: {}", e)))?
    } else {
        client
            .get(url)
            .send()
            .await
            .map_err(|e| IaGetError::Network(format!("Download failed: {}", e)))?
    };

    let content_length = response.content_length().unwrap_or(0);
    let total_expected_size = if file_size > 0 {
        content_length + file_size
    } else {
        content_length
    };
    let mut total_bytes: u64 = file_size;
    let mut downloaded_bytes: u64 = 0;

    let start_time = std::time::Instant::now();

    while let Some(chunk) = response.chunk().await? {
        if !running.load(Ordering::SeqCst) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "Download interrupted during file transfer",
            )
            .into());
        }

        file.write_all(&chunk)?;
        downloaded_bytes += chunk.len() as u64;
        total_bytes += chunk.len() as u64;

        // Update progress bar with download percentage and speed
        if total_expected_size > 0 {
            let percentage = (total_bytes * 100) / total_expected_size;
            let elapsed = start_time.elapsed().as_secs_f64();
            if elapsed > 1.0 {
                // Only show speed after 1 second
                let speed_bytes_per_sec = downloaded_bytes as f64 / elapsed;
                let speed_mb_per_sec = speed_bytes_per_sec / (1024.0 * 1024.0);
                if speed_mb_per_sec >= 1.0 {
                    progress_bar.set_prefix(format!(
                        "{} ({}% @ {:.1}MB/s)",
                        file_name, percentage, speed_mb_per_sec
                    ));
                } else {
                    let speed_kb_per_sec = speed_bytes_per_sec / 1024.0;
                    progress_bar.set_prefix(format!(
                        "{} ({}% @ {:.0}KB/s)",
                        file_name, percentage, speed_kb_per_sec
                    ));
                }
            } else {
                progress_bar.set_prefix(format!("{} ({}%)", file_name, percentage));
            }
        } else {
            // If we don't know the total size, just show downloaded amount
            let mb_downloaded = downloaded_bytes as f64 / (1024.0 * 1024.0);
            if mb_downloaded >= 1.0 {
                progress_bar
                    .set_prefix(format!("{} ({:.1}MB downloaded)", file_name, mb_downloaded));
            } else {
                let kb_downloaded = downloaded_bytes as f64 / 1024.0;
                progress_bar
                    .set_prefix(format!("{} ({:.0}KB downloaded)", file_name, kb_downloaded));
            }
        }
    }

    // Ensure data is written to disk
    file.flush()?;

    Ok(total_bytes)
}

/// Verify a downloaded file's hash against an expected value (no separate progress output)
/// For XML files, performs alternative validation (size + structure) due to frequent hash mismatches
fn verify_downloaded_file_simple(
    file_path: &str,
    expected_md5: Option<&str>,
    running: &Arc<AtomicBool>,
) -> Result<bool> {
    if expected_md5.is_none() {
        return Ok(true); // No hash to check against, consider it verified
    }

    // Special handling for XML files due to frequent hash mismatches at Internet Archive
    if file_path.to_lowercase().ends_with(".xml") {
        return verify_xml_file_alternative(file_path, running);
    }

    let expected_md5_str = expected_md5.unwrap();
    let local_md5 = calculate_md5(file_path, running)?;

    Ok(local_md5 == expected_md5_str)
}

/// Alternative verification for XML files using size and structure validation
/// Returns true if the file appears to be a valid XML file
fn verify_xml_file_alternative(file_path: &str, running: &Arc<AtomicBool>) -> Result<bool> {
    use std::fs;

    // Check if we should stop due to signal
    if !running.load(std::sync::atomic::Ordering::SeqCst) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Interrupted,
            "XML verification interrupted by signal",
        )
        .into());
    }

    // Check if file exists and has reasonable size (not empty, not too small)
    let metadata = match fs::metadata(file_path) {
        Ok(meta) => meta,
        Err(_) => return Ok(false), // File doesn't exist or can't be read
    };

    let file_size = metadata.len();
    if file_size < 10 {
        return Ok(false); // File too small to be valid XML
    }

    // Read the file content for structure validation
    let content = match fs::read_to_string(file_path) {
        Ok(content) => content,
        Err(_) => return Ok(false), // Can't read file as UTF-8
    };

    // Basic XML structure validation
    let content = content.trim();

    // Check for XML declaration or root element
    let has_xml_declaration = content.starts_with("<?xml");
    let has_root_element = content.contains('<') && content.contains('>');

    // Check for basic XML structure (matching angle brackets)
    let open_brackets = content.chars().filter(|&c| c == '<').count();
    let close_brackets = content.chars().filter(|&c| c == '>').count();
    let balanced_brackets = open_brackets == close_brackets;

    // Check for common Internet Archive XML patterns
    let has_ia_patterns = content.contains("<files>")
        || content.contains("<file ")
        || content.contains("name=")
        || content.contains("size=");

    // File is considered valid if it has basic XML structure and reasonable size
    let is_valid = (has_xml_declaration || has_root_element)
        && balanced_brackets
        && has_ia_patterns
        && file_size > 50; // Minimum reasonable size for IA XML files

    Ok(is_valid)
}

/// Download multiple files with shared signal handling and single-line progress display
///
/// This function provides a clean, single-line progress display that updates in-place
/// showing current file, progress, overall progress, hash matches, fails, and remaining files.
pub async fn download_files<I>(
    client: &Client,
    files: I,
    total_files: usize,
    log_hash_errors: bool,
    running: Arc<AtomicBool>,
) -> Result<()>
where
    I: IntoIterator<Item = (String, String, Option<String>)>, // (url, filename, md5)
{
    use indicatif::{ProgressBar, ProgressStyle};

    // Collect files into a Vec to allow multiple passes
    let files_vec: Vec<(String, String, Option<String>)> = files.into_iter().collect();

    // Create a single progress bar for the entire batch
    let progress_bar = ProgressBar::new(total_files as u64);
    progress_bar.set_style(
        ProgressStyle::default_bar()
            .template("{bar:40.cyan/blue} {pos:>2}/{len:2} | {msg} | {prefix}")
            .expect("Failed to set progress bar style")
            .progress_chars("█▉▊▋▌▍▎▏ "),
    );

    // Statistics tracking - these should be mutually exclusive
    let mut downloaded_count = 0; // Successfully downloaded new files
    let mut skipped_count = 0; // Files already existed with correct hash
    let mut failed_count = 0; // Files that failed to download or verify

    #[derive(Serialize, Deserialize, Debug, Clone)]
    struct BatchLogEntry {
        url: String,
        file_path: String,
        expected_md5: Option<String>,
        error: String,
        timestamp: String,
    }

    let mut batchlog: Vec<BatchLogEntry> = Vec::new();
    let batchlog_path = "batchlog.json";
    // If file exists, load previous log (to avoid duplicate entries)
    if log_hash_errors && std::path::Path::new(batchlog_path).exists() {
        if let Ok(data) = std::fs::read_to_string(batchlog_path) {
            if let Ok(entries) = serde_json::from_str::<Vec<BatchLogEntry>>(&data) {
                batchlog = entries;
            }
        }
    }

    for (index, (url, file_path, expected_md5)) in files_vec.into_iter().enumerate() {
        // Check if we should stop due to signal
        if !running.load(Ordering::SeqCst) {
            progress_bar.abandon_with_message("Download interrupted by user".to_string());
            break;
        }

        let remaining = total_files - index;
        let file_name = std::path::Path::new(&file_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Unknown file");

        // Update progress bar with current statistics and file info
        let stats_msg = format!(
            "OK:{} ERR:{} SKIP:{} LEFT:{}",
            downloaded_count + skipped_count, // Total successful files
            failed_count,
            skipped_count,
            remaining
        );

        progress_bar.set_message(stats_msg);
        progress_bar.set_prefix(file_name.to_string());

        // Try up to 2 attempts: first with resume, second (if needed) from scratch
        let mut attempt = 0;
        let max_attempts = 2;
        let mut success = false;
        let mut delay = std::time::Duration::from_secs(60);

        while attempt < max_attempts {
            // Update progress to show we're checking existing file
            if Path::new(&file_path).exists() {
                progress_bar.set_prefix(format!("{} (checking hash)", file_name));
            }

            // Check existing file
            if let Some(is_valid) =
                check_existing_file(&file_path, expected_md5.as_deref(), &running)?
            {
                if is_valid {
                    skipped_count += 1; // Only count as skipped, not as hash match
                    success = true;
                    break;
                }
            }

            // Reset prefix for download
            progress_bar.set_prefix(file_name.to_string());

            ensure_parent_directories(&file_path)?;

            // If this is the second attempt, delete the file and start from scratch
            if attempt == 1 && Path::new(&file_path).exists() {
                tokio::time::sleep(std::time::Duration::from_secs(15)).await;
                fs::remove_file(&file_path)?;
            }

            let mut file = prepare_file_for_download(&file_path)?;
            let file_size = file.metadata()?.len();

            let download_result = download_file_content_simple(
                client,
                &url,
                file_size,
                &mut file,
                &running,
                &progress_bar,
                file_name,
            )
            .await;

            match download_result {
                Ok(_) => {
                    // Update progress to show we're verifying hash
                    progress_bar.set_prefix(format!("{} (verifying)", file_name));
                    let verified = verify_downloaded_file_simple(
                        &file_path,
                        expected_md5.as_deref(),
                        &running,
                    )?;
                    // Reset prefix
                    progress_bar.set_prefix(file_name.to_string());

                    if verified {
                        downloaded_count += 1; // Successfully downloaded and verified
                        success = true;
                        break;
                    } else {
                        // Hash verification failed, will retry if attempts remain
                        // Don't increment failed_count here as we might retry
                    }
                }
                Err(e) => {
                    // Only pause if it's a transient network error
                    let is_transient = matches!(e, IaGetError::Network(_));
                    if is_transient && attempt + 1 < max_attempts {
                        tokio::time::sleep(delay).await;
                        delay *= 2;
                    } else if !is_transient {
                        // For non-transient errors, break immediately
                        break;
                    }
                }
            }
            attempt += 1;
        }

        if !success {
            failed_count += 1;
            if log_hash_errors {
                let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
                let entry = BatchLogEntry {
                    url: url.clone(),
                    file_path: file_path.clone(),
                    expected_md5: expected_md5.clone(),
                    error: "Failed to download file with correct hash".to_string(),
                    timestamp,
                };
                // Avoid duplicate entries (by file_path)
                let already_logged: HashSet<_> = batchlog.iter().map(|e| &e.file_path).collect();
                if !already_logged.contains(&file_path) {
                    batchlog.push(entry);
                }
            }
        }

        // Update progress
        progress_bar.inc(1);

        // Update final message for this file
        let stats_msg = format!(
            "OK:{} ERR:{} SKIP:{} LEFT:{}",
            downloaded_count + skipped_count, // Total successful files
            failed_count,
            skipped_count,
            total_files - index - 1
        );
        progress_bar.set_message(stats_msg);
    }

    // Finish with summary
    progress_bar.finish_with_message(format!(
        "Complete: OK:{} ERR:{} SKIP:{}",
        downloaded_count + skipped_count, // Total successful files
        failed_count,
        skipped_count
    ));

    // Write batchlog to file if logging is enabled
    if log_hash_errors {
        match serde_json::to_string_pretty(&batchlog) {
            Ok(json) => {
                if let Err(e) = std::fs::write(batchlog_path, json) {
                    eprintln!("{} Failed to write batchlog.json: {}", "▲".yellow(), e);
                }
            }
            Err(e) => {
                eprintln!("{} Failed to serialize batch log: {}", "▲".yellow(), e);
            }
        }
        // Ensure the file exists even if there are no errors
        if batchlog.is_empty() && !std::path::Path::new(batchlog_path).exists() {
            if let Err(e) = std::fs::write(batchlog_path, "[]") {
                eprintln!(
                    "{} Failed to create empty batchlog.json: {}",
                    "▲".yellow(),
                    e
                );
            }
        }
    }
    Ok(())
}
