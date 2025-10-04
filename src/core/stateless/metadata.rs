//! Stateless metadata fetching
//!
//! Pure functions for fetching Internet Archive metadata without any state management.
//! All functions are designed to be called from FFI with simple input/output.

use crate::{
    core::session::ArchiveMetadata, error::IaGetError, infrastructure::http::is_transient_error,
    Result,
};
use reqwest::blocking::Client;
use std::time::Duration;

/// Fetch metadata synchronously (blocking)
///
/// This is a pure function with no state. Perfect for FFI integration.
///
/// # Arguments
///
/// * `identifier` - Archive.org identifier (e.g., "commute_test")
///
/// # Returns
///
/// * `Ok(ArchiveMetadata)` - Successfully fetched metadata
/// * `Err(IaGetError)` - Network or parsing error
///
/// # Example
///
/// ```rust,no_run
/// use ia_get::core::stateless::metadata::fetch_metadata_sync;
///
/// let metadata = fetch_metadata_sync("commute_test")?;
/// println!("Files: {}", metadata.files.len());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn fetch_metadata_sync(identifier: &str) -> Result<ArchiveMetadata> {
    // Create client with reasonable timeouts
    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| IaGetError::Network(format!("Failed to create HTTP client: {}", e)))?;

    // Build metadata URL
    let url = format!("https://archive.org/metadata/{}", identifier);

    // Fetch with retry logic (up to 3 attempts)
    let max_retries = 3;
    let mut last_error = None;

    for attempt in 1..=max_retries {
        match client.get(&url).header("Accept", "application/json").send() {
            Ok(response) => {
                if response.status().is_success() {
                    let text = response.text().map_err(|e| {
                        IaGetError::Network(format!("Failed to read response: {}", e))
                    })?;

                    match serde_json::from_str::<ArchiveMetadata>(&text) {
                        Ok(metadata) => return Ok(metadata),
                        Err(e) => {
                            return Err(IaGetError::Parse(format!(
                                "Failed to parse metadata JSON: {}",
                                e
                            )));
                        }
                    }
                } else {
                    let status = response.status();
                    return Err(IaGetError::Network(format!(
                        "HTTP error {}: {}",
                        status.as_u16(),
                        status.canonical_reason().unwrap_or("Unknown")
                    )));
                }
            }
            Err(e) => {
                last_error = Some(e);

                // Only retry on transient errors
                if attempt < max_retries && is_transient_error(last_error.as_ref().unwrap()) {
                    std::thread::sleep(Duration::from_millis(1000 * attempt as u64));
                    continue;
                }

                break;
            }
        }
    }

    // All retries failed
    Err(IaGetError::Network(format!(
        "Failed to fetch metadata after {} attempts: {}",
        max_retries,
        last_error
            .map(|e| e.to_string())
            .unwrap_or_else(|| "Unknown error".to_string())
    )))
}

/// Fetch metadata as JSON string (for easy FFI transfer)
///
/// This version returns the raw JSON string, which is easier to pass
/// across FFI boundaries.
///
/// # Arguments
///
/// * `identifier` - Archive.org identifier
///
/// # Returns
///
/// * `Ok(String)` - JSON string containing metadata
/// * `Err(IaGetError)` - Network or parsing error
///
/// # Example
///
/// ```rust,no_run
/// use ia_get::core::stateless::metadata::fetch_metadata_json;
///
/// let json = fetch_metadata_json("commute_test")?;
/// println!("Metadata JSON: {}", json);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn fetch_metadata_json(identifier: &str) -> Result<String> {
    let metadata = fetch_metadata_sync(identifier)?;

    serde_json::to_string(&metadata)
        .map_err(|e| IaGetError::Parse(format!("Failed to serialize metadata to JSON: {}", e)))
}

/// Async version for CLI use
///
/// This version uses async/await for better performance in the CLI tool.
///
/// # Arguments
///
/// * `identifier` - Archive.org identifier
///
/// # Returns
///
/// * `Ok(ArchiveMetadata)` - Successfully fetched metadata
/// * `Err(IaGetError)` - Network or parsing error
pub async fn fetch_metadata_async(identifier: &str) -> Result<ArchiveMetadata> {
    // Create async client
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| IaGetError::Network(format!("Failed to create HTTP client: {}", e)))?;

    // Build metadata URL
    let url = format!("https://archive.org/metadata/{}", identifier);

    // Fetch with retry logic
    let max_retries = 3;
    let mut last_error = None;

    for attempt in 1..=max_retries {
        match client
            .get(&url)
            .header("Accept", "application/json")
            .send()
            .await
        {
            Ok(response) => {
                if response.status().is_success() {
                    match response.json::<ArchiveMetadata>().await {
                        Ok(metadata) => return Ok(metadata),
                        Err(e) => {
                            return Err(IaGetError::Parse(format!(
                                "Failed to parse metadata JSON: {}",
                                e
                            )));
                        }
                    }
                } else {
                    let status = response.status();
                    return Err(IaGetError::Network(format!(
                        "HTTP error {}: {}",
                        status.as_u16(),
                        status.canonical_reason().unwrap_or("Unknown")
                    )));
                }
            }
            Err(e) => {
                last_error = Some(e);

                if attempt < max_retries && is_transient_error(last_error.as_ref().unwrap()) {
                    tokio::time::sleep(Duration::from_millis(1000 * attempt as u64)).await;
                    continue;
                }

                break;
            }
        }
    }

    Err(IaGetError::Network(format!(
        "Failed to fetch metadata after {} attempts: {}",
        max_retries,
        last_error
            .map(|e| e.to_string())
            .unwrap_or_else(|| "Unknown error".to_string())
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fetch_metadata_sync() {
        // This is an integration test that requires network access
        // Skip in CI environments without network
        if std::env::var("CI").is_ok() {
            return;
        }

        let result = fetch_metadata_sync("commute_test");
        assert!(
            result.is_ok(),
            "Failed to fetch metadata: {:?}",
            result.err()
        );

        let metadata = result.unwrap();
        assert!(!metadata.files.is_empty(), "No files in metadata");
    }

    #[test]
    fn test_fetch_metadata_json() {
        if std::env::var("CI").is_ok() {
            return;
        }

        let result = fetch_metadata_json("commute_test");
        assert!(
            result.is_ok(),
            "Failed to fetch metadata JSON: {:?}",
            result.err()
        );

        let json = result.unwrap();
        assert!(json.contains("files"), "JSON doesn't contain files field");
        assert!(
            json.contains("metadata"),
            "JSON doesn't contain metadata field"
        );
    }

    #[tokio::test]
    async fn test_fetch_metadata_async() {
        if std::env::var("CI").is_ok() {
            return;
        }

        let result = fetch_metadata_async("commute_test").await;
        assert!(
            result.is_ok(),
            "Failed to fetch metadata: {:?}",
            result.err()
        );

        let metadata = result.unwrap();
        assert!(!metadata.files.is_empty(), "No files in metadata");
    }
}
