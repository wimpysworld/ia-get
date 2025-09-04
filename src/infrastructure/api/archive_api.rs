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
