//! # ia-get
//!
//! A robust command-line tool for downloading files from the Internet Archive.
//!
//! ## Features
//!
//! - **Concurrent Downloads**: Fast parallel downloading with configurable concurrency limits
//! - **JSON API Integration**: Uses Internet Archive's modern JSON API for metadata
//! - **Session Management**: Resumable downloads with automatic session tracking
//! - **Compression Support**: Automatic decompression of common archive formats
//! - **Progress Tracking**: Real-time download progress and statistics
//! - **Error Handling**: Robust retry logic and comprehensive error reporting
//! - **Filtering**: Download specific files by format, size, or pattern
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use ia_get::{metadata::fetch_json_metadata, enhanced_downloader::ArchiveDownloader};
//! use reqwest::Client;
//! use indicatif::ProgressBar;
//! use std::path::PathBuf;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let client = Client::new();
//!     let progress = ProgressBar::new_spinner();
//!     
//!     // Fetch archive metadata
//!     let (metadata, _url) = fetch_json_metadata("identifier", &client, &progress).await?;
//!
//!     // Download with enhanced features
//!     let downloader = ArchiveDownloader::new(
//!         client, 4, true, true, PathBuf::from(".sessions"), false, false
//!     );
//!     // Use the downloader as needed...
//!     Ok(())
//! }
//! ```
//!
//! ## Architecture
//!
//! - [`metadata`]: JSON metadata fetching and parsing
//! - [`enhanced_downloader`]: Main download engine with session support
//! - [`concurrent_simple`]: Enhanced concurrent downloader
//! - [`metadata_storage`]: Session and file tracking structures
//! - [`compression`]: Automatic decompression utilities
//! - [`filters`]: File filtering and formatting utilities

pub mod archive_metadata;
pub mod cli;
pub mod compression;
pub mod concurrent_simple;
pub mod config;
pub mod constants;
pub mod downloader;
pub mod downloads;
pub mod enhanced_downloader;
pub mod error;
pub mod filters;
pub mod http_client;
pub mod interactive_menu;
pub mod metadata;
pub mod metadata_storage;
pub mod network;
pub mod performance;
pub mod url_processing;
pub mod utils;

// Re-export the error types for convenience
pub use error::{IaGetError, Result};

// Re-export commonly used functions
pub use cli::Cli;
pub use concurrent_simple::{DownloadStats, FileDownloadResult, SimpleConcurrentDownloader};
pub use constants::get_user_agent;
pub use downloads::download_files_with_retries;
pub use filters::{filter_files, format_size, parse_size_string};
pub use http_client::{ClientConfig, EnhancedHttpClient, HttpClientFactory};
pub use metadata::{fetch_json_metadata, get_json_url, parse_archive_metadata};
pub use network::{is_transient_error, is_transient_reqwest_error, is_url_accessible};
pub use performance::{AdaptiveBufferManager, PerformanceMetrics, PerformanceMonitor};
pub use url_processing::{
    construct_download_url, construct_metadata_url, extract_identifier_from_url, is_archive_url,
    validate_and_process_url,
};
