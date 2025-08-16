//! # ia-get
//!
//! A command-line tool for downloading files from the Internet Archive.
//! 
//! This tool takes an archive.org details URL and downloads all associated files,
//! with support for resumable downloads and MD5 hash verification.

use ia_get::{Result};
use ia_get::error::IaGetError;
use ia_get::utils::{create_spinner, validate_archive_url};
use ia_get::downloader; 
use ia_get::constants::{USER_AGENT, HTTP_TIMEOUT};
use ia_get::archive_metadata::{XmlFiles, parse_xml_files};
use indicatif::ProgressStyle;
use reqwest::Client;
use clap::Parser;
use colored::*; // Add this line

/// Checks if a URL is accessible by sending a HEAD request, with retry logic
async fn is_url_accessible(url: &str, client: &Client) -> Result<()> {
    let mut retries = 0;
    let max_retries = 5;
    let mut delay = std::time::Duration::from_secs(60);
    let max_delay = std::time::Duration::from_secs(900); // 15 minutes
    loop {
        let result = client.head(url)
            .timeout(std::time::Duration::from_secs(HTTP_TIMEOUT))
            .send().await
            .map_err(|e| IaGetError::Network(format!("HEAD request failed: {}", e)));
        match result {
            Ok(response) => {
                // Check for HTTP 429 and Retry-After header BEFORE moving response
                if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                    if let Some(retry_after) = response.headers().get(reqwest::header::RETRY_AFTER) {
                        if let Ok(retry_after_str) = retry_after.to_str() {
                            if let Ok(secs) = retry_after_str.parse::<u64>() {
                                eprintln!("{} Rate limited (429). Waiting {}s before retry...", "▲".yellow(), secs);
                                tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
                                retries += 1;
                                continue;
                            }
                        }
                    }
                }
                if let Err(e) = response.error_for_status() {
                    if retries < max_retries && is_transient_error(&e) {
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

/// Determines if a reqwest::Error is transient (network, timeout, etc.)
fn is_transient_reqwest_error(e: &reqwest::Error) -> bool {
    e.is_timeout() || e.is_connect() || e.is_request() || e.is_body() || e.is_status()
}

/// Determines if a reqwest::StatusCode error is transient (5xx, 429, etc.)
fn is_transient_error(e: &reqwest::Error) -> bool {
    if let Some(status) = e.status() {
        status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS
    } else {
        false
    }
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
    // Remove trailing slash if present to get a consistent base for identifier extraction
    let trimmed_url = original_url.trim_end_matches('/');

    // The identifier is the last segment of the trimmed URL
    // This expect is considered safe because get_xml_url is only called after
    // validate_archive_url has confirmed the URL structure.
    let identifier = trimmed_url.rsplit('/').next() // Changed from split().last() to address clippy warning
        .expect("Validated URL should have a valid identifier segment after validation");

    // The base URL for download is "https://archive.org/download/{identifier}"
    let download_url_base = format!("https://archive.org/download/{}", identifier);

    // The XML URL is "{download_url_base}/{identifier}_files.xml"
    format!("{}/{}_files.xml", download_url_base, identifier)
}

/// Fetches and parses XML metadata from archive.org, with retry logic for network errors
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
    spinner: &indicatif::ProgressBar,
) -> Result<(XmlFiles, reqwest::Url)> {
    // Generate XML URL
    let xml_url = get_xml_url(details_url);
    spinner.set_message(format!(
        "{} Accessing XML metadata: {}",
        "⚙".blue(),
        xml_url.bold()
    ));

    // Check XML URL accessibility
    if let Err(e) = is_url_accessible(&xml_url, client).await {
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
                    if let Some(retry_after) = response.headers().get(reqwest::header::RETRY_AFTER) {
                        if let Ok(retry_after_str) = retry_after.to_str() {
                            if let Ok(secs) = retry_after_str.parse::<u64>() {
                                eprintln!("{} Rate limited (429). Waiting {}s before retry...", "▲".yellow(), secs);
                                tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
                                retries += 1;
                                continue;
                            }
                        }
                    }
                }
                if let Err(e) = response.error_for_status_ref() {
                    if retries < max_retries && is_transient_error(&e) {
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
                        let is_transient = is_transient_reqwest_error(&e);
                        if retries < max_retries && is_transient {
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

/// Command-line interface for ia-get
#[derive(Parser)]
#[command(name = "ia-get")]
#[command(about = "A command-line tool for downloading files from the Internet Archive")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(author = env!("CARGO_PKG_AUTHORS"))]
struct Cli {
    /// URL to an archive.org details page
    url: String,

    /// Output directory for downloaded files (default: identifier from URL)
    #[arg(short, long, value_name = "DIR")]
    output_path: Option<String>,

    /// Enable logging of files with failed hash verification to a file named 'Hash errors'
    #[arg(short = 'H', long = "hash")]
    log_hash_errors: bool,
}

/// Main application entry point
/// 
/// Parses command line arguments, validates the archive.org URL, checks URL accessibility,
/// downloads XML metadata, and initiates file downloads with built-in signal handling.
#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Set up signal handler for graceful shutdown (only once per program execution)
    let running = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let r = running.clone();
    
    // Use a static flag to ensure we only set the handler once
    use std::sync::Once;
    static SIGNAL_HANDLER_INIT: Once = Once::new();
    
    SIGNAL_HANDLER_INIT.call_once(|| {
        ctrlc::set_handler(move || {
            r.store(false, std::sync::atomic::Ordering::SeqCst);
            println!("\n{} Received Ctrl+C, finishing current operation...", "✘".red().bold());
        }).expect("Error setting Ctrl+C handler");
    });

    let mut url = cli.url.clone();
    // If the input is not a full URL, treat it as an identifier and construct the details URL
    if !url.starts_with("http://") && !url.starts_with("https://") {
        url = format!("https://archive.org/details/{}", url);
    }

    // Create a single client instance for all requests
    let client = Client::builder()
        .user_agent(USER_AGENT)
        .timeout(std::time::Duration::from_secs(HTTP_TIMEOUT))
        .build()?;

    // Start a single spinner for the entire initialization process
    let spinner = create_spinner(&format!("Processing archive.org URL: {}", url.bold()));

    // Validate URL format using consolidated function
    if let Err(e) = validate_archive_url(&url) {
        spinner.finish_with_message(format!("{} {}", "✘".red().bold(), e));
        return Err(e);
    }

    // Check URL accessibility
    if let Err(e) = is_url_accessible(&url, &client).await {
        spinner.finish_with_message(format!(
            "{} Archive.org URL not accessible: {}",
            "✘".red().bold(),
            url.bold()
        ));
        return Err(e); // Propagate error
    }

    // Fetch and parse XML metadata in one operation
    let (files, base_url) = fetch_xml_metadata(&url, &client, &spinner).await?;

    // Successfully finished initialization
    spinner.set_style(
        ProgressStyle::default_spinner()
            .template(&format!(
                "{} {} to download {} files from archive.org {}",
                "✔".green().bold(),
                "Ready".bold(),
                files.files.len().to_string().bold(),
                "★".yellow()
            ))
            .expect("Failed to set completion style"),
    );
    spinner.finish();

    use std::path::PathBuf;
    // Determine identifier from URL for default output directory
    let identifier = {
        let trimmed_url = url.trim_end_matches('/');
        trimmed_url.rsplit('/').next().unwrap_or("ia-get-download")
    };
    let output_dir = PathBuf::from(
        cli.output_path.clone().unwrap_or_else(|| identifier.to_string())
    );

    // Prepare download data for batch processing
    let download_data = files.files.into_iter().map(|file| {
        let mut absolute_url = base_url.clone();
        if let Ok(joined_url) = absolute_url.join(&file.name) {
            absolute_url = joined_url;
        }
        // Prepend output directory to file name
        let file_path = output_dir.join(&file.name);
        (absolute_url.to_string(), file_path.to_string_lossy().to_string(), file.md5)
    }).collect::<Vec<_>>();

    // Download all files with integrated signal handling and retry logic
    download_files_with_retries(&client, download_data.clone(), download_data.len(), cli.log_hash_errors, running.clone()).await?;

    // After batch, offer to retry errors, process new request, or exit
    use std::io::{self, Write};
    loop {
        println!("\nBatch complete. What would you like to do next?");
        println!("[R]etry failed downloads | [E]xit");
        print!("Enter choice (R/E): ");
        io::stdout().flush().ok();
        let mut input = String::new();
        io::stdin().read_line(&mut input).ok();
        let choice = input.trim().to_lowercase();
        match choice.as_str() {
            "r" => {
                // Retry failed downloads using batchlog.json only
                let batchlog_path = "batchlog.json";
                let batchlog: Vec<serde_json::Value> = match std::fs::read_to_string(batchlog_path)
                    .ok()
                    .and_then(|data| serde_json::from_str(&data).ok()) {
                    Some(entries) => entries,
                    None => {
                        println!("No failed downloads to retry.");
                        continue;
                    }
                };
                if batchlog.is_empty() {
                    println!("No failed downloads to retry.");
                    continue;
                }
                // Match by file name only
                let files_set: std::collections::HashSet<_> = batchlog.iter()
                    .filter_map(|entry| entry.get("file_path").and_then(|v| v.as_str()))
                    .filter_map(|f| std::path::Path::new(f).file_name().map(|n| n.to_string_lossy().to_string()))
                    .collect();
                let retry_data: Vec<_> = download_data.iter()
                    .filter(|(_, path, _)| {
                        std::path::Path::new(path).file_name().map(|n| files_set.contains(&n.to_string_lossy().to_string())).unwrap_or(false)
                    })
                    .cloned()
                    .collect();
                // Warn if any files in batchlog are not found in this session
                let download_names: std::collections::HashSet<_> = download_data.iter().filter_map(|(_, path, _)| std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().to_string())).collect();
                let missing: Vec<_> = files_set.difference(&download_names).collect();
                if !missing.is_empty() {
                    println!("Warning: The following files in the batch log are not part of this session and will not be retried:");
                    for m in &missing {
                        println!("  - {}", m);
                    }
                }
                if retry_data.is_empty() {
                    println!("No matching failed files found in this session.");
                    continue;
                }
                println!("Retrying {} failed downloads...", retry_data.len());
                // Clear the batchlog before retry
                let _ = std::fs::remove_file(batchlog_path);
                download_files_with_retries(&client, retry_data.clone(), retry_data.len(), cli.log_hash_errors, running.clone()).await?;
            }
            "e" => {
                println!("Exiting safely. Goodbye!");
                break;
            }
            _ => {
                println!("Invalid choice. Please enter R or E.");
            }
        }
    }
    Ok(())
}

// Wrapper to add retry logic to downloader::download_files for transient errors
async fn download_files_with_retries(
    client: &Client,
    download_data: Vec<(String, String, Option<String>)>,
    total_files: usize,
    log_hash_errors: bool,
    running: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<()> {
    let max_retries = 3;
    let mut retries = 0;
    let mut delay = std::time::Duration::from_secs(60);
    let max_delay = std::time::Duration::from_secs(900); // 15 minutes
    loop {
        // Extract output directory from the first file path  
        let result = downloader::download_files(client, download_data.clone(), total_files, log_hash_errors, running.clone()).await.map_err(|e| match e {
            IaGetError::Network(msg) => IaGetError::Network(msg),
            other => IaGetError::Network(format!("Other error: {}", other)),
        });
        match result {
            Ok(_) => return Ok(()),
            Err(e) => {
                // Only retry on transient network errors
                let is_transient = matches!(e, IaGetError::Network(_));
                if is_transient && retries < max_retries {
                    eprintln!("{} Network error: {}. Retrying in {}s...", "▲".yellow(), e, delay.as_secs());
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

#[cfg(test)]
mod tests {
    use super::*;
    use ia_get::utils::validate_archive_url;

    #[test]
    fn check_valid_pattern() {
        assert!(validate_archive_url("https://archive.org/details/Valid-Pattern").is_ok());
        assert!(validate_archive_url("https://archive.org/details/Valid-Pattern/").is_ok());
        assert!(validate_archive_url("https://archive.org/details/test123").is_ok());
        assert!(validate_archive_url("https://archive.org/details/test123/").is_ok());
        assert!(validate_archive_url("https://archive.org/details/test_file-name.data").is_ok());
        assert!(validate_archive_url("https://archive.org/details/test_file-name.data/").is_ok());
        assert!(validate_archive_url("https://archive.org/details/user@domain").is_ok());
        assert!(validate_archive_url("https://archive.org/details/user@domain/").is_ok());
    }

    #[test]
    fn check_invalid_pattern() {
        assert!(validate_archive_url("https://archive.org/details/Invalid-Pattern-*").is_err());
        assert!(validate_archive_url("https://archive.org/details/").is_err()); // This should still be an error (empty identifier)
        assert!(validate_archive_url("https://example.com/details/test").is_err());
        assert!(validate_archive_url("http://archive.org/details/test").is_err());
        assert!(validate_archive_url("https://archive.org/details/test/extra").is_err());
        assert!(validate_archive_url("https://archive.org/details/test//").is_err()); // Multiple trailing slashes
    }

    #[test]
    fn check_get_xml_url() {
        assert_eq!(
            get_xml_url("https://archive.org/details/item1"),
            "https://archive.org/download/item1/item1_files.xml"
        );
        assert_eq!(
            get_xml_url("https://archive.org/details/item1/"), // With trailing slash
            "https://archive.org/download/item1/item1_files.xml"
        );
        assert_eq!(
            get_xml_url("https://archive.org/details/another-item_v2.0"),
            "https://archive.org/download/another-item_v2.0/another-item_v2.0_files.xml"
        );
        assert_eq!(
            get_xml_url("https://archive.org/details/another-item_v2.0/"), // With trailing slash
            "https://archive.org/download/another-item_v2.0/another-item_v2.0_files.xml"
        );
    }
}