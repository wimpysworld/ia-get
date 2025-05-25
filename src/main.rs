//! # ia-get
//!
//! A command-line tool for downloading files from the Internet Archive.
//! 
//! This tool takes an archive.org details URL and downloads all associated files,
//! with support for resumable downloads and MD5 hash verification.

use ia_get::{IaGetError, Result};
use ia_get::utils::create_spinner; // Removed format_duration, format_size, format_transfer_rate
use ia_get::downloader; 
use indicatif::ProgressStyle;
use regex::Regex;
// Removed use reqwest::header::{HeaderValue, HeaderMap};
use reqwest::Client;
use serde_xml_rs::from_str;
use clap::Parser;
// Removed use std::fs::{self, File};
// Removed use std::io::{BufReader, Read, Write};
use std::process;
// Removed use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use ia_get::archive_metadata::XmlFiles;

/// User agent string for HTTP requests
const USER_AGENT: &str = "ia-get";

/// Default timeout for HTTP requests in seconds
const DEFAULT_HTTP_TIMEOUT: u64 = 60;

/// Timeout for URL accessibility checks in seconds
const URL_CHECK_TIMEOUT: u64 = 30;

/// Regex pattern for validating archive.org details URLs
const PATTERN: &str = r"^https://archive\.org/details/[a-zA-Z0-9_\-.@]+$";

/// Checks if a URL is accessible by sending a HEAD request
async fn is_url_accessible(url: &str, client: &Client) -> Result<()> {
    let response = client.head(url)
        .timeout(std::time::Duration::from_secs(URL_CHECK_TIMEOUT))
        .send().await
        .map_err(|e| {
            if e.is_connect() || e.is_timeout() {
                IaGetError::UrlError(format!("Failed to connect to {}: {}", url, e))
            } else {
                IaGetError::NetworkError(e)
            }
        })?;
    
    response.error_for_status()
        .map_err(|e| {
            let status = e.status().unwrap_or_default();
            IaGetError::UrlError(format!("URL returned error status {}: {}", status, url))
        })?;
    
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

/// Sets up signal handling for graceful shutdown on Ctrl+C
/// 
/// Returns an Arc<AtomicBool> that can be checked to see if the process
/// should stop. When Ctrl+C is pressed, this will be set to false.
fn setup_signal_handler() -> Arc<AtomicBool> {
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    
    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
        println!("\nReceived Ctrl+C, finishing current operation...");
    }).expect("Error setting Ctrl+C handler");
    
    running
}

/// Main application entry point
/// 
/// Parses command line arguments, validates the archive.org URL, checks URL accessibility,
/// downloads XML metadata, and iterates through files to download them with resume capability
/// and hash verification.
#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    
    // Set up signal handling for graceful shutdown
    let running = setup_signal_handler();
    
    // Create a single client instance for all requests
    let client = Client::builder()
        .user_agent(USER_AGENT)
        .timeout(std::time::Duration::from_secs(DEFAULT_HTTP_TIMEOUT))
        .build()?;

    // Create a regex object with the static pattern
    let regex = Regex::new(PATTERN)?;

    // Start a single spinner for the entire initialization process
    let spinner = create_spinner(&format!("Processing archive.org URL: {}", cli.url));
    
    // Validate URL format
    if !regex.is_match(&cli.url) {
        spinner.finish_with_message(format!("❌ Invalid archive.org URL format: {}", cli.url));
        println!("├╼ Archive.org URL is not in the expected format");
        println!("╰╼ Expected format: https://archive.org/details/<identifier>/");
        return Err(IaGetError::UrlFormatError(format!("URL '{}' does not match expected format", cli.url)).into());
    }

    // Check URL accessibility
    if let Err(e) = is_url_accessible(&cli.url, &client).await {
        spinner.finish_with_message(format!("🔴 Archive.org URL not accessible: {}", cli.url));
        eprintln!("╰╼ Exiting due to error: {}", e);
        process::exit(1);
    }

    // Derive XML URL
    let xml_url = get_xml_url(&cli.url);
    spinner.set_message(format!("Accessing XML metadata: {}", xml_url));

    // Check XML URL accessibility
    if let Err(e) = is_url_accessible(&xml_url, &client).await {
        spinner.finish_with_message(format!("🔴 XML metadata not accessible: {}", xml_url));
        eprintln!("╰╼ Exiting due to error: {}", e);
        process::exit(1);
    }

    // Parse XML metadata
    spinner.set_message("Parsing archive metadata... 👀");
    
    // Get the base URL from the XML URL
    let base_url = match reqwest::Url::parse(&xml_url) {
        Ok(url) => url,
        Err(e) => {
            spinner.finish_with_message("Failed to parse XML URL ❌");
            return Err(e.into());
        }
    };

    // Download XML file
    let response = match client.get(xml_url).send().await {
        Ok(resp) => {
            match resp.text().await {
                Ok(text) => text,
                Err(e) => {
                    spinner.finish_with_message("Failed to read XML response ❌");
                    return Err(e.into());
                }
            }
        },
        Err(e) => {
            spinner.finish_with_message("Failed to fetch XML metadata ❌");
            return Err(e.into());
        }
    };
    
    // Parse XML content
    let files: XmlFiles = match from_str(&response) {
        Ok(files) => files,
        Err(e) => {
            spinner.finish_with_message("Failed to parse XML content ❌");
            eprintln!("XML parsing error: {}", e);
            eprintln!("XML content: {}", &response[..response.len().min(1000)]);
            return Err(e.into());
        }
    };

    // Successfully finished initialization - replace with green tick
    spinner.set_style(
        ProgressStyle::default_spinner()
            .template(&format!("✅ Ready to download {} files from archive.org ✨", files.files.len()))
            .expect("Failed to set completion style")
    );
    spinner.finish();

    // Iterate over the XML files struct and download each file
    for file in files.files {
        // Check if we should stop due to signal
        if !running.load(Ordering::SeqCst) {
            println!("\nDownload interrupted. Run the command again to resume remaining files.");
            break;
        }

        // Create a clone of the base URL
        let mut absolute_url = base_url.clone();

        // If the URL is relative, join it with the base_url to make it absolute
        if let Ok(joined_url) = absolute_url.join(&file.name) {
            absolute_url = joined_url;
        }

        // Download the file
        downloader::download_file( // Updated to use downloader::download_file
            &client, 
            absolute_url.as_str(), 
            &file.name, 
            file.md5.as_deref(), 
            &running
        ).await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_valid_pattern() {
        let regex = Regex::new(PATTERN).expect("Create regex");
        assert!(regex.is_match("https://archive.org/details/Valid-Pattern"));
        assert!(regex.is_match("https://archive.org/details/test123"));
        assert!(regex.is_match("https://archive.org/details/test_file-name.data"));
        assert!(regex.is_match("https://archive.org/details/user@domain"));
    }

    #[test]
    fn check_invalid_pattern() {
        let regex = Regex::new(PATTERN).expect("Create regex");
        assert!(!regex.is_match("https://archive.org/details/Invalid-Pattern-*"));
        assert!(!regex.is_match("https://archive.org/details/"));
        assert!(!regex.is_match("https://example.com/details/test"));
        assert!(!regex.is_match("http://archive.org/details/test"));
        assert!(!regex.is_match("https://archive.org/details/test/extra"));
    }
}
