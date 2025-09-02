//! Internet Archive metadata processing
//!
//! This module provides comprehensive metadata fetching and parsing functionality
//! for the Internet Archive's JSON API. It handles retries, error recovery, and
//! provides a clean interface for accessing archive file information.
//!
//! ## API Reference
//!
//! Internet Archive Metadata API: https://archive.org/developers/md-read.html
//!
//! ## Usage
//!
//! ```rust,no_run
//! use ia_get::metadata::fetch_json_metadata;
//! use reqwest::Client;
//! use indicatif::ProgressBar;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let client = Client::new();
//!     let progress = ProgressBar::new_spinner();
//!     
//!     // Fetch metadata for an archive
//!     let (metadata, _url) = fetch_json_metadata("internetarchive", &client, &progress).await?;
//!     println!("Found {} files", metadata.files.len());
//!
//!     // List all file names
//!     for file in &metadata.files {
//!         println!("File: {}", file.name);
//!     }
//!     Ok(())
//! }
//! ```
//!
//! ## Features
//!
//! - **Automatic Retries**: Robust retry logic for transient network errors
//! - **Progress Indicators**: Visual feedback during metadata fetching
//! - **JSON-Only**: Uses the modern JSON API (no legacy XML support)
//! - **Error Context**: Detailed error messages with context

use crate::{
    error::IaGetError, metadata_storage::ArchiveMetadata, network::is_transient_error, Result,
};
use colored::*;
use indicatif::ProgressBar;
use reqwest::Client;

/// Converts an archive.org details URL to the corresponding JSON metadata URL
///
/// The Internet Archive provides metadata in JSON format which is preferred over XML
/// due to better performance, support for partial reads, and cleaner data structures.
///
/// ## Supported URL Formats
///
/// - Details URL: `https://archive.org/details/identifier` → `https://archive.org/metadata/identifier`
/// - Already metadata URL: `https://archive.org/metadata/identifier` → unchanged
/// - Bare identifier: `identifier` → `https://archive.org/metadata/identifier`
///
/// ## Examples
///
/// ```rust
/// use ia_get::metadata::get_json_url;
///
/// // Convert details URL
/// let url = get_json_url("https://archive.org/details/internetarchive");
/// assert_eq!(url, "https://archive.org/metadata/internetarchive");
///
/// // Handle bare identifier
/// let url = get_json_url("internetarchive");
/// assert_eq!(url, "https://archive.org/metadata/internetarchive");
/// ```
///
/// ## Arguments
///
/// * `original_url` - The input URL or identifier to convert
///
/// ## Returns
///
/// A properly formatted JSON metadata URL
pub fn get_json_url(original_url: &str) -> String {
    if original_url.contains("/details/") {
        original_url.replace("/details/", "/metadata/")
    } else if original_url.contains("://archive.org/metadata/") {
        // Already a metadata URL, use as-is
        original_url.to_string()
    } else {
        // Fallback: extract identifier and construct JSON URL
        let identifier = original_url.rsplit('/').next().unwrap_or(original_url);
        format!("https://archive.org/metadata/{}", identifier)
    }
}

/// Fetches and parses JSON metadata with retry logic for transient errors
///
/// This function handles the complete flow of fetching metadata from the Internet Archive:
/// 1. Validates URL accessibility
/// 2. Fetches JSON content with retry logic for transient errors
/// 3. Parses the response into a structured ArchiveMetadata object
///
/// # Arguments
/// * `details_url` - The archive.org details URL or identifier
/// * `client` - HTTP client for making requests
/// * `progress` - Progress bar for user feedback
///
/// # Returns
/// * `Ok((ArchiveMetadata, reqwest::Url))` - Parsed metadata and the actual URL used
/// * `Err(IaGetError)` - Various error conditions (network, parsing, not found, etc.)
pub async fn fetch_json_metadata(
    details_url: &str,
    client: &Client,
    progress: &ProgressBar,
) -> Result<(ArchiveMetadata, reqwest::Url)> {
    // Generate JSON metadata URL
    let json_url = get_json_url(details_url);
    progress.set_message(format!(
        "{} Accessing JSON metadata: {}",
        "⚙".blue(),
        json_url.bold()
    ));

    // Check JSON URL accessibility
    if let Err(e) = crate::network::is_url_accessible(&json_url, client, Some(progress)).await {
        progress.finish_with_message(format!(
            "{} JSON metadata not accessible: {}",
            "✘".red().bold(),
            json_url.bold()
        ));
        return Err(e);
    }

    progress.set_message(format!(
        "{} {}",
        "⚙".blue(),
        "Parsing archive metadata...".bold()
    ));

    // Parse base URL and fetch JSON content with retry logic
    let base_url = reqwest::Url::parse(&json_url)
        .map_err(|e| IaGetError::Network(format!("URL parse failed: {}", e)))?;
    let mut retries = 0;
    let max_retries = 3;
    let mut delay = std::time::Duration::from_secs(30); // Conservative initial delay for metadata
    let max_delay = std::time::Duration::from_secs(600); // 10 minutes max

    let json_content = loop {
        let result = client
            .get(&json_url)
            .send()
            .await
            .map_err(|e| IaGetError::Network(format!("GET request failed: {}", e)));
        match result {
            Ok(response) => {
                // Check for HTTP 429 and Retry-After header
                if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                    let wait_time = response
                        .headers()
                        .get(reqwest::header::RETRY_AFTER)
                        .and_then(|h| h.to_str().ok())
                        .and_then(|s| s.parse::<u64>().ok())
                        .unwrap_or(60);

                    let wait_reason = format!(
                        "Rate limited during JSON fetch (HTTP 429) - waiting {}s as requested",
                        wait_time
                    );
                    progress.set_message(format!("{} {}", "⏳".yellow(), wait_reason));

                    tokio::time::sleep(std::time::Duration::from_secs(wait_time)).await;
                    continue;
                }

                if !response.status().is_success() {
                    let status = response.status();
                    let body_text = response
                        .text()
                        .await
                        .unwrap_or_else(|_| "Unable to read response body".to_string());
                    return Err(IaGetError::Network(format!(
                        "HTTP {}: {}",
                        status, body_text
                    )));
                }

                match response.text().await {
                    Ok(text) => break text,
                    Err(e) => {
                        if is_transient_error(&e) && retries < max_retries {
                            retries += 1;
                            let wait_reason = format!(
                                "Response read failed (attempt {}/{}): {}",
                                retries,
                                max_retries + 1,
                                e
                            );
                            progress.set_message(format!(
                                "{} {} - retrying in {}s",
                                "⏳".yellow(),
                                wait_reason,
                                delay.as_secs()
                            ));
                            tokio::time::sleep(delay).await;
                            delay = std::cmp::min(delay * 2, max_delay);
                            continue;
                        } else {
                            return Err(IaGetError::Network(format!(
                                "Response text failed after {} retries: {}",
                                retries, e
                            )));
                        }
                    }
                }
            }
            Err(e) => {
                // For IaGetError, check if it's a network error that might be transient
                let is_transient = match &e {
                    IaGetError::Network(msg) => {
                        msg.contains("timeout") || msg.contains("connection")
                    }
                    _ => false,
                };

                if is_transient && retries < max_retries {
                    retries += 1;
                    let wait_reason = format!(
                        "Request failed (attempt {}/{}): {}",
                        retries,
                        max_retries + 1,
                        e
                    );
                    progress.set_message(format!(
                        "{} {} - retrying in {}s",
                        "⏳".yellow(),
                        wait_reason,
                        delay.as_secs()
                    ));
                    tokio::time::sleep(delay).await;
                    delay = std::cmp::min(delay * 2, max_delay);
                    continue;
                } else {
                    return Err(e);
                }
            }
        }
    };

    // Parse the JSON response
    let metadata = parse_archive_metadata(&json_content)?;

    progress.set_message(format!(
        "{} Successfully parsed metadata: {} files",
        "✓".green(),
        metadata.files.len()
    ));

    Ok((metadata, base_url))
}

/// Parses archive metadata from JSON content
///
/// Converts the raw JSON response from Internet Archive into a structured ArchiveMetadata object.
/// This function handles the complex JSON structure and provides helpful error messages for debugging.
///
/// # Arguments
/// * `json_content` - Raw JSON string from the metadata API
///
/// # Returns
/// * `Ok(ArchiveMetadata)` - Successfully parsed metadata
/// * `Err(IaGetError)` - Parsing failed with context for debugging
pub fn parse_archive_metadata(json_content: &str) -> Result<ArchiveMetadata> {
    match serde_json::from_str::<ArchiveMetadata>(json_content) {
        Ok(metadata) => {
            if metadata.files.is_empty() {
                eprintln!("Warning: Parsed JSON metadata but found no files");
            }
            Ok(metadata)
        }
        Err(e) => {
            // Provide helpful debugging information
            const DEBUG_TRUNCATE_LEN: usize = 200;
            let preview = if json_content.len() > DEBUG_TRUNCATE_LEN {
                &json_content[..DEBUG_TRUNCATE_LEN]
            } else {
                json_content
            };

            eprintln!(
                "JSON parsing failed.\nError: {}\nContent preview: {}{}",
                e,
                preview,
                if json_content.len() > DEBUG_TRUNCATE_LEN {
                    "..."
                } else {
                    ""
                }
            );
            Err(IaGetError::JsonParsing(format!(
                "Failed to parse JSON metadata: {}",
                e
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_json_url_from_details() {
        let details_url = "https://archive.org/details/example-item";
        let expected = "https://archive.org/metadata/example-item";
        assert_eq!(get_json_url(details_url), expected);
    }

    #[test]
    fn test_get_json_url_from_metadata_url() {
        let metadata_url = "https://archive.org/metadata/example-item";
        assert_eq!(get_json_url(metadata_url), metadata_url);
    }

    #[test]
    fn test_get_json_url_from_identifier() {
        let identifier = "example-item";
        let expected = "https://archive.org/metadata/example-item";
        assert_eq!(get_json_url(identifier), expected);
    }
}
