//! # ia-get
//!
//! A command-line tool for downloading files from the Internet Archive.
//! 
//! This tool takes an archive.org details URL and downloads all associated files,
//! with support for resumable downloads and MD5 hash verification.

use ia_get::{IaGetError, Result};
use ia_get::utils::create_spinner;
use ia_get::downloader; 
use ia_get::constants::{USER_AGENT, HTTP_TIMEOUT, URL_PATTERN};
use indicatif::ProgressStyle;
use regex::Regex;
use reqwest::Client;
use serde_xml_rs::from_str;
use clap::Parser;
use std::process;
use ia_get::archive_metadata::XmlFiles;

/// Checks if a URL is accessible by sending a HEAD request
async fn is_url_accessible(url: &str, client: &Client) -> Result<()> {
    let response = client.head(url)
        .timeout(std::time::Duration::from_secs(HTTP_TIMEOUT))
        .send().await?;
    
    response.error_for_status()?;
    Ok(())
}

/// Converts a details URL to the corresponding XML files list URL
/// 
/// Takes an archive.org details URL and converts it to the XML metadata URL
/// by replacing "details" with "download" and appending "_files.xml"
/// 
/// # Arguments
/// * `original_url` - The archive.org details URL
/// 
/// # Returns
/// The corresponding XML files list URL
fn get_xml_url(original_url: &str) -> String {
    let base_new_url = original_url.replacen("details", "download", 1);
    if let Some(last_segment) = original_url.split('/').next_back() {
        format!("{}/{}_files.xml", base_new_url, last_segment)
    } else {
        base_new_url
    }
}

/// Fetches and parses XML metadata from archive.org
/// 
/// Combines XML URL generation, accessibility check, download, and parsing
/// into a single operation with integrated error handling.
/// 
/// # Arguments
/// * `details_url` - The original archive.org details URL
/// * `client` - HTTP client for requests
/// * `spinner` - Progress spinner to update during processing
/// 
/// # Returns
/// Tuple of (XmlFiles, base_url) for download processing
async fn fetch_xml_metadata(
    details_url: &str, 
    client: &Client, 
    spinner: &indicatif::ProgressBar
) -> Result<(XmlFiles, reqwest::Url)> {
    // Generate XML URL
    let xml_url = get_xml_url(details_url);
    spinner.set_message(format!("Accessing XML metadata: {}", xml_url));

    // Check XML URL accessibility
    if let Err(e) = is_url_accessible(&xml_url, client).await {
        spinner.finish_with_message(format!("🔴 XML metadata not accessible: {}", xml_url));
        eprintln!("╰╼ Exiting due to error: {}", e);
        process::exit(1);
    }

    spinner.set_message("Parsing archive metadata... 👀");
    
    // Parse base URL and fetch XML content
    let base_url = reqwest::Url::parse(&xml_url)?;
    let response = client.get(&xml_url).send().await?;
    let xml_content = response.text().await?;
    
    // Parse XML content with error context
    let files: XmlFiles = from_str(&xml_content)
        .map_err(|e| {
            eprintln!("XML parsing error: {}", e);
            eprintln!("XML content: {}", &xml_content[..xml_content.len().min(1000)]);
            e
        })?;

    Ok((files, base_url))
}

/// Command-line interface for ia-get
#[derive(Parser)]
#[command(name = "ia-get")]
#[command(about = "A command-line tool for downloading files from the Internet Archive")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(author = env!("CARGO_PKG_AUTHORS"))]
struct Cli {
    /// URL to an archive.org details page
    url: String,
}

/// Main application entry point
/// 
/// Parses command line arguments, validates the archive.org URL, checks URL accessibility,
/// downloads XML metadata, and initiates file downloads with built-in signal handling.
#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    
    // Create a single client instance for all requests
    let client = Client::builder()
        .user_agent(USER_AGENT)
        .timeout(std::time::Duration::from_secs(HTTP_TIMEOUT))
        .build()?;

    // Compile regex pattern for URL validation
    let url_regex = Regex::new(URL_PATTERN)?;

    // Start a single spinner for the entire initialization process
    let spinner = create_spinner(&format!("Processing archive.org URL: {}", cli.url));
    
    // Validate URL format
    if !url_regex.is_match(&cli.url) {
        spinner.finish_with_message(format!("❌ Invalid archive.org URL format: {}", cli.url));
        println!("├╼ Archive.org URL is not in the expected format");
        println!("╰╼ Expected format: https://archive.org/details/<identifier>/");
        return Err(IaGetError::UrlFormat(format!("URL '{}' does not match expected format", cli.url)).into());
    }

    // Check URL accessibility
    if let Err(e) = is_url_accessible(&cli.url, &client).await {
        spinner.finish_with_message(format!("🔴 Archive.org URL not accessible: {}", cli.url));
        eprintln!("╰╼ Exiting due to error: {}", e);
        process::exit(1);
    }

    // Fetch and parse XML metadata in one operation
    let (files, base_url) = fetch_xml_metadata(&cli.url, &client, &spinner).await?;

    // Successfully finished initialization - replace with green tick
    spinner.set_style(
        ProgressStyle::default_spinner()
            .template(&format!("✅ Ready to download {} files from archive.org ✨", files.files.len()))
            .expect("Failed to set completion style")
    );
    spinner.finish();

    // Prepare download data for batch processing
    let download_data = files.files.into_iter().map(|file| {
        let mut absolute_url = base_url.clone();
        if let Ok(joined_url) = absolute_url.join(&file.name) {
            absolute_url = joined_url;
        }
        (absolute_url.to_string(), file.name, file.md5)
    }).collect::<Vec<_>>();

    // Download all files with integrated signal handling
    downloader::download_files(&client, download_data).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ia_get::constants::URL_PATTERN;

    #[test]
    fn check_valid_pattern() {
        let url_regex = Regex::new(URL_PATTERN).unwrap();
        assert!(url_regex.is_match("https://archive.org/details/Valid-Pattern"));
        assert!(url_regex.is_match("https://archive.org/details/test123"));
        assert!(url_regex.is_match("https://archive.org/details/test_file-name.data"));
        assert!(url_regex.is_match("https://archive.org/details/user@domain"));
    }

    #[test]
    fn check_invalid_pattern() {
        let url_regex = Regex::new(URL_PATTERN).unwrap();
        assert!(!url_regex.is_match("https://archive.org/details/Invalid-Pattern-*"));
        assert!(!url_regex.is_match("https://archive.org/details/"));
        assert!(!url_regex.is_match("https://example.com/details/test"));
        assert!(!url_regex.is_match("http://archive.org/details/test"));
        assert!(!url_regex.is_match("https://archive.org/details/test/extra"));
    }
}
