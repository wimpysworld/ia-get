//! URL processing and validation module for ia-get
//!
//! Handles Internet Archive URL parsing, validation, and format conversions.

use crate::{error::IaGetError, Result};
use url::Url;

/// Validates and processes Internet Archive URLs
pub fn validate_and_process_url(url_input: &str) -> Result<String> {
    // Check if it's a valid URL
    match Url::parse(url_input) {
        Ok(url) => {
            // Ensure it's an archive.org URL
            if let Some(host) = url.host_str() {
                if host.ends_with("archive.org") {
                    return Ok(url.to_string());
                }
            }
            Err(IaGetError::UrlFormat(
                "URL must be from archive.org".to_string(),
            ))
        }
        Err(_) => {
            // If it's not a valid URL, try to construct it as an identifier
            if url_input.contains('.') || url_input.contains('/') {
                Err(IaGetError::UrlFormat("Input appears to be a partial URL. Please provide a full archive.org URL or a simple identifier.".to_string()))
            } else {
                // Construct the full URL
                Ok(format!("https://archive.org/details/{}", url_input))
            }
        }
    }
}

/// Checks if a string is a complete archive.org URL
pub fn is_archive_url(input: &str) -> bool {
    if let Ok(url) = Url::parse(input) {
        if let Some(host) = url.host_str() {
            return host.ends_with("archive.org");
        }
    }
    false
}

/// Extracts identifier from archive.org URL
pub fn extract_identifier_from_url(url: &str) -> Result<String> {
    let parsed_url =
        Url::parse(url).map_err(|_| IaGetError::UrlFormat("Invalid URL format".to_string()))?;

    if let Some(host) = parsed_url.host_str() {
        if !host.ends_with("archive.org") {
            return Err(IaGetError::UrlFormat(
                "URL must be from archive.org".to_string(),
            ));
        }
    } else {
        return Err(IaGetError::UrlFormat("Invalid URL format".to_string()));
    }

    let path = parsed_url.path();

    // Handle /details/, /metadata/, and /download/ paths
    if let Some(identifier) = path.strip_prefix("/details/") {
        if identifier.is_empty() {
            return Err(IaGetError::UrlFormat(
                "No identifier found in URL".to_string(),
            ));
        }
        Ok(identifier.to_string())
    } else if let Some(identifier) = path.strip_prefix("/metadata/") {
        if identifier.is_empty() {
            return Err(IaGetError::UrlFormat(
                "No identifier found in URL".to_string(),
            ));
        }
        Ok(identifier.to_string())
    } else if let Some(path_after_download) = path.strip_prefix("/download/") {
        if path_after_download.is_empty() {
            return Err(IaGetError::UrlFormat(
                "No identifier found in URL".to_string(),
            ));
        }
        // For download URLs, extract just the identifier (first part before any slash)
        let identifier = path_after_download
            .split('/')
            .next()
            .unwrap_or(path_after_download);
        if identifier.is_empty() {
            return Err(IaGetError::UrlFormat(
                "No identifier found in URL".to_string(),
            ));
        }
        Ok(identifier.to_string())
    } else {
        Err(IaGetError::UrlFormat(
            "URL must contain /details/, /metadata/, or /download/ path".to_string(),
        ))
    }
}

/// Constructs metadata URL from identifier
pub fn construct_metadata_url(identifier: &str) -> String {
    format!("https://archive.org/metadata/{}", identifier)
}

/// Constructs download URL from identifier
pub fn construct_download_url(identifier: &str) -> String {
    format!("https://archive.org/download/{}", identifier)
}

/// Normalizes archive input to just the identifier portion
///
/// Accepts either a full archive.org URL or just an identifier,
/// and returns just the identifier portion for use in local paths and filenames.
///
/// # Examples
///
/// ```rust
/// use ia_get::url_processing::normalize_archive_identifier;
///
/// assert_eq!(normalize_archive_identifier("mario").unwrap(), "mario");
/// assert_eq!(normalize_archive_identifier("https://archive.org/details/mario").unwrap(), "mario");
/// assert_eq!(normalize_archive_identifier("https://archive.org/download/luigi").unwrap(), "luigi");
/// ```
pub fn normalize_archive_identifier(input: &str) -> Result<String> {
    if is_archive_url(input) {
        extract_identifier_from_url(input)
    } else {
        Ok(input.to_string())
    }
}
