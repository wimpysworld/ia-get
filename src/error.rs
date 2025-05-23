use thiserror::Error;
use std::path::PathBuf;

/// Custom error types for ia-get operations
#[derive(Error, Debug)]
pub enum IaGetError {
    /// Errors related to URL validation and access
    #[error("URL error: {0}")]
    UrlError(String),

    /// Invalid URL format
    #[error("Invalid URL format: {0}")]
    UrlFormatError(String),
    
    /// Errors when accessing Internet Archive
    #[error("Internet Archive error: {0}")]
    ArchiveError(String),
    
    /// File system errors
    #[error("File system error: {0}")]
    FileError(#[from] std::io::Error),
    
    /// Network request errors
    #[error("Network error: {0}")]
    NetworkError(#[from] reqwest::Error),
    
    /// XML parsing errors
    #[error("XML parsing error: {0}")]
    XmlParseError(#[from] serde_xml_rs::Error),

    /// URL parsing errors
    #[error("URL parsing error: {0}")]
    UrlParseError(#[from] url::ParseError),
    
    /// Hash verification errors
    #[error("Hash verification failed for {0}")]
    HashMismatchError(PathBuf),
    
    /// Hash missing in metadata
    #[error("No MD5 hash available for {0}")]
    MissingHashError(PathBuf),
    
    /// Regex compilation errors
    #[error("Regex error: {0}")]
    RegexError(#[from] regex::Error),
}

/// Result type that uses our custom IaGetError
pub type Result<T> = std::result::Result<T, IaGetError>;