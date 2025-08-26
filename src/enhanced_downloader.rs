//! Enhanced downloader module for ia-get
//!
//! Implements comprehensive downloading with metadata storage, resume functionality,
//! and full compliance with Internet Archive API recommendations.

use crate::{
    Result, 
    IaGetError,
    metadata_storage::{ArchiveMetadata, ArchiveFile, DownloadSession, DownloadConfig, DownloadState, FileDownloadStatus},
};
use reqwest::Client;
use std::path::{Path, PathBuf};
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use colored::*;
use indicatif::{ProgressBar, ProgressStyle, MultiProgress};
use std::sync::Arc;
use tokio::sync::Semaphore;

/// Enhanced downloader that uses full Archive.org metadata
pub struct ArchiveDownloader {
    client: Client,
    max_concurrent: usize,
    verify_md5: bool,
    preserve_mtime: bool,
    session_dir: PathBuf,
}

impl ArchiveDownloader {
    /// Create a new downloader instance
    pub fn new(
        client: Client,
        max_concurrent: usize,
        verify_md5: bool,
        preserve_mtime: bool,
        session_dir: PathBuf,
    ) -> Self {
        Self {
            client,
            max_concurrent,
            verify_md5,
            preserve_mtime,
            session_dir,
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
        let mut session = self.create_or_resume_session(
            original_url,
            identifier.clone(),
            archive_metadata,
            download_config,
            requested_files,
        ).await?;

        // Create session directory if it doesn't exist
        tokio::fs::create_dir_all(&self.session_dir).await
            .map_err(|e| IaGetError::FileSystem(format!("Failed to create session directory: {}", e)))?;

        // Save initial session state
        let session_file = self.session_dir.join(crate::metadata_storage::generate_session_filename(&identifier));
        session.save_to_file(&session_file)?;

        progress_bar.set_message("Initializing downloads...".to_string());

        // Get pending files to download
        let pending_files: Vec<String> = session.get_pending_files().into_iter().map(|s| s.to_string()).collect();
        
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
                
                // Create individual progress bar for this file
                let file_progress = multi_progress.add(ProgressBar::new(file_info.size.unwrap_or(0)));
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
                        file_progress,
                    ).await
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
                    main_progress.set_message(format!("Completed: {}, Failed: {}", completed, failed));
                }
                Ok(Err(e)) => {
                    session.update_file_status(&file_name, DownloadState::Failed);
                    if let Some(file_status) = session.file_status.get_mut(&file_name) {
                        file_status.error_message = Some(e.to_string());
                    }
                    failed += 1;
                    main_progress.inc(1);
                    main_progress.set_message(format!("Completed: {}, Failed: {}", completed, failed));
                    eprintln!("{} Failed to download {}: {}", "✘".red(), file_name, e);
                }
                Err(e) => {
                    session.update_file_status(&file_name, DownloadState::Failed);
                    if let Some(file_status) = session.file_status.get_mut(&file_name) {
                        file_status.error_message = Some(format!("Task join error: {}", e));
                    }
                    failed += 1;
                    main_progress.inc(1);
                    main_progress.set_message(format!("Completed: {}, Failed: {}", completed, failed));
                    eprintln!("{} Task failed for {}: {}", "✘".red(), file_name, e);
                }
            }
        }

        // Save final session state
        session.save_to_file(&session_file)?;

        if failed == 0 {
            main_progress.finish_with_message(format!("✓ Successfully downloaded {} files", completed).green().to_string());
        } else {
            main_progress.finish_with_message(format!("⚠ Completed {} files, {} failed", completed, failed).yellow().to_string());
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
        progress_bar: ProgressBar,
    ) -> Result<()> {
        // Create output directory if it doesn't exist
        if let Some(parent) = output_path.parent() {
            tokio::fs::create_dir_all(parent).await
                .map_err(|e| IaGetError::FileSystem(format!("Failed to create output directory: {}", e)))?;
        }

        // Check if file already exists and is valid
        if output_path.exists() {
            if verify_md5 && file_info.md5.is_some() {
                progress_bar.set_message(format!("Verifying existing {}", file_info.name));
                
                // Convert async path to sync for MD5 calculation
                let path_str = output_path.to_string_lossy().to_string();
                let file_info_clone = file_info.clone();
                let validation_result = tokio::task::spawn_blocking(move || {
                    file_info_clone.validate_md5(&path_str)
                }).await
                .map_err(|e| IaGetError::Network(format!("MD5 validation task failed: {}", e)))??;
                
                if validation_result {
                    progress_bar.finish_with_message(format!("✓ {} already exists and is valid", file_info.name).green().to_string());
                    return Ok(());
                } else {
                    progress_bar.set_message(format!("MD5 mismatch, re-downloading {}", file_info.name));
                }
            } else {
                progress_bar.finish_with_message(format!("✓ {} already exists (skipping verification)", file_info.name).yellow().to_string());
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
            ).await {
                Ok(_) => {
                    // Verify MD5 if required and available
                    if verify_md5 && file_info.md5.is_some() {
                        progress_bar.set_message(format!("Verifying {}", file_info.name));
                        
                        let path_str = output_path.to_string_lossy().to_string();
                        let file_info_clone = file_info.clone();
                        let validation_result = tokio::task::spawn_blocking(move || {
                            file_info_clone.validate_md5(&path_str)
                        }).await
                        .map_err(|e| IaGetError::Network(format!("MD5 validation task failed: {}", e)))??;
                        
                        if !validation_result {
                            let error_msg = format!("MD5 verification failed for {}", file_info.name);
                            progress_bar.finish_with_message(format!("✘ {}", error_msg).red().to_string());
                            
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
                        }).await;
                    }

                    progress_bar.finish_with_message(format!("✓ Downloaded {}", file_info.name).green().to_string());
                    return Ok(());
                }
                Err(e) => {
                    progress_bar.set_message(format!("Failed from {}, trying next server...", server));
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
            last_error.as_ref().map(|e| e.to_string()).unwrap_or_else(|| "Unknown error".to_string())
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
        let response = client.get(url)
            .send()
            .await
            .map_err(|e| IaGetError::Network(format!("Failed to start download: {}", e)))?;

        if !response.status().is_success() {
            return Err(IaGetError::Network(format!(
                "HTTP error {}: {}",
                response.status(),
                response.status().canonical_reason().unwrap_or("Unknown error")
            )));
        }

        // Set up progress bar with file size
        if let Some(total_size) = file_info.size {
            progress_bar.set_length(total_size);
        }

        // Create temporary file
        let temp_path = output_path.with_extension("tmp");
        let mut file = File::create(&temp_path).await
            .map_err(|e| IaGetError::FileSystem(format!("Failed to create temporary file: {}", e)))?;

        // Download with progress tracking
        let mut downloaded = 0u64;
        
        use futures_util::StreamExt;
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk
                .map_err(|e| IaGetError::Network(format!("Download stream error: {}", e)))?;
            
            file.write_all(&chunk).await
                .map_err(|e| IaGetError::FileSystem(format!("Failed to write to file: {}", e)))?;
            
            downloaded += chunk.len() as u64;
            progress_bar.set_position(downloaded);
        }

        // Ensure all data is written
        file.flush().await
            .map_err(|e| IaGetError::FileSystem(format!("Failed to flush file: {}", e)))?;
        drop(file);

        // Move temporary file to final location
        tokio::fs::rename(&temp_path, output_path).await
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
        if let Ok(Some(session_file)) = crate::metadata_storage::find_latest_session_file(&identifier, &self.session_dir.to_string_lossy()) {
            if let Ok(mut existing_session) = DownloadSession::load_from_file(&session_file) {
                // Update with any new files that weren't in the original session
                for file_name in &requested_files {
                    if !existing_session.file_status.contains_key(file_name) {
                        if let Some(file_info) = archive_metadata.files.iter().find(|f| f.name == *file_name) {
                            let local_path = format!("{}/{}", download_config.output_dir, file_name);
                            existing_session.file_status.insert(file_name.clone(), FileDownloadStatus {
                                file_info: file_info.clone(),
                                status: DownloadState::Pending,
                                bytes_downloaded: 0,
                                started_at: None,
                                completed_at: None,
                                error_message: None,
                                retry_count: 0,
                                server_used: None,
                                local_path,
                            });
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
