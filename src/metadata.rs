//! Metadata processing module for ia-get
//!
//! Handles JSON and XML metadata fetching and parsing from Internet Archive.
//! Implements the Internet Archive Metadata API as documented at:
//! https://archive.org/developers/md-read.html

use crate::{
    Result, 
    error::IaGetError, 
    archive_metadata::{XmlFiles, parse_xml_files}, 
    metadata_storage::ArchiveMetadata,
    network::is_transient_error
};
use reqwest::Client;
use colored::*;
use indicatif::ProgressBar;

/// Generates XML metadata URL from archive.org details URL (legacy)
pub fn get_xml_url(original_url: &str) -> String {
    if original_url.contains("/details/") {
        original_url.replace("/details/", "/metadata/") + "?output=xml"
    } else if original_url.contains("://archive.org/metadata/") {
        if original_url.contains("?output=xml") || original_url.contains("&output=xml") {
            original_url.to_string()
        } else {
            original_url.to_string() + "?output=xml"
        }
    } else {
        // Fallback: extract identifier and construct XML URL
        let identifier = original_url
            .rsplit('/')
            .next()
            .unwrap_or(original_url);
        format!("https://archive.org/metadata/{}?output=xml", identifier)
    }
}

/// Generates JSON metadata URL from archive.org details URL (current API default)
pub fn get_json_url(original_url: &str) -> String {
    if original_url.contains("/details/") {
        original_url.replace("/details/", "/metadata/")
    } else if original_url.contains("://archive.org/metadata/") {
        // Remove any output=xml parameter for JSON API
        original_url.replace("?output=xml", "").replace("&output=xml", "")
    } else {
        // Fallback: extract identifier and construct JSON URL
        let identifier = original_url
            .rsplit('/')
            .next()
            .unwrap_or(original_url);
        format!("https://archive.org/metadata/{}", identifier)
    }
}

/// Fetches and parses XML metadata with retry logic for transient errors
pub async fn fetch_xml_metadata(
    details_url: &str,
    client: &Client,
    spinner: &ProgressBar,
) -> Result<(XmlFiles, reqwest::Url)> {
    // Generate XML URL
    let xml_url = get_xml_url(details_url);
    spinner.set_message(format!(
        "{} Accessing XML metadata: {}",
        "⚙".blue(),
        xml_url.bold()
    ));

    // Check XML URL accessibility
    if let Err(e) = crate::network::is_url_accessible(&xml_url, client, Some(spinner)).await {
        spinner.finish_with_message(format!(
            "{} XML metadata not accessible: {}",
            "✘".red().bold(),
            xml_url.bold()
        ));
        return Err(e); // Propagate the error
    }

    spinner.set_message(format!("{} {}", "⚙".blue(), "Parsing archive metadata...".bold()));

    // Parse base URL and fetch XML content with retry logic
    let base_url = reqwest::Url::parse(&xml_url).map_err(|e| IaGetError::Network(format!("URL parse failed: {}", e)))?;
    let mut retries = 0;
    let max_retries = 3;
    let mut delay = std::time::Duration::from_secs(60);
    let max_delay = std::time::Duration::from_secs(900); // 15 minutes
    let xml_content = loop {
        let result = client.get(&xml_url).send().await.map_err(|e| IaGetError::Network(format!("GET request failed: {}", e)));
        match result {
            Ok(response) => {
                // Check for HTTP 429 and Retry-After header BEFORE moving response
                if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                    let wait_time = response.headers().get(reqwest::header::RETRY_AFTER)
                        .and_then(|h| h.to_str().ok())
                        .and_then(|s| s.parse::<u64>().ok())
                        .unwrap_or(60);
                    
                    let wait_reason = format!("Rate limited during XML fetch (HTTP 429) - waiting {}s as requested", wait_time);
                    spinner.set_message(format!("{} {}", "⏳".yellow(), wait_reason));
                    
                    tokio::time::sleep(std::time::Duration::from_secs(wait_time)).await;
                    retries += 1;
                    continue;
                }
                if let Err(e) = response.error_for_status_ref() {
                    if retries < max_retries && is_transient_error(&e) {
                        let wait_reason = format!("Server error during XML fetch (HTTP {}) - retrying in {}s (attempt {}/{})", 
                                                e.status().map(|s| s.as_u16()).unwrap_or(0), 
                                                delay.as_secs(), 
                                                retries + 1, 
                                                max_retries);
                        spinner.set_message(format!("{} {}", "⏳".yellow(), wait_reason));
                        
                        retries += 1;
                        tokio::time::sleep(delay).await;
                        delay = std::cmp::min(delay * 2, max_delay);
                        continue;
                    } else {
                        return Err(e.into());
                    }
                }
                let text = response.text().await;
                match text {
                    Ok(t) => break t,
                    Err(e) => {
                        let is_transient = crate::network::is_transient_reqwest_error(&e);
                        if retries < max_retries && is_transient {
                            let wait_reason = format!("Network error during XML fetch - retrying in {}s (attempt {}/{})", 
                                                    delay.as_secs(), 
                                                    retries + 1, 
                                                    max_retries);
                            spinner.set_message(format!("{} {}", "⏳".yellow(), wait_reason));
                            
                            retries += 1;
                            tokio::time::sleep(delay).await;
                            delay = std::cmp::min(delay * 2, max_delay);
                            continue;
                        } else {
                            return Err(e.into());
                        }
                    }
                }
            }
            Err(e) => {
                let is_transient = matches!(&e, IaGetError::Network(_));
                if retries < max_retries && is_transient {
                    let wait_reason = format!("Network error during XML fetch - retrying in {}s (attempt {}/{})", 
                                            delay.as_secs(), 
                                            retries + 1, 
                                            max_retries);
                    spinner.set_message(format!("{} {}", "⏳".yellow(), wait_reason));
                    
                    retries += 1;
                    tokio::time::sleep(delay).await;
                    delay = std::cmp::min(delay * 2, max_delay);
                    continue;
                } else {
                    return Err(e);
                }
            }
        }
    };
    // Parse XML content with improved error handling
    let files = parse_xml_files(&xml_content)?;

    Ok((files, base_url))
}

/// Fetches and parses JSON metadata with retry logic for transient errors (primary method)
pub async fn fetch_json_metadata(
    details_url: &str,
    client: &Client,
    spinner: &ProgressBar,
) -> Result<(ArchiveMetadata, reqwest::Url)> {
    // Generate JSON URL
    let json_url = get_json_url(details_url);
    spinner.set_message(format!(
        "{} Accessing JSON metadata: {}",
        "⚙".blue(),
        json_url.bold()
    ));

    // Check JSON URL accessibility
    if let Err(e) = crate::network::is_url_accessible(&json_url, client, Some(spinner)).await {
        spinner.finish_with_message(format!(
            "{} JSON metadata not accessible: {}",
            "✘".red().bold(),
            json_url.bold()
        ));
        return Err(e); // Propagate the error
    }

    spinner.set_message(format!("{} {}", "⚙".blue(), "Parsing archive metadata...".bold()));

    // Parse base URL and fetch JSON content with retry logic
    let base_url = reqwest::Url::parse(&json_url).map_err(|e| IaGetError::Network(format!("URL parse failed: {}", e)))?;
    let mut retries = 0;
    let max_retries = 3;
    let mut delay = std::time::Duration::from_secs(60);
    let max_delay = std::time::Duration::from_secs(900); // 15 minutes
    let json_content = loop {
        let result = client.get(&json_url).send().await.map_err(|e| IaGetError::Network(format!("GET request failed: {}", e)));
        match result {
            Ok(response) => {
                // Check for HTTP 429 and Retry-After header BEFORE moving response
                if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                    let wait_time = response.headers().get(reqwest::header::RETRY_AFTER)
                        .and_then(|h| h.to_str().ok())
                        .and_then(|s| s.parse::<u64>().ok())
                        .unwrap_or(60);
                    
                    let wait_reason = format!("Rate limited during JSON fetch (HTTP 429) - waiting {}s as requested", wait_time);
                    spinner.set_message(format!("{} {}", "⏳".yellow(), wait_reason));
                    
                    tokio::time::sleep(std::time::Duration::from_secs(wait_time)).await;
                    retries += 1;
                    continue;
                }
                if let Err(e) = response.error_for_status_ref() {
                    if retries < max_retries && is_transient_error(&e) {
                        let wait_reason = format!("Server error during JSON fetch (HTTP {}) - retrying in {}s (attempt {}/{})", 
                                                e.status().map(|s| s.as_u16()).unwrap_or(0), 
                                                delay.as_secs(), 
                                                retries + 1, 
                                                max_retries);
                        spinner.set_message(format!("{} {}", "⏳".yellow(), wait_reason));
                        
                        retries += 1;
                        tokio::time::sleep(delay).await;
                        delay = std::cmp::min(delay * 2, max_delay);
                        continue;
                    } else {
                        return Err(e.into());
                    }
                }
                let text = response.text().await;
                match text {
                    Ok(t) => break t,
                    Err(e) => {
                        let is_transient = crate::network::is_transient_reqwest_error(&e);
                        if retries < max_retries && is_transient {
                            let wait_reason = format!("Network error during JSON fetch - retrying in {}s (attempt {}/{})", 
                                                    delay.as_secs(), 
                                                    retries + 1, 
                                                    max_retries);
                            spinner.set_message(format!("{} {}", "⏳".yellow(), wait_reason));
                            
                            retries += 1;
                            tokio::time::sleep(delay).await;
                            delay = std::cmp::min(delay * 2, max_delay);
                            continue;
                        } else {
                            return Err(e.into());
                        }
                    }
                }
            }
            Err(e) => {
                let is_transient = matches!(&e, IaGetError::Network(_));
                if retries < max_retries && is_transient {
                    let wait_reason = format!("Network error during JSON fetch - retrying in {}s (attempt {}/{})", 
                                            delay.as_secs(), 
                                            retries + 1, 
                                            max_retries);
                    spinner.set_message(format!("{} {}", "⏳".yellow(), wait_reason));
                    
                    retries += 1;
                    tokio::time::sleep(delay).await;
                    delay = std::cmp::min(delay * 2, max_delay);
                    continue;
                } else {
                    return Err(e);
                }
            }
        }
    };
    // Parse JSON content as ArchiveMetadata
    let metadata = parse_archive_metadata(&json_content)?;

    Ok((metadata, base_url))
}

/// Parses JSON content into ArchiveMetadata structure
/// 
/// # Arguments
/// * `json_content` - Raw JSON content string from archive.org
/// 
/// # Returns
/// * `Ok(ArchiveMetadata)` if parsing succeeds
/// * `Err(IaGetError)` with context if parsing fails
pub fn parse_archive_metadata(json_content: &str) -> Result<ArchiveMetadata> {
    serde_json::from_str(json_content).map_err(|e| {
        let preview = if json_content.len() > crate::constants::XML_DEBUG_TRUNCATE_LEN {
            &json_content[..crate::constants::XML_DEBUG_TRUNCATE_LEN]
        } else {
            json_content
        };
        
        IaGetError::JsonParsing(format!(
            "Failed to parse JSON metadata: {}. Content preview: {}{}",
            e,
            preview,
            if json_content.len() > crate::constants::XML_DEBUG_TRUNCATE_LEN { "..." } else { "" }
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_xml_url_details() {
        let details_url = "https://archive.org/details/example";
        let xml_url = get_xml_url(details_url);
        assert_eq!(xml_url, "https://archive.org/metadata/example?output=xml");
    }

    #[test]
    fn test_get_xml_url_metadata() {
        let metadata_url = "https://archive.org/metadata/example";
        let xml_url = get_xml_url(metadata_url);
        assert_eq!(xml_url, "https://archive.org/metadata/example?output=xml");
    }

    #[test]
    fn test_get_xml_url_already_xml() {
        let xml_url_input = "https://archive.org/metadata/example?output=xml";
        let xml_url = get_xml_url(xml_url_input);
        assert_eq!(xml_url, "https://archive.org/metadata/example?output=xml");
    }

    #[test]
    fn test_get_xml_url_fallback() {
        let simple_url = "example";
        let xml_url = get_xml_url(simple_url);
        assert_eq!(xml_url, "https://archive.org/metadata/example?output=xml");
    }

    #[test]
    fn test_get_json_url_details() {
        let details_url = "https://archive.org/details/example";
        let json_url = get_json_url(details_url);
        assert_eq!(json_url, "https://archive.org/metadata/example");
    }

    #[test]
    fn test_get_json_url_metadata() {
        let metadata_url = "https://archive.org/metadata/example";
        let json_url = get_json_url(metadata_url);
        assert_eq!(json_url, "https://archive.org/metadata/example");
    }

    #[test]
    fn test_get_json_url_removes_xml_param() {
        let xml_url = "https://archive.org/metadata/example?output=xml";
        let json_url = get_json_url(xml_url);
        assert_eq!(json_url, "https://archive.org/metadata/example");
    }

    #[test]
    fn test_get_json_url_fallback() {
        let simple_url = "example";
        let json_url = get_json_url(simple_url);
        assert_eq!(json_url, "https://archive.org/metadata/example");
    }
}
