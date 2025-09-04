//! Internet Archive metadata processing
//!
//! This module provides comprehensive metadata fetching and parsing functionality
//! for the Internet Archive's JSON API. It handles retries, error recovery, and
//! provides a clean interface for accessing archive file information.
//!
//! ## API Reference
//!
//! Internet Archive Metadata API: <https://archive.org/developers/md-read.html>
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
    core::session::ArchiveMetadata, error::IaGetError, infrastructure::http::is_transient_error,
    Result,
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
    if let Err(e) =
        crate::infrastructure::http::is_url_accessible(&json_url, client, Some(progress)).await
    {
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
            // Debug JSON parsing failures (disabled in tests to reduce noise)
            // eprintln!("JSON parsing failed: {}", e);
            Err(IaGetError::JsonParsing(format!(
                "Failed to parse JSON metadata: {}",
                e
            )))
        }
    }
}

/// Enhanced metadata functionality using the Internet Archive APIs
pub mod enhanced {
    use super::*;
    use crate::infrastructure::api::EnhancedArchiveApiClient;
    use indicatif::ProgressBar;
    use reqwest::Client;
    use serde_json::Value;

    /// Enhanced metadata structure with additional API information
    #[derive(Debug, serde::Deserialize, serde::Serialize)]
    pub struct EnhancedMetadata {
        pub basic_metadata: ArchiveMetadata,
        pub search_results: Option<Value>,
        pub related_items: Option<Value>,
        pub tasks_status: Option<Value>,
        pub collection_info: Option<Value>,
    }

    /// Fetch comprehensive metadata using enhanced API client
    ///
    /// This function fetches not just the basic metadata but also:
    /// - Related items in the same collection
    /// - Task status information
    /// - Search context for the item
    pub async fn fetch_enhanced_metadata(
        identifier: &str,
        client: &Client,
        progress: &ProgressBar,
        include_related: bool,
        include_tasks: bool,
    ) -> Result<EnhancedMetadata> {
        progress.set_message(format!("🔍 Fetching enhanced metadata for {}", identifier));

        // Create enhanced API client
        let mut api_client = EnhancedArchiveApiClient::new(client.clone());

        // Fetch basic metadata using existing function
        let (basic_metadata, _url) = fetch_json_metadata(identifier, client, progress).await?;

        let mut enhanced = EnhancedMetadata {
            basic_metadata,
            search_results: None,
            related_items: None,
            tasks_status: None,
            collection_info: None,
        };

        // Fetch related items if requested
        if include_related {
            progress.set_message(format!("🔗 Finding related items for {}", identifier));
            match api_client.find_related_items(identifier, Some(10)).await {
                Ok(response) => {
                    if let Ok(text) = response.text().await {
                        if let Ok(json) = serde_json::from_str::<Value>(&text) {
                            enhanced.related_items = Some(json);
                        }
                    }
                }
                Err(e) => {
                    // Log error but don't fail the entire operation
                    progress.set_message(format!("⚠️ Could not fetch related items: {}", e));
                }
            }
        }

        // Fetch task status if requested
        if include_tasks {
            progress.set_message(format!("⚙️ Checking task status for {}", identifier));
            match api_client.get_tasks(identifier).await {
                Ok(response) => {
                    if let Ok(text) = response.text().await {
                        if let Ok(json) = serde_json::from_str::<Value>(&text) {
                            enhanced.tasks_status = Some(json);
                        }
                    }
                }
                Err(e) => {
                    // Log error but don't fail the entire operation
                    progress.set_message(format!("⚠️ Could not fetch task status: {}", e));
                }
            }
        }

        // Try to get collection information from metadata
        if let Some(collections) = enhanced.basic_metadata.metadata.get("collection") {
            progress.set_message(format!("📁 Fetching collection info for {}", identifier));

            // If there are collections, try to get info about the first one
            if let Some(collection_array) = collections.as_array() {
                if let Some(first_collection) = collection_array.first() {
                    if let Some(collection_id) = first_collection.as_str() {
                        match api_client
                            .search_collection(
                                collection_id,
                                Some("identifier,title,description,item_count"),
                                Some(5),
                            )
                            .await
                        {
                            Ok(response) => {
                                if let Ok(text) = response.text().await {
                                    if let Ok(json) = serde_json::from_str::<Value>(&text) {
                                        enhanced.collection_info = Some(json);
                                    }
                                }
                            }
                            Err(e) => {
                                progress.set_message(format!(
                                    "⚠️ Could not fetch collection info: {}",
                                    e
                                ));
                            }
                        }
                    }
                }
            }
        }

        progress.set_message(format!("✅ Enhanced metadata retrieved for {}", identifier));
        Ok(enhanced)
    }

    /// Analyze metadata to provide insights about the archive item
    pub fn analyze_metadata(metadata: &EnhancedMetadata) -> MetadataAnalysis {
        let basic = &metadata.basic_metadata;

        let file_count = basic.files.len();
        let total_size: u64 = basic.files.iter().filter_map(|f| f.size).sum();

        // Analyze file types
        let mut file_types = std::collections::HashMap::new();
        for file in &basic.files {
            if let Some(extension) = std::path::Path::new(&file.name).extension() {
                let ext = extension.to_string_lossy().to_lowercase();
                *file_types.entry(ext).or_insert(0) += 1;
            }
        }

        // Analyze collections
        let collections: Vec<String> = metadata
            .basic_metadata
            .metadata
            .get("collection")
            .and_then(|c| c.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        MetadataAnalysis {
            identifier: basic
                .metadata
                .get("identifier")
                .and_then(|i| i.as_str())
                .unwrap_or("unknown")
                .to_string(),
            file_count,
            total_size,
            file_types,
            collections,
            has_related_items: metadata.related_items.is_some(),
            has_tasks: metadata.tasks_status.is_some(),
            creation_date: basic
                .metadata
                .get("date")
                .and_then(|d| d.as_str())
                .map(String::from),
        }
    }

    /// Metadata analysis results
    #[derive(Debug)]
    pub struct MetadataAnalysis {
        pub identifier: String,
        pub file_count: usize,
        pub total_size: u64,
        pub file_types: std::collections::HashMap<String, usize>,
        pub collections: Vec<String>,
        pub has_related_items: bool,
        pub has_tasks: bool,
        pub creation_date: Option<String>,
    }

    impl std::fmt::Display for MetadataAnalysis {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            use crate::utilities::filters::format_size;

            writeln!(f, "📊 Metadata Analysis for '{}'", self.identifier)?;
            writeln!(f, "   Files: {}", self.file_count)?;
            writeln!(f, "   Total Size: {}", format_size(self.total_size))?;

            if !self.file_types.is_empty() {
                writeln!(f, "   File Types:")?;
                let mut types: Vec<_> = self.file_types.iter().collect();
                types.sort_by(|a, b| b.1.cmp(a.1)); // Sort by count, descending
                for (ext, count) in types.iter().take(5) {
                    writeln!(f, "     {}: {}", ext, count)?;
                }
            }

            if !self.collections.is_empty() {
                writeln!(f, "   Collections: {}", self.collections.join(", "))?;
            }

            if let Some(date) = &self.creation_date {
                writeln!(f, "   Created: {}", date)?;
            }

            if self.has_related_items {
                writeln!(f, "   ✓ Related items available")?;
            }

            if self.has_tasks {
                writeln!(f, "   ✓ Task status available")?;
            }

            Ok(())
        }
    }
}
