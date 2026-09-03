//! Error types for ia-get

use thiserror::Error;

/// Result type alias for ia-get operations
pub type Result<T> = std::result::Result<T, IaGetError>;

/// Main error type for ia-get operations
#[derive(Error, Debug)]
pub enum IaGetError {
    /// Network-related errors including connection failures, timeouts, and HTTP errors
    #[error("Network error: {detail}")]
    Network {
        detail: String,
        /// The underlying error (e.g. the reqwest error), when one exists
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    /// File system errors during download operations
    #[error("File operation failed: {detail}")]
    FileSystem {
        detail: String,
        /// The underlying error (e.g. the std::io::Error), when one exists
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    /// URL format or parsing errors
    #[error(
        "Invalid archive.org URL: {0}. Expected https://archive.org/details/<identifier> or https://archive.org/download/<identifier>/<file>"
    )]
    UrlFormat(String),

    /// The URL named a file the archive's `_files.xml` metadata does not list
    #[error("File not found in archive: {path} — item {identifier} does not contain it")]
    FileNotFoundInArchive { identifier: String, path: String },

    /// No file survived selection for the download: the item lists none,
    /// the `--include`/`--exclude` filters matched nothing, or every
    /// candidate name encodes to an empty path
    #[error("No files selected for download in {identifier}")]
    NoFilesSelected { identifier: String },

    /// The `-o`/`--output-dir` argument names no directory
    #[error("Invalid output directory: {0} — the path must name a directory")]
    InvalidOutputDir(String),

    /// The `--limit-rate` value cannot be parsed as a throughput
    #[error("Invalid rate limit: {0}")]
    InvalidRate(String),

    /// The `--proxy` value (or the `HTTPS_PROXY` env var) cannot be turned
    /// into a proxy the HTTP client can use
    #[error("Invalid proxy: {0}")]
    InvalidProxy(String),

    /// The planned downloads need more space than the target volume has free;
    /// `required`/`available` are pre-formatted human sizes
    #[error(
        "Not enough disk space: need {required} for the download, only {available} free in {path}"
    )]
    InsufficientDiskSpace {
        required: String,
        available: String,
        path: String,
    },

    /// A `--check` run found the directory does not match the archive's
    /// metadata; the per-file findings were already printed, this only carries
    /// the failure count so the process exits non-zero
    #[error("Directory check found {problems} problem(s)")]
    CheckFailed { problems: usize },

    /// The directory given to `--check` does not exist
    #[error("Directory to check not found: {0}")]
    CheckDirectoryNotFound(String),

    /// XML parsing errors
    #[error("Failed to parse XML: {0}")]
    XmlParsing(String),

    /// The system clock failed to provide the current time (a time before
    /// the Unix epoch on platforms that allow it)
    #[error("Failed to get the system time: {0}")]
    SystemTime(#[source] std::time::SystemTimeError),

    /// The supplied cookie input cannot be turned into a valid HTTP header
    #[error("Invalid cookie header: {0}")]
    InvalidCookie(String),

    /// Server rejected the resume offset (HTTP 416): the local partial file is not a valid prefix
    #[error(
        "Server rejected the resume offset (HTTP 416); the partial file must be re-downloaded from scratch"
    )]
    RangeNotSatisfiable,

    /// The run was interrupted by the user (Ctrl+C)
    #[error("Interrupted by user")]
    Interrupted,

    /// One or more files in a batch could not be downloaded.
    ///
    /// The per-file reasons stay out of the display: the batch loop already
    /// printed them before constructing this error. `details` remains
    /// available for structured access.
    #[error("{count} of {total} file(s) failed to download")]
    BatchFailed {
        count: usize,
        total: usize,
        details: String,
    },
}

impl From<reqwest::Error> for IaGetError {
    fn from(err: reqwest::Error) -> Self {
        // Connection failures and HTTP errors get a more specific prefix;
        // the rest fall back to the reqwest message as is.
        let detail = if err.is_connect() || err.is_timeout() {
            format!("Connection failed: {err}")
        } else if let Some(status) = err.status() {
            format!("HTTP error {status}: {err}")
        } else {
            err.to_string()
        };
        IaGetError::Network {
            detail,
            source: Some(Box::new(err)),
        }
    }
}

impl From<std::io::Error> for IaGetError {
    fn from(err: std::io::Error) -> Self {
        // An OS-level Interrupted (a syscall interrupted by a signal) is a
        // transient I/O problem, not a user request to stop the run: map it
        // to FileSystem so the retry loop can treat it as retryable. The
        // user's Ctrl+C reaches the code through the running flag, never
        // through an io::Error.
        IaGetError::FileSystem {
            detail: err.to_string(),
            source: Some(Box::new(err)),
        }
    }
}

impl From<url::ParseError> for IaGetError {
    fn from(err: url::ParseError) -> Self {
        IaGetError::UrlFormat(err.to_string())
    }
}

impl From<serde_xml_rs::Error> for IaGetError {
    fn from(err: serde_xml_rs::Error) -> Self {
        IaGetError::XmlParsing(err.to_string())
    }
}

/// Converts an `io::Error` into an `IaGetError` naming the file the
/// operation touched. A bare message ("File operation failed: Access is
/// denied") cannot be located in a multi-hundred-file batch; the path can.
pub fn io_error_with_path(path: impl AsRef<std::path::Path>, err: std::io::Error) -> IaGetError {
    IaGetError::FileSystem {
        detail: format!("{}: {}", path.as_ref().display(), err),
        source: Some(Box::new(err)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::ErrorKind;
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn system_time_error_displays_its_own_kind() {
        // A clock before the Unix epoch is not a file system problem: it
        // must not wear the "File operation failed" label.
        let past = UNIX_EPOCH - Duration::from_secs(1);
        let err = IaGetError::SystemTime(
            past.duration_since(UNIX_EPOCH)
                .expect_err("a past time has no duration_since"),
        );
        assert!(err.to_string().contains("system time"), "got: {err}");
        assert!(!err.to_string().contains("File operation"));
    }

    #[test]
    fn os_interrupted_maps_to_filesystem_error() {
        // A syscall interrupted by a signal must not be mistaken for the
        // user's Ctrl+C.
        let io_err = std::io::Error::from(ErrorKind::Interrupted);
        let err = IaGetError::from(io_err);
        assert!(
            matches!(err, IaGetError::FileSystem { .. }),
            "an OS-level Interrupted must stay a file system error, got {err:?}"
        );
    }
}
