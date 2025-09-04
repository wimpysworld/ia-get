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
//! - **GUI Interface**: Cross-platform graphical user interface using egui
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
//! - [`gui`]: Cross-platform graphical user interface

// Organized module structure
pub mod core;
pub mod error;
pub mod infrastructure;
pub mod interface;
pub mod utilities;

// Re-export the error types for convenience
pub use error::{IaGetError, Result};

// Re-export commonly used functions from organized modules
pub use core::archive::{
    fetch_json_metadata, get_json_url, parse_archive_metadata, AdvancedMetadataProcessor,
    MetadataAnalysis,
};
pub use core::download::{
    download_files_with_retries, DownloadRequest, DownloadResult, DownloadService, DownloadStats,
    FileDownloadResult, ProgressUpdate, SimpleConcurrentDownloader,
};
pub use core::session::{
    sanitize_filename_for_filesystem, ArchiveFile, ArchiveMetadata, DownloadConfig,
    DownloadSession, DownloadState,
};
pub use infrastructure::api::{
    validate_identifier, ApiStats, ArchiveOrgApiClient, EnhancedArchiveApiClient, ItemDetails,
    ServiceStatus,
};
pub use infrastructure::http::{
    is_transient_error, is_transient_reqwest_error, is_url_accessible, ClientConfig,
    EnhancedHttpClient, HttpClientFactory,
};
pub use interface::cli::{Cli, SourceType};
#[cfg(feature = "gui")]
pub use interface::gui::IaGetApp;
pub use utilities::common::{
    construct_download_url, construct_metadata_url, extract_identifier_from_url, get_user_agent,
    is_archive_url, normalize_archive_identifier, validate_and_process_url, AdaptiveBufferManager,
    PerformanceMetrics, PerformanceMonitor, StringTruncate,
};
pub use utilities::compression::*;
pub use utilities::filters::{
    filter_files, format_size, parse_size_string, FileFormats, FormatCategory,
};

// Legacy compatibility re-exports for external tests and examples
pub mod metadata {
    pub use crate::core::archive::*;
}

pub mod metadata_storage {
    pub use crate::core::session::*;
}

pub mod url_processing {
    pub use crate::utilities::common::*;
}

pub mod constants {
    pub use crate::utilities::common::*;
}

pub mod cli {
    pub use crate::interface::cli::*;
}

pub mod archive_metadata {
    pub use crate::core::archive::*;
}

pub mod filters {
    pub use crate::utilities::filters::*;
}

pub mod file_formats {
    pub use crate::utilities::filters::*;
}

pub mod progress {
    pub use crate::utilities::common::*;
}

pub mod concurrent_simple {
    pub use crate::core::download::*;
}

pub mod enhanced_downloader {
    pub use crate::core::download::*;
}

pub mod compression {
    pub use crate::utilities::compression::*;
}

/// Detect if GUI mode is available and appropriate
pub fn can_use_gui() -> bool {
    // Check if GUI features are compiled in
    #[cfg(not(feature = "gui"))]
    return false;

    #[cfg(feature = "gui")]
    {
        // Platform-specific GUI detection
        #[cfg(target_os = "windows")]
        {
            // On Windows, assume GUI is available unless we're in a Windows Terminal
            // that explicitly indicates headless mode
            std::env::var("WT_SESSION").is_ok() || std::env::var("SESSIONNAME").is_ok()
        }

        #[cfg(target_os = "macos")]
        {
            // On macOS, check for common GUI indicators
            // Most macOS environments have GUI available
            std::env::var("DISPLAY").is_ok()
                || std::env::var("TERM_PROGRAM").is_ok()
                || std::env::var("Apple_PubSub_Socket_Render").is_ok()
        }

        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            // On Linux and other Unix-like systems
            // If we're in SSH or explicit terminal contexts, prefer CLI
            if std::env::var("SSH_CONNECTION").is_ok()
                || std::env::var("SSH_CLIENT").is_ok()
                || std::env::var("SSH_TTY").is_ok()
            {
                return false;
            }

            // Check for X11 or Wayland display
            if std::env::var("DISPLAY").is_ok() || std::env::var("WAYLAND_DISPLAY").is_ok() {
                return true;
            }

            // Check for desktop environment variables
            if std::env::var("XDG_CURRENT_DESKTOP").is_ok()
                || std::env::var("DESKTOP_SESSION").is_ok()
                || std::env::var("GNOME_DESKTOP_SESSION_ID").is_ok()
                || std::env::var("KDE_FULL_SESSION").is_ok()
            {
                return true;
            }

            // Default to false for headless/server environments
            false
        }
    }
}
