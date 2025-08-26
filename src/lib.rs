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
pub mod concurrent_simple;
// pub mod concurrent;  // Temporarily disabled due to compilation issues
pub mod compression;
pub mod config;
pub mod interactive_menu;

// Re-export the error types for convenience
pub use error::{IaGetError, Result};

// Re-export commonly used functions
pub use network::{is_url_accessible, is_transient_reqwest_error, is_transient_error};
pub use url_processing::{validate_and_process_url, is_archive_url, extract_identifier_from_url, construct_metadata_url, construct_download_url};
pub use constants::get_user_agent;
pub use cli::Cli;
pub use metadata::{get_xml_url, get_json_url, fetch_xml_metadata, fetch_json_metadata, parse_archive_metadata};
pub use downloads::download_files_with_retries;
pub use filters::{parse_size_string, filter_files, format_size};
// pub use concurrent::{ConcurrentDownloader, DownloadStats, DownloadResult}; // Export concurrent downloader types
