//! Error types for ia-get

use std::path::PathBuf;

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
    #[error("Invalid archive.org URL: {0}. Expected format: https://archive.org/details/<identifier>[/] or https://archive.org/download/<identifier>/<path>")]
    UrlFormat(String),

    /// Requested file was not found in the archive item metadata
    #[error("File '{file}' not found in archive item '{item}'")]
    FileNotFound {
        /// Requested file path within the archive item
        file: String,
        /// Archive.org item identifier
        item: String,
    },

    /// XML parsing errors
    #[error("Failed to parse XML: {0}")]
    XmlParsing(String),

    /// Integrity verification errors (such as MD5 hash mismatches)
    #[error("Integrity check failed: {0}")]
    Integrity(String),

    /// One or more files in a batch download failed
    #[error("{failed} of {total} file(s) failed to download")]
    BatchFailed {
        /// Number of files that failed to download
        failed: usize,
        /// Total number of files in the batch
        total: usize,
    },

    /// One or more files failed local verification
    #[error("{failed} of {total} file(s) failed verification")]
    VerificationFailed {
        /// Number of files that failed verification
        failed: usize,
        /// Total number of files checked
        total: usize,
    },

    /// User-supplied filter glob pattern is invalid
    #[error("Invalid filter pattern '{pattern}': {reason}")]
    InvalidFilter {
        /// Original pattern supplied by the user
        pattern: String,
        /// Reason the pattern could not be compiled
        reason: String,
    },

    /// Filter patterns matched no files in the archive item
    #[error("No files matched filter(s): {patterns}")]
    NoMatchingFiles {
        /// Comma-separated filter patterns supplied by the user
        patterns: String,
    },

    /// Destination volume does not have enough free disk space for the planned download
    #[error(
        "Insufficient disk space in '{}': need {}, only {} available",
        path.display(),
        crate::utils::format_size(*required),
        crate::utils::format_size(*available)
    )]
    InsufficientDiskSpace {
        /// Estimated remaining bytes required for the planned download
        required: u64,
        /// Available bytes reported on the destination volume
        available: u64,
        /// Destination directory path
        path: PathBuf,
    },

    /// User-supplied bandwidth rate limit is invalid
    #[error("Invalid rate limit '{input}': {reason}")]
    InvalidRateLimit {
        /// Original rate limit string supplied by the user
        input: String,
        /// Reason the rate limit could not be parsed
        reason: String,
    },

    /// Proxy environment configuration is invalid
    #[error("Invalid proxy configuration in {var} ('{value}'): {reason}")]
    InvalidProxy {
        /// Environment variable name holding the invalid proxy configuration
        var: String,
        /// Redacted proxy value from the environment variable
        value: String,
        /// Reason the proxy configuration is invalid
        reason: String,
    },

    /// Two or more planned entries map to the same local destination or staging path
    #[error(
        "Path collision for '{local_path}': '{first_entry}' conflicts with '{second_entry}' ({reason})"
    )]
    PathCollision {
        /// Local destination or staging path where the collision occurs
        local_path: String,
        /// First archive entry or path claiming the local path
        first_entry: String,
        /// Second archive entry or path colliding with it
        second_entry: String,
        /// Description of whether the conflict is final-vs-final or staging-vs-final
        reason: String,
    },

    /// All selected files are marked private in archive.org metadata and no cookies were supplied
    #[error(
        "All {count} selected file(s) are marked private in archive.org metadata and require authentication with -b/--cookies"
    )]
    PrivateFilesOnly {
        /// Number of selected files marked private in archive.org metadata
        count: usize,
    },

    /// Download was interrupted by the user
    #[error("Download interrupted by user")]
    Interrupted,
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

impl From<std::io::Error> for IaGetError {
    fn from(err: std::io::Error) -> Self {
        IaGetError::FileSystem(err.to_string())
    }
}

impl From<url::ParseError> for IaGetError {
    fn from(err: url::ParseError) -> Self {
        IaGetError::UrlFormat(err.to_string())
    }
}
