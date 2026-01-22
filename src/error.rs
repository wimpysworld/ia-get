//! Error types for ia-get

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

    /// XML parsing errors
    #[error("Failed to parse XML: {0}")]
    XmlParsing(String),
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

impl From<serde_xml_rs::Error> for IaGetError {
    fn from(err: serde_xml_rs::Error) -> Self {
        IaGetError::XmlParsing(err.to_string())
    }
}
