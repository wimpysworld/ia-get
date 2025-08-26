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
//! ```rust
//! use ia_get::{metadata::fetch_json_metadata, enhanced_downloader::EnhancedDownloader};
//! 
//! // Fetch archive metadata
//! let metadata = fetch_json_metadata("identifier").await?;
//! 
//! // Download with enhanced features
//! let downloader = EnhancedDownloader::new(4)?; // 4 concurrent downloads
//! downloader.download_archive(&metadata, "output_dir", &config).await?;
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
pub use metadata::{get_json_url, fetch_json_metadata, parse_archive_metadata};
pub use downloads::download_files_with_retries;
pub use filters::{parse_size_string, filter_files, format_size};
pub use concurrent_simple::{SimpleConcurrentDownloader, DownloadStats, FileDownloadResult};
