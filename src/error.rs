//! Comprehensive error handling for ia-get
//!
//! This module defines all error types used throughout the ia-get library,
//! providing clear error messages and context for debugging and user feedback.
//!
//! ## Error Categories
//!
//! - **Network**: Connection failures, timeouts, HTTP errors
//! - **FileSystem**: Local file operations, permission issues, disk space
//! - **UrlFormat**: Invalid or malformed Internet Archive URLs
//! - **Parse**: JSON/data parsing failures from API responses
//! - **Io**: Low-level I/O operations (wraps std::io::Error)
//! - **ReqwestError**: HTTP client errors (wraps reqwest::Error)
//!
//! ## Usage
//!
//! ```rust,no_run
//! use ia_get::{Result, IaGetError};
//!
//! fn download_file() -> Result<()> {
//!     // Operations that might fail
//!     let invalid_url = false; // example
//!     if invalid_url {
//!         return Err(IaGetError::UrlFormat("Invalid identifier".to_string()));
//!     }
//!     Ok(())
//! }
//! ```

use thiserror::Error;

/// Result type alias for ia-get operations
pub type Result<T> = std::result::Result<T, IaGetError>;

/// Main error type for ia-get operations
#[derive(Error, Debug)]
pub enum IaGetError {
    /// Network-related errors including connection failures, timeouts, and HTTP errors
    #[error("Network error: {0}")]
    Network(String),

    /// File system errors during download operations
    #[error("File operation failed: {0}")]
    FileSystem(String),

    /// URL format or parsing errors
    #[error("Invalid archive.org URL: {0}. Expected format: https://archive.org/details/<identifier>[/]")]
    UrlFormat(String),

    /// MD5 hash verification failures
    #[error("Hash verification failed: {0}")]
    HashMismatch(String),

    /// JSON parsing errors
    #[error("Failed to parse JSON: {0}")]
    JsonParsing(String),

    /// Configuration errors
    #[error("Configuration error: {0}")]
    Config(String),

    /// General parsing errors
    #[error("Parse error: {0}")]
    Parse(String),

    /// Missing files error
    #[error("No files found: {0}")]
    NoFilesFound(String),

    /// IO errors
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

impl From<reqwest::Error> for IaGetError {
    fn from(err: reqwest::Error) -> Self {
        if err.is_connect() || err.is_timeout() {
            IaGetError::Network(format!("Connection failed: {}", err))
        } else if err.is_status() {
            let status = err.status().map(|s| s.to_string()).unwrap_or_default();
            IaGetError::Network(format!("HTTP error {}: {}", status, err))
        } else {
            IaGetError::Network(err.to_string())
        }
    }
}

impl From<url::ParseError> for IaGetError {
    fn from(err: url::ParseError) -> Self {
        IaGetError::UrlFormat(err.to_string())
    }
}
