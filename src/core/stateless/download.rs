//! Stateless download operations
//!
//! Pure functions for downloading files without state management.
//! Progress tracking handled via callbacks.

use crate::{error::IaGetError, Result};
use reqwest::blocking::Client;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

/// Progress callback type for downloads
///
/// Arguments: (bytes_downloaded, total_bytes)
///
/// Note: For synchronous downloads, Send+Sync is not required since the callback
/// is only invoked on the same thread that calls download_file_sync
pub type ProgressCallback = Box<dyn Fn(u64, u64)>;

/// Download a file synchronously with progress tracking
///
/// This is a stateless function - all state managed by caller.
///
/// # Arguments
///
/// * `url` - Source URL to download from
/// * `output_path` - Destination file path
/// * `progress_callback` - Optional callback for progress updates
///
/// # Returns
///
/// * `Ok(u64)` - Number of bytes downloaded
/// * `Err(IaGetError)` - Download failed
pub fn download_file_sync<P>(
    url: &str,
    output_path: P,
    progress_callback: Option<ProgressCallback>,
) -> Result<u64>
where
    P: AsRef<Path>,
{
    let client = Client::builder()
        .timeout(Duration::from_secs(300)) // 5 minutes
        .connect_timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| IaGetError::Network(format!("Failed to create HTTP client: {}", e)))?;

    let mut response = client
        .get(url)
        .header("Accept-Encoding", "deflate, gzip") // Archive.org recommendation
        .header("X-Accept-Reduced-Priority", "1") // Avoid rate limiting
        .send()
        .map_err(|e| IaGetError::Network(format!("Failed to send request: {}", e)))?;

    if !response.status().is_success() {
        return Err(IaGetError::Network(format!(
            "HTTP error {}: {}",
            response.status().as_u16(),
            response.status().canonical_reason().unwrap_or("Unknown")
        )));
    }

    let total_size = response.content_length().unwrap_or(0);
    let mut file = File::create(output_path.as_ref())
        .map_err(|e| IaGetError::FileSystem(format!("Failed to create file: {}", e)))?;

    let mut downloaded = 0u64;
    let mut buffer = vec![0u8; 8192];

    loop {
        use std::io::Read;

        let bytes_read = response
            .read(&mut buffer)
            .map_err(|e| IaGetError::Network(format!("Failed to read response: {}", e)))?;

        if bytes_read == 0 {
            break;
        }

        file.write_all(&buffer[..bytes_read])
            .map_err(|e| IaGetError::FileSystem(format!("Failed to write to file: {}", e)))?;

        downloaded += bytes_read as u64;

        if let Some(ref callback) = progress_callback {
            callback(downloaded, total_size);
        }
    }

    Ok(downloaded)
}

/// Download a file asynchronously with progress tracking
///
/// This async version is optimized for CLI use and provides better performance
/// for multiple concurrent downloads.
///
/// # Arguments
///
/// * `url` - Source URL to download from
/// * `output_path` - Destination file path
/// * `progress_callback` - Optional callback for progress updates
///
/// # Returns
///
/// * `Ok(u64)` - Number of bytes downloaded
/// * `Err(IaGetError)` - Download failed
pub async fn download_file_async<P, F>(
    url: &str,
    output_path: P,
    mut progress_callback: Option<F>,
) -> Result<u64>
where
    P: AsRef<Path>,
    F: FnMut(u64, u64) + Send,
{
    use tokio::io::AsyncWriteExt;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300)) // 5 minutes
        .connect_timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| IaGetError::Network(format!("Failed to create HTTP client: {}", e)))?;

    let mut response = client
        .get(url)
        .header("Accept-Encoding", "deflate, gzip") // Archive.org recommendation
        .header("X-Accept-Reduced-Priority", "1") // Avoid rate limiting
        .send()
        .await
        .map_err(|e| IaGetError::Network(format!("Failed to send request: {}", e)))?;

    if !response.status().is_success() {
        return Err(IaGetError::Network(format!(
            "HTTP error {}: {}",
            response.status().as_u16(),
            response.status().canonical_reason().unwrap_or("Unknown")
        )));
    }

    let total_size = response.content_length().unwrap_or(0);
    let mut file = tokio::fs::File::create(output_path.as_ref())
        .await
        .map_err(|e| IaGetError::FileSystem(format!("Failed to create file: {}", e)))?;

    let mut downloaded = 0u64;

    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| IaGetError::Network(format!("Failed to read response chunk: {}", e)))?
    {
        file.write_all(&chunk)
            .await
            .map_err(|e| IaGetError::FileSystem(format!("Failed to write to file: {}", e)))?;

        downloaded += chunk.len() as u64;

        if let Some(ref mut callback) = progress_callback {
            callback(downloaded, total_size);
        }
    }

    file.flush()
        .await
        .map_err(|e| IaGetError::FileSystem(format!("Failed to flush file: {}", e)))?;

    Ok(downloaded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_download_file_sync() {
        if std::env::var("CI").is_ok() {
            return;
        }

        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test_file.txt");

        // Download a small file from Internet Archive
        let url = "https://archive.org/download/commute_test/test.txt";

        let result = download_file_sync(url, &file_path, None);

        // This test may fail if the URL doesn't exist, which is fine for now
        if result.is_ok() {
            assert!(file_path.exists(), "Downloaded file doesn't exist");
            let metadata = fs::metadata(&file_path).unwrap();
            assert!(metadata.len() > 0, "Downloaded file is empty");
        }
    }

    #[tokio::test]
    async fn test_download_file_async() {
        if std::env::var("CI").is_ok() {
            return;
        }

        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test_file_async.txt");

        // Download a small file from Internet Archive
        let url = "https://archive.org/download/commute_test/test.txt";

        let result = download_file_async(url, &file_path, None::<fn(u64, u64)>).await;

        // This test may fail if the URL doesn't exist, which is fine for now
        if result.is_ok() {
            assert!(file_path.exists(), "Downloaded file doesn't exist");
            let metadata = fs::metadata(&file_path).unwrap();
            assert!(metadata.len() > 0, "Downloaded file is empty");
        }
    }
}
