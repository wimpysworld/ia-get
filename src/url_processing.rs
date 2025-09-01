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

    // Handle both /details/ and /metadata/ paths
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
    } else {
        Err(IaGetError::UrlFormat(
            "URL must contain /details/ or /metadata/ path".to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_and_process_url_valid_archive_url() {
        let url = "https://archive.org/details/example";
        let result = validate_and_process_url(url).unwrap();
        assert_eq!(result, "https://archive.org/details/example");
    }

    #[test]
    fn test_validate_and_process_url_identifier() {
        let identifier = "example";
        let result = validate_and_process_url(identifier).unwrap();
        assert_eq!(result, "https://archive.org/details/example");
    }

    #[test]
    fn test_validate_and_process_url_invalid_url() {
        let url = "https://example.com/test";
        let result = validate_and_process_url(url);
        assert!(result.is_err());
    }

    #[test]
    fn test_is_archive_url() {
        assert!(is_archive_url("https://archive.org/details/example"));
        assert!(is_archive_url("https://web.archive.org/details/example"));
        assert!(!is_archive_url("https://example.com"));
        assert!(!is_archive_url("invalid"));
    }

    #[test]
    fn test_extract_identifier_from_url() {
        let url = "https://archive.org/details/example";
        let result = extract_identifier_from_url(url).unwrap();
        assert_eq!(result, "example");
    }

    #[test]
    fn test_extract_identifier_from_url_invalid() {
        let url = "https://example.com/test";
        let result = extract_identifier_from_url(url);
        assert!(result.is_err());
    }

    #[test]
    fn test_construct_metadata_url() {
        let identifier = "example";
        let result = construct_metadata_url(identifier);
        assert_eq!(result, "https://archive.org/metadata/example");
    }

    #[test]
    fn test_construct_download_url() {
        let identifier = "example";
        let result = construct_download_url(identifier);
        assert_eq!(result, "https://archive.org/download/example");
    }
}
