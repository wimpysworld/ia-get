//! # ia-get
//!
//! A command-line tool for downloading files from the Internet Archive.
//! 
//! This tool takes an archive.org details URL and downloads all associated files,
//! with support for resumable downloads and MD5 hash verification.

pub mod error;
pub mod utils;
pub mod archive_metadata;
pub mod downloader;
pub mod constants;
pub mod network;
pub mod url_processing;
pub mod cli;
pub mod metadata;
pub mod downloads;
pub mod enhanced_downloader;
pub mod metadata_storage;
pub mod filters;
pub mod compression;

// Re-export the error types for convenience
pub use error::{IaGetError, Result};

// Re-export commonly used functions
pub use network::{is_url_accessible, is_transient_reqwest_error, is_transient_error};
pub use url_processing::{validate_and_process_url, is_archive_url, extract_identifier_from_url, construct_metadata_url, construct_download_url};
pub use constants::get_user_agent;
pub use cli::Cli;
pub use metadata::{get_xml_url, fetch_xml_metadata};
pub use downloads::download_files_with_retries;

// Placeholder for fetch_json_metadata until properly implemented
pub async fn fetch_json_metadata(url: &str, client: &reqwest::Client) -> crate::Result<(crate::metadata_storage::ArchiveMetadata, String)> {
    use crate::metadata_storage::ArchiveMetadata;
    
    // Extract identifier from URL
    let identifier = if url.contains("/details/") {
        url.split("/details/").nth(1).unwrap_or("").split('/').next().unwrap_or("")
    } else {
        url.trim_end_matches('/').rsplit('/').next().unwrap_or("")
    };
    
    if identifier.is_empty() {
        return Err(crate::error::IaGetError::Parse("Could not extract identifier from URL".to_string()));
    }
    
    // Construct JSON metadata URL
    let metadata_url = format!("https://archive.org/metadata/{}", identifier);
    let base_url = format!("https://archive.org/download/{}/", identifier);
    
    // Fetch JSON metadata
    let response = client.get(&metadata_url)
        .send()
        .await
        .map_err(|e| crate::error::IaGetError::Network(format!("Failed to fetch metadata: {}", e)))?;
    
    if !response.status().is_success() {
        return Err(crate::error::IaGetError::Network(format!(
            "HTTP error {}: {}",
            response.status(),
            response.status().canonical_reason().unwrap_or("Unknown error")
        )));
    }
    
    let text = response.text().await
        .map_err(|e| crate::error::IaGetError::Network(format!("Failed to read response: {}", e)))?;
    
    let metadata: ArchiveMetadata = serde_json::from_str(&text)
        .map_err(|e| crate::error::IaGetError::Parse(format!("Failed to parse JSON metadata: {}", e)))?;
    
    Ok((metadata, base_url))
}