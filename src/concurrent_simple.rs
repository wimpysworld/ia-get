//! Simple concurrent downloader for Internet Archive files
//!
//! This module provides a simplified concurrent downloader that works
//! with the existing metadata structures without complex session tracking.

use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Semaphore;
use reqwest::Client;
use crate::{
    Result, IaGetError,
    metadata_storage::{ArchiveFile, ArchiveMetadata, DownloadConfig},
    constants::get_user_agent,
};

/// Simple concurrent downloader for Archive.org files
pub struct SimpleConcurrentDownloader {
    client: Client,
    semaphore: Arc<Semaphore>,
}

/// Download result for a single file
#[derive(Debug)]
pub struct FileDownloadResult {
    pub file_name: String,
    pub success: bool,
    pub bytes_downloaded: u64,
    pub error: Option<String>,
}

impl SimpleConcurrentDownloader {
    /// Create a new simple concurrent downloader
    pub fn new(max_concurrent: usize) -> Result<Self> {
        let client = Client::builder()
            .user_agent(get_user_agent())
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .map_err(IaGetError::from)?;

        Ok(Self {
            client,
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
        })
    }

    /// Download files concurrently with basic progress tracking
    pub async fn download_files(
        &self,
        metadata: &ArchiveMetadata,
        files_to_download: Vec<String>,
        output_dir: &str,
    ) -> Result<Vec<FileDownloadResult>> {
        // Filter files based on request
        let files: Vec<&ArchiveFile> = metadata.files.iter()
            .filter(|f| files_to_download.contains(&f.name))
            .collect();

        if files.is_empty() {
            return Err(IaGetError::NoFilesFound("No matching files found".to_string()));
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
                    });
                }
            }
        }

        let duration = start_time.elapsed();
        println!("✅ Download completed in {:.1}s", duration.as_secs_f64());
        println!("📊 Success: {}, Failed: {}", successful, failed);
        if total_downloaded > 0 {
            println!("📦 Total downloaded: {}", crate::filters::format_size(total_downloaded));
        }

        Ok(download_results)
    }

    /// Download a single file
    async fn download_single_file(
        &self,
        file: ArchiveFile,
        server: String,
        output_dir: String,
    ) -> Result<FileDownloadResult> {
        let _permit = self.semaphore.acquire().await.unwrap();

        let url = format!("https://{}{}/{}", server, &server, file.name);
        let file_path = std::path::Path::new(&output_dir).join(&file.name);

        // Create output directory if needed
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent).map_err(IaGetError::Io)?;
        }

        match self.download_file_content(&url, &file_path).await {
            Ok(bytes) => Ok(FileDownloadResult {
                file_name: file.name,
                success: true,
                bytes_downloaded: bytes,
                error: None,
            }),
            Err(e) => Ok(FileDownloadResult {
                file_name: file.name,
                success: false,
                bytes_downloaded: 0,
                error: Some(e.to_string()),
            }),
        }
    }

    /// Download file content to disk
    async fn download_file_content(
        &self,
        url: &str,
        file_path: &std::path::Path,
    ) -> Result<u64> {
        let response = self.client.get(url).send().await.map_err(IaGetError::from)?;
        
        if !response.status().is_success() {
            return Err(IaGetError::Network(format!("HTTP {} error for {}", response.status(), url)));
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
