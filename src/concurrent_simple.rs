//! Enhanced concurrent downloader for Internet Archive files
//!
//! This module provides an improved concurrent downloader that integrates with the
//! existing metadata structures and includes session tracking, progress reporting,
//! and comprehensive statistics.
//!
//! ## Features
//!
//! - **Concurrent Downloads**: Parallel file downloads with semaphore-based rate limiting
//! - **Session Integration**: Works with DownloadSession for resume capability  
//! - **Progress Tracking**: Real-time download statistics and speed monitoring
//! - **Error Handling**: Robust error handling with detailed failure reporting
//! - **Server Selection**: Uses the optimal Internet Archive servers for downloads
//!
//! ## Usage
//!
//! ```rust,no_run
//! use ia_get::concurrent_simple::SimpleConcurrentDownloader;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Create downloader with max 4 concurrent downloads
//!     let downloader = SimpleConcurrentDownloader::new(4)?;
//!
//!     // Download files concurrently (example - would need actual metadata)
//!     // let results = downloader.download_files(&metadata, files, "output").await?;
//!     Ok(())
//! }
//! ```
//!
//! ## Architecture
//!
//! The downloader uses tokio's semaphore system to limit concurrent downloads and
//! integrates with the metadata storage system for session tracking and resume functionality.

use crate::{
    constants::get_user_agent,
    metadata_storage::{ArchiveFile, ArchiveMetadata, DownloadSession, DownloadState},
    IaGetError, Result,
};
use reqwest::Client;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, Semaphore};

/// Download statistics for tracking progress
#[derive(Debug, Clone)]
pub struct DownloadStats {
    pub total_files: usize,
    pub completed_files: usize,
    pub failed_files: usize,
    pub skipped_files: usize,
    pub total_bytes: u64,
    pub downloaded_bytes: u64,
    pub start_time: Instant,
    pub current_speed: f64, // bytes per second
}

impl DownloadStats {
    pub fn new(total_files: usize, total_bytes: u64) -> Self {
        Self {
            total_files,
            completed_files: 0,
            failed_files: 0,
            skipped_files: 0,
            total_bytes,
            downloaded_bytes: 0,
            start_time: Instant::now(),
            current_speed: 0.0,
        }
    }

    pub fn update_progress(&mut self, additional_bytes: u64) {
        self.downloaded_bytes += additional_bytes;
        let elapsed = self.start_time.elapsed().as_secs_f64();
        if elapsed > 0.0 {
            self.current_speed = self.downloaded_bytes as f64 / elapsed;
        }
    }

    pub fn completion_percentage(&self) -> f64 {
        if self.total_files == 0 {
            0.0
        } else {
            (self.completed_files as f64 / self.total_files as f64) * 100.0
        }
    }

    pub fn eta_seconds(&self) -> Option<u64> {
        if self.current_speed <= 0.0 || self.downloaded_bytes >= self.total_bytes {
            return None;
        }
        let remaining_bytes = self.total_bytes.saturating_sub(self.downloaded_bytes);
        Some((remaining_bytes as f64 / self.current_speed) as u64)
    }

    pub fn format_speed(&self) -> String {
        crate::filters::format_size(self.current_speed as u64) + "/s"
    }
}

/// Enhanced concurrent downloader for Archive.org files
pub struct SimpleConcurrentDownloader {
    client: Client,
    semaphore: Arc<Semaphore>,
    stats: Arc<Mutex<DownloadStats>>,
}

/// Download result for a single file with enhanced information
#[derive(Debug)]
pub struct FileDownloadResult {
    pub file_name: String,
    pub success: bool,
    pub bytes_downloaded: u64,
    pub error: Option<String>,
    pub duration: Duration,
    pub server_used: String,
}

impl SimpleConcurrentDownloader {
    /// Create a new enhanced concurrent downloader
    pub fn new(max_concurrent: usize) -> Result<Self> {
        let client = Client::builder()
            .user_agent(get_user_agent())
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .map_err(IaGetError::from)?;

        Ok(Self {
            client,
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            stats: Arc::new(Mutex::new(DownloadStats::new(0, 0))),
        })
    }

    /// Get current download statistics
    pub async fn get_stats(&self) -> DownloadStats {
        self.stats.lock().await.clone()
    }

    /// Update download session with file status
    pub async fn update_session_status(
        &self,
        session: &mut DownloadSession,
        file_name: &str,
        status: DownloadState,
        bytes_downloaded: u64,
        error_message: Option<String>,
    ) {
        if let Some(file_status) = session.file_status.get_mut(file_name) {
            file_status.status = status.clone();
            file_status.bytes_downloaded = bytes_downloaded;
            file_status.error_message = error_message;

            match status {
                DownloadState::InProgress => {
                    file_status.started_at = Some(
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs(),
                    );
                }
                DownloadState::Completed | DownloadState::Failed => {
                    file_status.completed_at = Some(
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs(),
                    );
                }
                _ => {}
            }
        }

        // Update session timestamp
        session.last_updated = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
    }

    /// Download files concurrently with basic progress tracking
    pub async fn download_files(
        &self,
        metadata: &ArchiveMetadata,
        files_to_download: Vec<String>,
        output_dir: &str,
    ) -> Result<Vec<FileDownloadResult>> {
        // Filter files based on request
        let files: Vec<&ArchiveFile> = metadata
            .files
            .iter()
            .filter(|f| files_to_download.contains(&f.name))
            .collect();

        if files.is_empty() {
            return Err(IaGetError::NoFilesFound(
                "No matching files found".to_string(),
            ));
        }

        println!("🚀 Starting download of {} files", files.len());
        let start_time = Instant::now();

        // Create download tasks
        let mut tasks = vec![];
        for file in files {
            let task = self.download_single_file(
                file.clone(),
                metadata.server.clone(),
                output_dir.to_string(),
            );
            tasks.push(task);
        }

        // Execute downloads concurrently
        let results = futures::future::join_all(tasks).await;

        // Process results
        let mut download_results = vec![];
        let mut total_downloaded = 0u64;
        let mut successful = 0;
        let mut failed = 0;

        for result in results {
            match result {
                Ok(file_result) => {
                    if file_result.success {
                        successful += 1;
                        total_downloaded += file_result.bytes_downloaded;
                    } else {
                        failed += 1;
                    }
                    download_results.push(file_result);
                }
                Err(e) => {
                    failed += 1;
                    download_results.push(FileDownloadResult {
                        file_name: "unknown".to_string(),
                        success: false,
                        bytes_downloaded: 0,
                        error: Some(e.to_string()),
                        duration: Duration::from_secs(0),
                        server_used: "unknown".to_string(),
                    });
                }
            }
        }

        let duration = start_time.elapsed();
        println!("✅ Download completed in {:.1}s", duration.as_secs_f64());
        println!("📊 Success: {}, Failed: {}", successful, failed);
        if total_downloaded > 0 {
            println!(
                "📦 Total downloaded: {}",
                crate::filters::format_size(total_downloaded)
            );
        }

        Ok(download_results)
    }

    /// Download a single file with enhanced tracking
    async fn download_single_file(
        &self,
        file: ArchiveFile,
        server: String,
        output_dir: String,
    ) -> Result<FileDownloadResult> {
        let _permit = self.semaphore.acquire().await.unwrap();
        let start_time = Instant::now();

        let url = format!("https://{}{}/{}", server, &server, file.name);
        let file_path = std::path::Path::new(&output_dir).join(&file.name);

        // Create output directory if needed
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent).map_err(IaGetError::Io)?;
        }

        match self.download_file_content(&url, &file_path).await {
            Ok(bytes) => {
                let duration = start_time.elapsed();
                Ok(FileDownloadResult {
                    file_name: file.name,
                    success: true,
                    bytes_downloaded: bytes,
                    error: None,
                    duration,
                    server_used: server,
                })
            }
            Err(e) => {
                let duration = start_time.elapsed();
                Ok(FileDownloadResult {
                    file_name: file.name,
                    success: false,
                    bytes_downloaded: 0,
                    error: Some(e.to_string()),
                    duration,
                    server_used: server,
                })
            }
        }
    }

    /// Download file content to disk
    async fn download_file_content(&self, url: &str, file_path: &std::path::Path) -> Result<u64> {
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(IaGetError::from)?;

        if !response.status().is_success() {
            return Err(IaGetError::Network(format!(
                "HTTP {} error for {}",
                response.status(),
                url
            )));
        }

        let mut file = std::fs::File::create(file_path).map_err(IaGetError::Io)?;
        let mut bytes_downloaded = 0u64;
        let mut stream = response.bytes_stream();

        use futures::StreamExt;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(IaGetError::from)?;
            bytes_downloaded += chunk.len() as u64;
            std::io::Write::write_all(&mut file, &chunk).map_err(IaGetError::Io)?;
        }

        Ok(bytes_downloaded)
    }
}
