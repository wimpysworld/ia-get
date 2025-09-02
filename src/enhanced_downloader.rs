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
    metadata_storage::{
        ArchiveFile, ArchiveMetadata, DownloadConfig, DownloadSession, DownloadState,
        FileDownloadStatus,
    },
    IaGetError, Result,
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
    enable_compression: bool,
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
        let session_file =
            self.session_dir
                .join(crate::metadata_storage::generate_session_filename(
                    &identifier,
                ));
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
                .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} files ({msg})")
                .unwrap()
                .progress_chars("##-")
        );
        main_progress.set_message("Starting downloads".to_string());

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
                let enable_compression = self.enable_compression;
                let auto_decompress = self.auto_decompress;
                let decompress_formats = session.download_config.decompress_formats.clone();

                // Create individual progress bar for this file
                let file_progress =
                    multi_progress.add(ProgressBar::new(file_info.size.unwrap_or(0)));
                file_progress.set_style(
                    ProgressStyle::default_bar()
                        .template("{spinner:.green} {msg} [{bar:30.cyan/blue}] {bytes}/{total_bytes} ({eta})")
                        .unwrap()
                        .progress_chars("##-")
                );
                file_progress.set_message(format!("Downloading {}", file_info.name));

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
                        enable_compression,
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

        for (file_name, handle) in handles {
            match handle.await {
                Ok(Ok(_)) => {
                    session.update_file_status(&file_name, DownloadState::Completed);
                    completed += 1;
                    main_progress.inc(1);
                    main_progress
                        .set_message(format!("Completed: {}, Failed: {}", completed, failed));
                }
                Ok(Err(e)) => {
                    session.update_file_status(&file_name, DownloadState::Failed);
                    if let Some(file_status) = session.file_status.get_mut(&file_name) {
                        file_status.error_message = Some(e.to_string());
                    }
                    failed += 1;
                    main_progress.inc(1);
                    main_progress
                        .set_message(format!("Completed: {}, Failed: {}", completed, failed));
                    eprintln!("{} Failed to download {}: {}", "✘".red(), file_name, e);
                }
                Err(e) => {
                    session.update_file_status(&file_name, DownloadState::Failed);
                    if let Some(file_status) = session.file_status.get_mut(&file_name) {
                        file_status.error_message = Some(format!("Task join error: {}", e));
                    }
                    failed += 1;
                    main_progress.inc(1);
                    main_progress
                        .set_message(format!("Completed: {}, Failed: {}", completed, failed));
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
        enable_compression: bool,
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
                enable_compression,
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
                                crate::compression::CompressionFormat::from_filename(
                                    &file_info.name,
                                )
                            {
                                if crate::compression::should_decompress(
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
                                            crate::compression::decompress_file(
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
                    progress_bar
                        .set_message(format!("Failed from {}, trying next server...", server));
                    last_error = Some(e);

                    // Add delay before trying next server
                    if retry_count < servers.len() - 1 {
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    }
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
        enable_compression: bool,
    ) -> Result<()> {
        // Try with compression first, then fall back to no compression if decoding fails
        let max_attempts = if enable_compression { 2 } else { 1 };

        for attempt in 0..max_attempts {
            let use_compression = enable_compression && attempt == 0;

            match Self::attempt_download(
                client,
                url,
                output_path,
                file_info,
                progress_bar,
                use_compression,
            )
            .await
            {
                Ok(()) => return Ok(()),
                Err(e) => {
                    // If it's a decoding error and we haven't tried without compression yet
                    if attempt == 0 && enable_compression && e.to_string().contains("decoding")
                        || e.to_string().contains("decode")
                    {
                        progress_bar.set_message(format!(
                            "Compression decoding failed, retrying without compression: {}",
                            file_info.name
                        ));
                        continue;
                    }
                    return Err(e);
                }
            }
        }

        Err(IaGetError::Network(format!(
            "Failed to download {} after {} attempts",
            file_info.name, max_attempts
        )))
    }

    /// Attempt a single download with specified compression setting
    async fn attempt_download(
        client: &Client,
        url: &str,
        output_path: &Path,
        file_info: &ArchiveFile,
        progress_bar: &ProgressBar,
        enable_compression: bool,
    ) -> Result<()> {
        const MAX_RESUME_ATTEMPTS: usize = 3;
        let temp_path = output_path.with_extension("tmp");

        for attempt in 0..MAX_RESUME_ATTEMPTS {
            // Check if we have a partial file from previous attempt
            let resume_from = if attempt > 0 && temp_path.exists() {
                tokio::fs::metadata(&temp_path)
                    .await
                    .map(|m| m.len())
                    .unwrap_or(0)
            } else {
                0
            };

            // Create download context to avoid too many function arguments
            let download_ctx = DownloadContext {
                client,
                url,
                temp_path: &temp_path,
                output_path,
                file_info,
                progress_bar,
                enable_compression,
                resume_from,
            };

            match Self::perform_download(download_ctx).await {
                Ok(()) => return Ok(()),
                Err(e) => {
                    if attempt < MAX_RESUME_ATTEMPTS - 1 {
                        // Check if this looks like an incomplete download that we can resume
                        if e.to_string().contains("Download incomplete")
                            || e.to_string().contains("stream error")
                            || e.to_string().contains("connection")
                        {
                            progress_bar.set_message(format!(
                                "Download interrupted, resuming from {} bytes (attempt {}/{})",
                                resume_from,
                                attempt + 2,
                                MAX_RESUME_ATTEMPTS
                            ));
                            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                            continue;
                        }
                    }
                    return Err(e);
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

        // Enable HTTP compression if requested and not resuming
        // (compression and range requests don't mix well)
        if ctx.enable_compression && ctx.resume_from == 0 {
            request = request.header("Accept-Encoding", "gzip, deflate, br");
        }

        // Add range header for resume
        if ctx.resume_from > 0 {
            request = request.header("Range", format!("bytes={}-", ctx.resume_from));
        }

        let response = request
            .send()
            .await
            .map_err(|e| IaGetError::Network(format!("Failed to start download: {}", e)))?;

        if !response.status().is_success()
            && response.status() != reqwest::StatusCode::PARTIAL_CONTENT
        {
            return Err(IaGetError::Network(format!(
                "HTTP error {}: {}",
                response.status(),
                response
                    .status()
                    .canonical_reason()
                    .unwrap_or("Unknown error")
            )));
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
                    // Handle response body decoding errors more gracefully
                    let error_msg = if e.to_string().contains("decode") {
                        format!("Error decoding response body for {}: {}. This may be due to corrupted data or encoding issues.", ctx.file_info.name, e)
                    } else {
                        format!("Download stream error for {}: {}", ctx.file_info.name, e)
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
        if let Ok(Some(session_file)) = crate::metadata_storage::find_latest_session_file(
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
                                crate::metadata_storage::sanitize_filename_for_filesystem(
                                    file_name,
                                );
                            let local_path =
                                format!("{}/{}", download_config.output_dir, sanitized_filename);

                            // Validate path length for Windows compatibility
                            if let Err(e) = crate::metadata_storage::validate_path_length(
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
