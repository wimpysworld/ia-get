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
pub async fn fetch_json_metadata(_url: &str, _client: &reqwest::Client) -> crate::Result<serde_json::Value> {
    Err(crate::error::IaGetError::Parse("JSON metadata not yet implemented".to_string()))
}