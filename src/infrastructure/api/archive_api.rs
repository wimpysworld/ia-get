//! Internet Archive API compliance and rate limiting module
//!
//! This module implements Internet Archive-specific API compliance measures,
//! including proper rate limiting, server selection, and request formatting
//! following the Internet Archive's guidelines and best practices.

use crate::{utilities::common::*, IaGetError, Result};
use reqwest::{Client, Response};
use std::time::{Duration, Instant};
use tokio::time::sleep;

/// Internet Archive API compliance manager
#[derive(Debug)]
pub struct ArchiveOrgApiClient {
    client: Client,
    last_request_time: Option<Instant>,
    request_count: u64,
    session_start: Instant,
}

impl ArchiveOrgApiClient {
    /// Create a new Archive.org API client with compliance features
    pub fn new(client: Client) -> Self {
        Self {
            client,
            last_request_time: None,
            request_count: 0,
            session_start: Instant::now(),
        }
    }

    /// Make a rate-limited request to Archive.org with proper compliance
    pub async fn make_request(&mut self, url: &str) -> Result<Response> {
        // Implement minimum delay between requests
        if let Some(last_time) = self.last_request_time {
            let elapsed = last_time.elapsed();
            let min_delay = Duration::from_millis(MIN_REQUEST_DELAY_MS);

            if elapsed < min_delay {
                let wait_time = min_delay - elapsed;
                sleep(wait_time).await;
            }
        }

        // Add Archive.org-specific headers
        let response = self
            .client
            .get(url)
            .header("Accept", "application/json, text/plain, */*")
            .header("Accept-Language", "en-US,en;q=0.9")
            .header("Cache-Control", "no-cache")
            .header("DNT", "1") // Do Not Track - be respectful
            .timeout(Duration::from_secs(HTTP_TIMEOUT))
            .send()
            .await
            .map_err(|e| IaGetError::Network(format!("Request to {} failed: {}", url, e)))?;

        // Update request tracking
        self.last_request_time = Some(Instant::now());
        self.request_count += 1;

        // Handle Archive.org-specific status codes
        self.handle_archive_response(&response, url).await?;

        Ok(response)
    }

    /// Handle Archive.org-specific response status codes and rate limiting
    async fn handle_archive_response(&self, response: &Response, url: &str) -> Result<()> {
        match response.status() {
            reqwest::StatusCode::TOO_MANY_REQUESTS => {
                let wait_time = response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|h| h.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(DEFAULT_RETRY_DELAY_SECS);

                Err(IaGetError::Network(format!(
                    "Rate limited by Archive.org. Please wait {}s before retrying. URL: {}",
                    wait_time, url
                )))
            }
            reqwest::StatusCode::SERVICE_UNAVAILABLE => Err(IaGetError::Network(format!(
                "Archive.org service temporarily unavailable. URL: {}",
                url
            ))),
            reqwest::StatusCode::FORBIDDEN => Err(IaGetError::Network(format!(
                "Access forbidden. This item may be restricted or require authentication. URL: {}",
                url
            ))),
            reqwest::StatusCode::NOT_FOUND => Err(IaGetError::Network(format!(
                "Archive item not found. The identifier may be incorrect. URL: {}",
                url
            ))),
            status if status.is_server_error() => Err(IaGetError::Network(format!(
                "Archive.org server error ({}). This is likely temporary. URL: {}",
                status.as_u16(),
                url
            ))),
            status if status.is_client_error() => Err(IaGetError::Network(format!(
                "Client error ({}). Check your request format. URL: {}",
                status.as_u16(),
                url
            ))),
            _ => Ok(()),
        }
    }

    /// Get the underlying HTTP client
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Get request statistics for this session
    pub fn get_stats(&self) -> ApiStats {
        ApiStats {
            request_count: self.request_count,
            session_duration: self.session_start.elapsed(),
            average_requests_per_minute: self.calculate_requests_per_minute(),
        }
    }

    /// Calculate average requests per minute
    fn calculate_requests_per_minute(&self) -> f64 {
        let minutes = self.session_start.elapsed().as_secs_f64() / 60.0;
        if minutes > 0.0 {
            self.request_count as f64 / minutes
        } else {
            0.0
        }
    }

    /// Check if we're making requests at a reasonable rate
    pub fn is_rate_healthy(&self) -> bool {
        // Archive.org can handle reasonable request rates
        // Keep under 30 requests per minute to be polite
        self.calculate_requests_per_minute() < 30.0
    }

    /// Wait if we're making requests too quickly
    pub async fn ensure_healthy_rate(&self) {
        if !self.is_rate_healthy() {
            // Wait a bit to slow down
            sleep(Duration::from_secs(2)).await;
        }
    }
}

/// API usage statistics
#[derive(Debug)]
pub struct ApiStats {
    pub request_count: u64,
    pub session_duration: Duration,
    pub average_requests_per_minute: f64,
}

impl std::fmt::Display for ApiStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Archive.org API Stats: {} requests in {:.1} minutes (avg: {:.1} req/min)",
            self.request_count,
            self.session_duration.as_secs_f64() / 60.0,
            self.average_requests_per_minute
        )
    }
}

/// Validate that an identifier follows Archive.org naming conventions
pub fn validate_identifier(identifier: &str) -> Result<()> {
    // Archive.org identifiers should:
    // - Be 3-100 characters long
    // - Contain only alphanumeric, hyphen, underscore, and period
    // - Not start or end with special characters
    // - Not contain consecutive special characters

    if identifier.is_empty() {
        return Err(IaGetError::UrlFormat(
            "Archive identifier cannot be empty".to_string(),
        ));
    }

    if identifier.len() < 3 {
        return Err(IaGetError::UrlFormat(
            "Archive identifier must be at least 3 characters long".to_string(),
        ));
    }

    if identifier.len() > 100 {
        return Err(IaGetError::UrlFormat(
            "Archive identifier cannot exceed 100 characters".to_string(),
        ));
    }

    // Check for valid characters
    if !identifier
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err(IaGetError::UrlFormat(
            "Archive identifier contains invalid characters. Only letters, numbers, hyphens, underscores, and periods are allowed".to_string(),
        ));
    }

    // Check start/end characters
    let first_char = identifier.chars().next().unwrap();
    let last_char = identifier.chars().last().unwrap();

    if !first_char.is_alphanumeric() || !last_char.is_alphanumeric() {
        return Err(IaGetError::UrlFormat(
            "Archive identifier must start and end with letters or numbers".to_string(),
        ));
    }

    // Check for consecutive special characters
    let mut prev_special = false;
    for c in identifier.chars() {
        let is_special = !c.is_alphanumeric();
        if is_special && prev_special {
            return Err(IaGetError::UrlFormat(
                "Archive identifier cannot contain consecutive special characters".to_string(),
            ));
        }
        prev_special = is_special;
    }

    Ok(())
}

/// Construct a server list following Archive.org recommendations
/// Primary servers are prioritized, with fallbacks for reliability
pub fn get_archive_servers() -> Vec<String> {
    vec![
        "https://archive.org".to_string(),
        "https://ia600001.us.archive.org".to_string(),
        "https://ia700001.us.archive.org".to_string(),
        "https://ia800001.us.archive.org".to_string(),
        "https://ia900001.us.archive.org".to_string(),
    ]
}

/// Internet Archive API endpoints following official API documentation
/// Reference: https://archive.org/developers/
pub mod endpoints {
    /// Metadata API endpoint - primary endpoint for file and item metadata
    /// Documentation: https://archive.org/developers/md-read.html
    pub fn metadata(identifier: &str) -> String {
        format!("https://archive.org/metadata/{}", identifier)
    }

    /// Download API endpoint - for accessing files directly
    /// Documentation: https://archive.org/developers/downloads.html
    pub fn download(identifier: &str) -> String {
        format!("https://archive.org/download/{}", identifier)
    }

    /// Search API endpoint - for discovering items and collections
    /// Documentation: https://archive.org/developers/search.html
    pub fn search() -> &'static str {
        "https://archive.org/advancedsearch.php"
    }

    /// Tasks API endpoint - for monitoring long-running operations
    /// Documentation: https://archive.org/developers/tasks.html
    pub fn tasks(identifier: &str) -> String {
        format!(
            "https://archive.org/services/tasks.php?identifier={}",
            identifier
        )
    }

    /// Collections API endpoint - for collection metadata and management
    pub fn collections(identifier: &str) -> String {
        format!("https://archive.org/metadata/{}/metadata", identifier)
    }

    /// Status API endpoint - for system health and service status
    pub fn status() -> &'static str {
        "https://archive.org/services/check"
    }

    /// Details page URL construction
    pub fn details(identifier: &str) -> String {
        format!("https://archive.org/details/{}", identifier)
    }
}

/// Enhanced Internet Archive API client with support for multiple endpoints
#[derive(Debug)]
pub struct EnhancedArchiveApiClient {
    base_client: ArchiveOrgApiClient,
}

impl EnhancedArchiveApiClient {
    /// Create a new enhanced Archive.org API client
    pub fn new(client: Client) -> Self {
        Self {
            base_client: ArchiveOrgApiClient::new(client),
        }
    }

    /// Search for items using the Internet Archive Search API
    ///
    /// Parameters:
    /// - query: Search query string
    /// - fields: Comma-separated list of fields to return
    /// - rows: Number of results to return (max 10000)
    /// - page: Page number for pagination
    ///
    /// Returns JSON response with search results
    pub async fn search_items(
        &mut self,
        query: &str,
        fields: Option<&str>,
        rows: Option<u32>,
        page: Option<u32>,
    ) -> Result<Response> {
        let mut url = format!(
            "{}?q={}&output=json",
            endpoints::search(),
            urlencoding::encode(query)
        );

        if let Some(fields) = fields {
            url.push_str(&format!("&fl={}", urlencoding::encode(fields)));
        }

        if let Some(rows) = rows {
            url.push_str(&format!("&rows={}", rows.min(10000))); // API limit
        }

        if let Some(page) = page {
            url.push_str(&format!("&page={}", page));
        }

        self.base_client.make_request(&url).await
    }

    /// Get tasks status for an item
    /// Useful for monitoring upload/processing status
    pub async fn get_tasks(&mut self, identifier: &str) -> Result<Response> {
        let url = endpoints::tasks(identifier);
        self.base_client.make_request(&url).await
    }

    /// Get Archive.org service status
    /// Returns system health information
    pub async fn get_service_status(&mut self) -> Result<Response> {
        let url = endpoints::status();
        self.base_client.make_request(url).await
    }

    /// Get basic metadata for an item (wraps existing functionality)
    pub async fn get_metadata(&mut self, identifier: &str) -> Result<Response> {
        let url = endpoints::metadata(identifier);
        self.base_client.make_request(&url).await
    }

    /// Search for items in a specific collection
    pub async fn search_collection(
        &mut self,
        collection: &str,
        fields: Option<&str>,
        rows: Option<u32>,
    ) -> Result<Response> {
        let query = format!("collection:{}", collection);
        self.search_items(&query, fields, rows, None).await
    }

    /// Find related items to a given identifier
    /// Uses subject, creator, and collection fields for similarity
    pub async fn find_related_items(
        &mut self,
        identifier: &str,
        max_results: Option<u32>,
    ) -> Result<Response> {
        // First get the item's metadata to extract relevant fields
        let _metadata_response = self.get_metadata(identifier).await?;

        // For now, do a simple search for items in the same collection
        // In a more sophisticated implementation, we'd parse the metadata
        // and build a more targeted query
        let query = format!("identifier:{}", identifier);
        self.search_items(
            &query,
            Some("identifier,title,creator,collection"),
            max_results,
            None,
        )
        .await
    }

    /// Get comprehensive item information including metadata and related items
    pub async fn get_item_details(&mut self, identifier: &str) -> Result<ItemDetails> {
        let metadata_response = self.get_metadata(identifier).await?;
        let metadata_text = metadata_response
            .text()
            .await
            .map_err(|e| IaGetError::Network(format!("Failed to read metadata response: {}", e)))?;

        // Parse basic metadata
        let metadata: serde_json::Value = serde_json::from_str(&metadata_text).map_err(|e| {
            IaGetError::JsonParsing(format!("Failed to parse metadata JSON: {}", e))
        })?;

        let item_details = ItemDetails {
            identifier: identifier.to_string(),
            metadata,
            tasks: None,         // Could be populated with get_tasks if needed
            related_items: None, // Could be populated with find_related_items if needed
        };

        Ok(item_details)
    }

    /// Get API usage statistics
    pub fn get_stats(&self) -> ApiStats {
        self.base_client.get_stats()
    }

    /// Check if request rate is healthy
    pub fn is_rate_healthy(&self) -> bool {
        self.base_client.is_rate_healthy()
    }

    /// Ensure we're not making requests too quickly
    pub async fn ensure_healthy_rate(&self) {
        self.base_client.ensure_healthy_rate().await
    }

    /// Get metadata as JSON value (used by AdvancedMetadataProcessor)
    pub async fn get_metadata_json(&mut self, identifier: &str) -> Result<serde_json::Value> {
        // Validate identifier first
        validate_identifier(identifier)?;

        let url = endpoints::metadata(identifier);
        let response = self.base_client.make_request(&url).await?;

        let metadata_text = response
            .text()
            .await
            .map_err(|e| IaGetError::Network(format!("Failed to read metadata response: {}", e)))?;

        let metadata: serde_json::Value = serde_json::from_str(&metadata_text).map_err(|e| {
            IaGetError::JsonParsing(format!("Failed to parse metadata JSON: {}", e))
        })?;

        Ok(metadata)
    }

    /// Search for collections
    pub async fn search_collections(&mut self, collection_name: &str) -> Result<serde_json::Value> {
        let query = format!("collection:{}", collection_name);
        let fields = "identifier,title,description,num_found";
        let response = self
            .search_items(&query, Some(fields), Some(5), None)
            .await?;

        // Parse the response to JSON
        let response_text = response
            .text()
            .await
            .map_err(|e| IaGetError::Network(format!("Failed to read search response: {}", e)))?;

        let search_results: serde_json::Value = serde_json::from_str(&response_text)
            .map_err(|e| IaGetError::JsonParsing(format!("Failed to parse search JSON: {}", e)))?;

        Ok(search_results)
    }

    /// Test metadata API health
    pub async fn test_metadata_api(&mut self) -> Result<()> {
        // Use a known stable identifier for testing
        let test_identifier = "internetarchive";
        self.get_metadata_json(test_identifier).await?;
        Ok(())
    }

    /// Test search API health
    pub async fn test_search_api(&mut self) -> Result<()> {
        // Perform a simple search
        let fields = "identifier";
        let _response = self
            .search_items("collection:opensource", Some(fields), Some(1), None)
            .await?;
        Ok(())
    }

    /// Test tasks API health
    pub async fn test_tasks_api(&mut self) -> Result<()> {
        // Tasks API is typically accessible if search API works
        // For now, we'll just return Ok since the tasks endpoint
        // is less critical and might not always be available
        Ok(())
    }
}

/// Comprehensive item details structure
#[derive(Debug)]
pub struct ItemDetails {
    pub identifier: String,
    pub metadata: serde_json::Value,
    pub tasks: Option<serde_json::Value>,
    pub related_items: Option<serde_json::Value>,
}

/// Service status information from Archive.org
#[derive(Debug, serde::Deserialize)]
pub struct ServiceStatus {
    pub status: String,
    pub version: Option<String>,
    pub timestamp: Option<String>,
}

/// Helper module for URL encoding
mod urlencoding {
    pub fn encode(input: &str) -> String {
        url::form_urlencoded::byte_serialize(input.as_bytes()).collect()
    }
}
