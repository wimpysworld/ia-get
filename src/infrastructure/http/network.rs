//! Network operations module for ia-get
//!
//! Handles HTTP requests, retries, and response processing for Internet Archive interactions.

use crate::{Result, error::IaGetError, utilities::common::HTTP_TIMEOUT};
use colored::*;
use reqwest::Client;

/// Checks if a URL is accessible by sending appropriate request method, with retry logic and dynamic wait reasons
pub async fn is_url_accessible(
    url: &str,
    client: &Client,
    spinner: Option<&indicatif::ProgressBar>,
) -> Result<()> {
    let mut retries = 0;
    let max_retries = 5;
    let mut delay = std::time::Duration::from_secs(30); // More conservative initial delay
    let max_delay = std::time::Duration::from_secs(600); // 10 minutes max (reduced from 15)

    loop {
        // Use GET for metadata URLs since Archive.org returns 405 for HEAD requests on metadata endpoints
        let result = if url.contains("/metadata/") {
            client
                .get(url)
                .timeout(std::time::Duration::from_secs(HTTP_TIMEOUT))
                .send()
                .await
                .map_err(|e| IaGetError::Network(format!("GET request failed: {}", e)))
        } else {
            client
                .head(url)
                .timeout(std::time::Duration::from_secs(HTTP_TIMEOUT))
                .send()
                .await
                .map_err(|e| IaGetError::Network(format!("HEAD request failed: {}", e)))
        };

        match result {
            Ok(response) => {
                // Check for HTTP 429 and Retry-After header BEFORE moving response
                if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                    let wait_time = if let Some(retry_after) =
                        response.headers().get(reqwest::header::RETRY_AFTER)
                    {
                        retry_after
                            .to_str()
                            .ok()
                            .and_then(|s| s.parse::<u64>().ok())
                            .unwrap_or(60)
                    } else {
                        60 // Default if no Retry-After header
                    };

                    let wait_reason = format!(
                        "Rate limited by server (HTTP 429) - waiting {}s as requested",
                        wait_time
                    );
                    if let Some(spinner) = spinner {
                        spinner.set_message(format!("{} {}", "⏳".yellow(), wait_reason));
                    } else {
                        eprintln!("{} {}", "▲".yellow(), wait_reason);
                    }

                    tokio::time::sleep(std::time::Duration::from_secs(wait_time)).await;
                    retries += 1;
                    continue;
                }

                if let Err(e) = response.error_for_status() {
                    if retries < max_retries && is_transient_error(&e) {
                        let wait_reason = format!(
                            "Server error (HTTP {}) - retrying in {}s (attempt {}/{})",
                            e.status().map(|s| s.as_u16()).unwrap_or(0),
                            delay.as_secs(),
                            retries + 1,
                            max_retries
                        );

                        if let Some(spinner) = spinner {
                            spinner.set_message(format!("{} {}", "⏳".yellow(), wait_reason));
                        } else {
                            eprintln!("{} {}", "▲".yellow(), wait_reason);
                        }

                        retries += 1;
                        tokio::time::sleep(delay).await;
                        delay = std::cmp::min(delay * 2, max_delay);
                        continue;
                    } else {
                        return Err(e.into());
                    }
                }
                return Ok(());
            }
            Err(e) => {
                let is_transient = matches!(&e, IaGetError::Network(_));
                if retries < max_retries && is_transient {
                    let wait_reason = format!(
                        "Network error - retrying in {}s (attempt {}/{})",
                        delay.as_secs(),
                        retries + 1,
                        max_retries
                    );

                    if let Some(spinner) = spinner {
                        spinner.set_message(format!("{} {}", "⏳".yellow(), wait_reason));
                    } else {
                        eprintln!("{} {}", "▲".yellow(), wait_reason);
                    }

                    retries += 1;
                    tokio::time::sleep(delay).await;
                    delay = std::cmp::min(delay * 2, max_delay);
                    continue;
                } else {
                    return Err(e);
                }
            }
        }
    }
}

/// Determines if a reqwest::Error is transient and should be retried
pub fn is_transient_reqwest_error(e: &reqwest::Error) -> bool {
    // Check for network-level transient errors
    if e.is_timeout() || e.is_connect() || e.is_request() || e.is_body() {
        return true;
    }

    // Check for HTTP status-based transient errors
    if let Some(status) = e.status() {
        status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS
    } else {
        false
    }
}

/// Determines if a reqwest::StatusCode error is transient (5xx, 429, etc.)
/// This is kept for backward compatibility and specific status-only checks
pub fn is_transient_error(e: &reqwest::Error) -> bool {
    if let Some(status) = e.status() {
        status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS
    } else {
        false
    }
}
