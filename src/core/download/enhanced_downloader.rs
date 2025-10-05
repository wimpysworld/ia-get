//! Enhanced Internet Archive downloader
//!
//! This module provides the main download engine for ia-get with comprehensive
//! session management, progress tracking, and resume capabilities.
//!
//! ## Features
//!
//! - **Session Management**: Persistent download sessions for resume capability
//! - **Concurrent Downloads**: Configurable parallel downloading with rate limiting
//! - **Progress Tracking**: Real-time progress bars and statistics
//! - **Error Recovery**: Automatic retry logic for transient failures
//! - **Filtering**: Advanced file filtering by format, size, and patterns
//! - **Compression**: Automatic decompression of downloaded archives
//! - **Verification**: MD5 hash verification for data integrity
//!
//! ## Usage
//!
//! ```rust,no_run
//! use ia_get::{
//!     enhanced_downloader::ArchiveDownloader,
//!     metadata_storage::DownloadConfig,
//! };
//! use reqwest::Client;
//! use std::path::PathBuf;
//!
//! // Create downloader with 4 concurrent connections
//! let client = Client::new();
//! let downloader = ArchiveDownloader::new(
//!     client, 4, true, true, PathBuf::from(".sessions"), false, false
//! );
//! ```

use crate::{
    IaGetError, Result,
    core::session::{
        ArchiveFile, ArchiveMetadata, DownloadConfig, DownloadSession, DownloadState,
        FileDownloadStatus,
    },
};
use colored::*;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use reqwest::Client;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio::sync::Semaphore;

/// Download context to avoid too many function arguments
struct DownloadContext<'a> {
    client: &'a Client,
    url: &'a str,
    temp_path: &'a Path,
    output_path: &'a Path,
    file_info: &'a ArchiveFile,
    progress_bar: &'a ProgressBar,
    resume_from: u64,
}

/// Enhanced downloader that uses full Archive.org metadata
pub struct ArchiveDownloader {
    client: Client,
    max_concurrent: usize,
    verify_md5: bool,
    preserve_mtime: bool,
    session_dir: PathBuf,
    enable_compression: bool,
    auto_decompress: bool,
}

impl ArchiveDownloader {
    /// Create a new downloader instance
    pub fn new(
        client: Client,
        max_concurrent: usize,
        verify_md5: bool,
        preserve_mtime: bool,
        session_dir: PathBuf,
        enable_compression: bool,
        auto_decompress: bool,
    ) -> Self {
        Self {
            client,
            max_concurrent,
            verify_md5,
            preserve_mtime,
            session_dir,
            enable_compression,
            auto_decompress,
        }
    }

    /// Download files using comprehensive metadata and session management
    pub async fn download_with_metadata(
        &self,
        original_url: String,
        identifier: String,
        archive_metadata: ArchiveMetadata,
        download_config: DownloadConfig,
        requested_files: Vec<String>,
        progress_bar: &ProgressBar,
    ) -> Result<DownloadSession> {
        // Create or resume download session
        let mut session = self
            .create_or_resume_session(
                original_url,
                identifier.clone(),
                archive_metadata,
                download_config,
                requested_files,
            )
            .await?;

        // Create session directory if it doesn't exist
        tokio::fs::create_dir_all(&self.session_dir)
            .await
            .map_err(|e| {
                IaGetError::FileSystem(format!("Failed to create session directory: {}", e))
            })?;

        // Save initial session state
        let session_file = self
            .session_dir
            .join(crate::core::session::generate_session_filename(&identifier));
        session.save_to_file(&session_file)?;

        progress_bar.set_message("Initializing downloads...".to_string());

        // Get pending files to download
        let pending_files: Vec<String> = session
            .get_pending_files()
            .into_iter()
            .map(|s| s.to_string())
            .collect();

        if pending_files.is_empty() {
            progress_bar.finish_with_message("All files already downloaded".green().to_string());
            return Ok(session);
        }

        // Setup progress tracking
        let multi_progress = MultiProgress::new();
        let main_progress = multi_progress.add(ProgressBar::new(pending_files.len() as u64));
        main_progress.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos:>3}/{len:>3} files {msg}")
                .unwrap()
                .progress_chars("█▉▊▋▌▍▎▏ ")
        );
        main_progress.set_message("(Completed: 0, Failed: 0)".to_string());

        // Create semaphore for concurrency control
        let semaphore = Arc::new(Semaphore::new(self.max_concurrent));
        let mut handles = Vec::new();

        // Start downloads for pending files
        for file_name in pending_files {
            if let Some(file_status) = session.file_status.get(&file_name) {
                let file_info = file_status.file_info.clone();
                let servers = session.archive_metadata.workable_servers.clone();
                let dir = session.archive_metadata.dir.clone();
                let output_path = PathBuf::from(&file_status.local_path);

                let client = self.client.clone();
                let semaphore_clone = semaphore.clone();
                let verify_md5 = self.verify_md5;
                let preserve_mtime = self.preserve_mtime;
                let _enable_compression = self.enable_compression; // Compression now always enabled per IA docs
                let auto_decompress = self.auto_decompress;
                let decompress_formats = session.download_config.decompress_formats.clone();

                // Create individual progress bar for this file
                let file_progress =
                    multi_progress.add(ProgressBar::new(file_info.size.unwrap_or(0)));
                file_progress.set_style(
                    ProgressStyle::default_bar()
                        .template("{spinner:.green} {msg:30.30} [{bar:25.cyan/blue}] {bytes:>8}/{total_bytes:>8} {eta:>8}")
                        .unwrap()
                        .progress_chars("█▉▊▋▌▍▎▏ ")
                );
                file_progress.set_message(file_info.name.chars().take(30).collect::<String>());

                let handle = tokio::spawn(async move {
                    let _permit = semaphore_clone.acquire().await.unwrap();

                    Self::download_single_file(
                        client,
                        file_info,
                        servers,
                        dir,
                        output_path,
                        verify_md5,
                        preserve_mtime,
                        auto_decompress,
                        decompress_formats,
                        file_progress,
                    )
                    .await
                });

                handles.push((file_name, handle));
            }
        }

        // Wait for all downloads to complete and update session
        let mut completed = 0;
        let mut failed = 0;
        let total_files = handles.len();

        // Update progress message less frequently to reduce screen spam
        let mut last_update = std::time::Instant::now();
        const UPDATE_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);

        for (file_name, handle) in handles {
            match handle.await {
                Ok(Ok(_)) => {
                    session.update_file_status(&file_name, DownloadState::Completed);
                    completed += 1;
                    main_progress.inc(1);

                    // Only update message if enough time has passed to reduce spam
                    let now = std::time::Instant::now();
                    if now.duration_since(last_update) >= UPDATE_INTERVAL
                        || completed + failed == total_files
                    {
                        main_progress
                            .set_message(format!("(Completed: {}, Failed: {})", completed, failed));
                        last_update = now;
                    }
                }
                Ok(Err(e)) => {
                    session.update_file_status(&file_name, DownloadState::Failed);
                    if let Some(file_status) = session.file_status.get_mut(&file_name) {
                        file_status.error_message = Some(e.to_string());
                    }
                    failed += 1;
                    main_progress.inc(1);

                    // Update message for failures and at intervals
                    let now = std::time::Instant::now();
                    if now.duration_since(last_update) >= UPDATE_INTERVAL
                        || completed + failed == total_files
                    {
                        main_progress
                            .set_message(format!("(Completed: {}, Failed: {})", completed, failed));
                        last_update = now;
                    }

                    // Use eprintln for errors instead of progress bar messages to avoid spam
                    eprintln!("{} Failed to download {}: {}", "✘".red(), file_name, e);
                }
                Err(e) => {
                    session.update_file_status(&file_name, DownloadState::Failed);
                    if let Some(file_status) = session.file_status.get_mut(&file_name) {
                        file_status.error_message = Some(format!("Task join error: {}", e));
                    }
                    failed += 1;
                    main_progress.inc(1);

                    // Update message for failures and at intervals
                    let now = std::time::Instant::now();
                    if now.duration_since(last_update) >= UPDATE_INTERVAL
                        || completed + failed == total_files
                    {
                        main_progress
                            .set_message(format!("(Completed: {}, Failed: {})", completed, failed));
                        last_update = now;
                    }

                    eprintln!("{} Task failed for {}: {}", "✘".red(), file_name, e);
                }
            }
        }

        // Save final session state
        session.save_to_file(&session_file)?;

        if failed == 0 {
            main_progress.finish_with_message(
                format!("✓ Successfully downloaded {} files", completed)
                    .green()
                    .to_string(),
            );
        } else {
            main_progress.finish_with_message(
                format!("⚠ Completed {} files, {} failed", completed, failed)
                    .yellow()
                    .to_string(),
            );
        }

        Ok(session)
    }

    /// Download a single file with retry logic and proper Archive.org server usage
    #[allow(clippy::too_many_arguments)]
    async fn download_single_file(
        client: Client,
        file_info: ArchiveFile,
        servers: Vec<String>,
        dir: String,
        output_path: PathBuf,
        verify_md5: bool,
        preserve_mtime: bool,
        auto_decompress: bool,
        decompress_formats: Vec<String>,
        progress_bar: ProgressBar,
    ) -> Result<()> {
        // Create output directory if it doesn't exist
        if let Some(parent) = output_path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                IaGetError::FileSystem(format!("Failed to create output directory: {}", e))
            })?;
        }

        // Check if file already exists and is valid
        if output_path.exists() {
            if verify_md5 && file_info.md5.is_some() {
                progress_bar.set_message(format!("Verifying existing {}", file_info.name));

                // Convert async path to sync for MD5 calculation
                let path_str = output_path.to_string_lossy().to_string();
                let file_info_clone = file_info.clone();
                let validation_result =
                    tokio::task::spawn_blocking(move || file_info_clone.validate_md5(&path_str))
                        .await
                        .map_err(|e| {
                            IaGetError::Network(format!("MD5 validation task failed: {}", e))
                        })??;

                if validation_result {
                    progress_bar.finish_with_message(
                        format!("✓ {} already exists and is valid", file_info.name)
                            .green()
                            .to_string(),
                    );
                    return Ok(());
                } else {
                    progress_bar
                        .set_message(format!("MD5 mismatch, re-downloading {}", file_info.name));
                }
            } else {
                progress_bar.finish_with_message(
                    format!(
                        "✓ {} already exists (skipping verification)",
                        file_info.name
                    )
                    .yellow()
                    .to_string(),
                );
                return Ok(());
            }
        }

        let mut last_error = None;
        let max_retries = 3;

        // Try each server in order (following Archive.org recommendations)
        for (retry_count, server) in servers.iter().enumerate() {
            if retry_count >= max_retries {
                break;
            }

            let download_url = file_info.get_download_url(server, &dir);
            progress_bar.set_message(format!("Downloading {} from {}", file_info.name, server));

            match Self::download_from_url(
                &client,
                &download_url,
                &output_path,
                &file_info,
                &progress_bar,
            )
            .await
            {
                Ok(_) => {
                    // Verify MD5 if required and available
                    if verify_md5 && file_info.md5.is_some() {
                        progress_bar.set_message(format!("Verifying {}", file_info.name));

                        let path_str = output_path.to_string_lossy().to_string();
                        let file_info_clone = file_info.clone();
                        let validation_result = tokio::task::spawn_blocking(move || {
                            file_info_clone.validate_md5(&path_str)
                        })
                        .await
                        .map_err(|e| {
                            IaGetError::Network(format!("MD5 validation task failed: {}", e))
                        })??;

                        if !validation_result {
                            let error_msg =
                                format!("MD5 verification failed for {}", file_info.name);
                            progress_bar
                                .finish_with_message(format!("✘ {}", error_msg).red().to_string());

                            // Remove invalid file
                            let _ = tokio::fs::remove_file(&output_path).await;
                            last_error = Some(IaGetError::HashMismatch(error_msg));
                            continue;
                        }
                    }

                    // Set modification time if required
                    if preserve_mtime {
                        let path_str = output_path.to_string_lossy().to_string();
                        let file_info_clone = file_info.clone();
                        let _ = tokio::task::spawn_blocking(move || {
                            file_info_clone.set_file_mtime(&path_str)
                        })
                        .await;
                    }

                    // Handle automatic decompression if enabled
                    if auto_decompress && file_info.is_compressed() {
                        if let Some(_compression_format) = file_info.get_compression_format() {
                            if let Some(format) =
                                crate::utilities::compression::CompressionFormat::from_filename(
                                    &file_info.name,
                                )
                            {
                                if crate::utilities::compression::should_decompress(
                                    &format,
                                    &decompress_formats,
                                ) {
                                    progress_bar
                                        .set_message(format!("Decompressing {}", file_info.name));

                                    // Determine output path for decompressed file(s)
                                    let decompressed_name = file_info.get_decompressed_name();
                                    let decompressed_path =
                                        if let Some(parent) = output_path.parent() {
                                            parent.join(&decompressed_name)
                                        } else {
                                            PathBuf::from(&decompressed_name)
                                        };

                                    // Perform decompression
                                    let output_path_clone = output_path.clone();
                                    let decompressed_path_clone = decompressed_path.clone();
                                    let progress_clone = progress_bar.clone();

                                    let decompress_result =
                                        tokio::task::spawn_blocking(move || {
                                            crate::utilities::compression::decompress_file(
                                                &output_path_clone,
                                                &decompressed_path_clone,
                                                format,
                                                Some(&progress_clone),
                                            )
                                        })
                                        .await;

                                    match decompress_result {
                                        Ok(Ok(())) => {
                                            progress_bar.set_message(format!(
                                                "Decompressed {} → {}",
                                                file_info.name, decompressed_name
                                            ));

                                            // Optionally remove the compressed file after successful decompression
                                            // For now, we'll keep both to be safe
                                        }
                                        Ok(Err(e)) => {
                                            progress_bar.set_message(format!(
                                                "Decompression failed for {}: {}",
                                                file_info.name, e
                                            ));
                                            // Continue without failing the download
                                        }
                                        Err(e) => {
                                            progress_bar.set_message(format!(
                                                "Decompression task failed for {}: {}",
                                                file_info.name, e
                                            ));
                                            // Continue without failing the download
                                        }
                                    }
                                }
                            }
                        }
                    }

                    progress_bar.finish_with_message(
                        format!("✓ Downloaded {}", file_info.name)
                            .green()
                            .to_string(),
                    );
                    return Ok(());
                }
                Err(e) => {
                    let error_str = e.to_string();
                    let should_retry_server = error_str.contains("503")
                        || error_str.contains("Service Unavailable")
                        || error_str.contains("connection")
                        || error_str.contains("timeout");

                    let should_backoff_rate_limit =
                        error_str.contains("429") || error_str.contains("Rate limited");

                    if should_backoff_rate_limit {
                        // For rate limiting, wait longer before trying next server
                        progress_bar.set_message(format!(
                            "Rate limited by IA server {}, backing off before trying next server...", 
                            server
                        ));
                        let backoff_delay = std::cmp::min(60, 2_u64.pow(retry_count as u32)); // Max 60 seconds for rate limits
                        tokio::time::sleep(std::time::Duration::from_secs(backoff_delay)).await;
                    } else if should_retry_server {
                        progress_bar.set_message(format!(
                            "Server {} unavailable (503/timeout), trying next server...",
                            server
                        ));
                        // Standard exponential backoff for server errors
                        if retry_count < servers.len() - 1 {
                            let backoff_delay = std::cmp::min(2_u64.pow(retry_count as u32), 30); // Max 30 seconds
                            tokio::time::sleep(std::time::Duration::from_secs(backoff_delay)).await;
                        }
                    } else {
                        progress_bar
                            .set_message(format!("Failed from {}, trying next server...", server));
                        // Quick retry for other errors
                        if retry_count < servers.len() - 1 {
                            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                        }
                    }

                    last_error = Some(e);
                }
            }
        }

        // If we get here, all servers failed
        let error_msg = format!(
            "Failed to download {} from all {} servers. Last error: {}",
            file_info.name,
            servers.len(),
            last_error
                .as_ref()
                .map(|e| e.to_string())
                .unwrap_or_else(|| "Unknown error".to_string())
        );

        progress_bar.finish_with_message(format!("✘ {}", error_msg).red().to_string());
        Err(last_error.unwrap_or(IaGetError::Network(error_msg)))
    }

    /// Download from a specific URL with progress tracking
    async fn download_from_url(
        client: &Client,
        url: &str,
        output_path: &Path,
        file_info: &ArchiveFile,
        progress_bar: &ProgressBar,
    ) -> Result<()> {
        let temp_path = output_path.with_extension("tmp");

        // Try download with resume capability
        const MAX_RESUME_ATTEMPTS: u32 = 3;

        for attempt in 0..MAX_RESUME_ATTEMPTS {
            let resume_from = if temp_path.exists() {
                match tokio::fs::metadata(&temp_path).await {
                    Ok(metadata) => metadata.len(),
                    Err(_) => 0,
                }
            } else {
                0
            };

            let ctx = DownloadContext {
                client,
                url,
                temp_path: &temp_path,
                output_path,
                file_info,
                progress_bar,
                resume_from,
            };

            match Self::perform_download(ctx).await {
                Ok(()) => return Ok(()),
                Err(e) => {
                    // For decoding errors, don't retry with compression disabled since we already do that
                    if attempt == MAX_RESUME_ATTEMPTS - 1 {
                        return Err(e);
                    }

                    // Short delay before retry
                    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
                }
            }
        }

        Err(IaGetError::Network(format!(
            "Failed to download {} after {} resume attempts",
            file_info.name, MAX_RESUME_ATTEMPTS
        )))
    }

    /// Perform a single download attempt with optional resume
    async fn perform_download(ctx: DownloadContext<'_>) -> Result<()> {
        let mut request = ctx.client.get(ctx.url);

        // Apply Internet Archive recommended compression headers
        // Based on IA API docs: "Clients which support compression should include
        // an Accept-Encoding: deflate, gzip header in requests"
        request = request.header("Accept-Encoding", "deflate, gzip");

        // Add IA-specific header to avoid rate limiting on high-volume downloads
        // IA API docs: "When set to a true-ish value (e.g., 1), a client submitting
        // a task for execution can avoid rate limiting"
        request = request.header("X-Accept-Reduced-Priority", "1");

        // Add range header for resume - IA supports partial content requests
        if ctx.resume_from > 0 {
            request = request.header("Range", format!("bytes={}-", ctx.resume_from));
        }

        let response = request
            .send()
            .await
            .map_err(|e| IaGetError::Network(format!("Failed to start download: {}", e)))?;

        // Handle Internet Archive specific HTTP status codes
        let status = response.status();
        if !status.is_success() && status != reqwest::StatusCode::PARTIAL_CONTENT {
            return match status {
                reqwest::StatusCode::TOO_MANY_REQUESTS => {
                    // IA API docs: Handle 429 Too Many Requests with backoff
                    let retry_after = response
                        .headers()
                        .get("retry-after")
                        .and_then(|h| h.to_str().ok())
                        .and_then(|s| s.parse::<u64>().ok())
                        .unwrap_or(60); // Default to 60 seconds if no header

                    Err(IaGetError::Network(format!(
                        "Rate limited by Internet Archive. Retry after {} seconds. File: {}",
                        retry_after, ctx.file_info.name
                    )))
                }
                reqwest::StatusCode::SERVICE_UNAVAILABLE => {
                    // IA API docs: 503 Service Unavailable - may be temporary
                    Err(IaGetError::Network(format!(
                        "Internet Archive server temporarily unavailable (503). File: {}",
                        ctx.file_info.name
                    )))
                }
                reqwest::StatusCode::NOT_FOUND => Err(IaGetError::Network(format!(
                    "File not found on Internet Archive (404): {}",
                    ctx.file_info.name
                ))),
                _ => Err(IaGetError::Network(format!(
                    "HTTP error {}: {} for file: {}",
                    status,
                    status.canonical_reason().unwrap_or("Unknown error"),
                    ctx.file_info.name
                ))),
            };
        }

        // Verify Content-Length if available
        let content_length = response.content_length();
        if let (Some(expected_size), Some(content_len)) = (ctx.file_info.size, content_length) {
            let expected_remaining = if ctx.resume_from > 0 {
                expected_size.saturating_sub(ctx.resume_from)
            } else {
                expected_size
            };

            if content_len != expected_remaining {
                ctx.progress_bar.set_message(format!(
                    "Warning: Content-Length mismatch for {}. Expected {} bytes, server reports {} bytes",
                    ctx.file_info.name, expected_remaining, content_len
                ));
            }
        }

        // Set up progress bar with file size
        if let Some(total_size) = ctx.file_info.size {
            ctx.progress_bar.set_length(total_size);
            if ctx.resume_from > 0 {
                ctx.progress_bar.set_position(ctx.resume_from);
            }
        }

        // Create or open temporary file for writing
        let mut file = if ctx.resume_from > 0 {
            tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(ctx.temp_path)
                .await
                .map_err(|e| {
                    IaGetError::FileSystem(format!(
                        "Failed to open temporary file for resume: {}",
                        e
                    ))
                })?
        } else {
            File::create(ctx.temp_path).await.map_err(|e| {
                IaGetError::FileSystem(format!("Failed to create temporary file: {}", e))
            })?
        };

        // Download with progress tracking
        let mut downloaded = ctx.resume_from;

        use futures_util::StreamExt;
        let mut stream = response.bytes_stream();
        while let Some(chunk_result) = stream.next().await {
            let chunk = match chunk_result {
                Ok(bytes) => bytes,
                Err(e) => {
                    // Enhanced error handling for Internet Archive specific issues
                    let error_msg = if e.to_string().to_lowercase().contains("decode")
                        || e.to_string().to_lowercase().contains("decompress")
                    {
                        format!(
                            "Compression decode error for {}: {}. Server may have sent corrupted compressed data. Will retry.",
                            ctx.file_info.name, e
                        )
                    } else if e.to_string().to_lowercase().contains("timeout") {
                        format!(
                            "Download timeout for {}: {}. File may be large or Internet Archive server is busy.",
                            ctx.file_info.name, e
                        )
                    } else if e.to_string().to_lowercase().contains("connection") {
                        format!(
                            "Connection lost for {}: {}. Will retry with exponential backoff.",
                            ctx.file_info.name, e
                        )
                    } else {
                        format!(
                            "Download stream error for {}: {}. Will attempt retry.",
                            ctx.file_info.name, e
                        )
                    };
                    return Err(IaGetError::Network(error_msg));
                }
            };

            file.write_all(&chunk)
                .await
                .map_err(|e| IaGetError::FileSystem(format!("Failed to write to file: {}", e)))?;

            downloaded += chunk.len() as u64;
            ctx.progress_bar.set_position(downloaded);
        }

        // Ensure all data is written
        file.flush()
            .await
            .map_err(|e| IaGetError::FileSystem(format!("Failed to flush file: {}", e)))?;
        drop(file);

        // Verify that we downloaded the expected amount of data
        if let Some(expected_size) = ctx.file_info.size {
            if downloaded != expected_size {
                // Don't delete the temp file - we might be able to resume
                return Err(IaGetError::Network(format!(
                    "Download incomplete: expected {} bytes, got {} bytes for {}",
                    expected_size, downloaded, ctx.file_info.name
                )));
            }
        }

        // Move temporary file to final location
        tokio::fs::rename(ctx.temp_path, ctx.output_path)
            .await
            .map_err(|e| IaGetError::FileSystem(format!("Failed to finalize file: {}", e)))?;

        Ok(())
    }

    /// Create or resume an existing download session
    async fn create_or_resume_session(
        &self,
        original_url: String,
        identifier: String,
        archive_metadata: ArchiveMetadata,
        download_config: DownloadConfig,
        requested_files: Vec<String>,
    ) -> Result<DownloadSession> {
        // Try to find existing session
        if let Ok(Some(session_file)) = crate::core::session::find_latest_session_file(
            &identifier,
            &self.session_dir.to_string_lossy(),
        ) {
            if let Ok(mut existing_session) = DownloadSession::load_from_file(&session_file) {
                // Update with any new files that weren't in the original session
                for file_name in &requested_files {
                    if !existing_session.file_status.contains_key(file_name) {
                        if let Some(file_info) =
                            archive_metadata.files.iter().find(|f| f.name == *file_name)
                        {
                            let sanitized_filename =
                                crate::core::session::sanitize_filename_for_filesystem(file_name);
                            let local_path =
                                format!("{}/{}", download_config.output_dir, sanitized_filename);

                            // Validate path length for Windows compatibility
                            if let Err(e) = crate::core::session::validate_path_length(
                                &download_config.output_dir,
                                &sanitized_filename,
                            ) {
                                eprintln!("⚠️  Warning: {}", e);
                            }
                            existing_session.file_status.insert(
                                file_name.clone(),
                                FileDownloadStatus {
                                    file_info: file_info.clone(),
                                    status: DownloadState::Pending,
                                    bytes_downloaded: 0,
                                    started_at: None,
                                    completed_at: None,
                                    error_message: None,
                                    retry_count: 0,
                                    server_used: None,
                                    local_path,
                                },
                            );
                        }
                    }
                }

                return Ok(existing_session);
            }
        }

        // Create new session
        Ok(DownloadSession::new(
            original_url,
            identifier,
            archive_metadata,
            download_config,
            requested_files,
        ))
    }
}
