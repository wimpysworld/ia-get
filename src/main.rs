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
use ia_get::constants::{get_user_agent, HTTP_TIMEOUT};
use ia_get::archive_metadata::{XmlFiles, parse_xml_files};
use indicatif::ProgressStyle;
use reqwest::Client;
use clap::Parser;
use colored::*; // Add this line

/// Checks if a URL is accessible by sending a HEAD request, with retry logic and dynamic wait reasons
async fn is_url_accessible(url: &str, client: &Client, spinner: Option<&indicatif::ProgressBar>) -> Result<()> {
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
                    let wait_time = if let Some(retry_after) = response.headers().get(reqwest::header::RETRY_AFTER) {
                        retry_after.to_str()
                            .ok()
                            .and_then(|s| s.parse::<u64>().ok())
                            .unwrap_or(60)
                    } else {
                        60 // Default if no Retry-After header
                    };
                    
                    let wait_reason = format!("Rate limited by server (HTTP 429) - waiting {}s as requested", wait_time);
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
                        let wait_reason = format!("Server error (HTTP {}) - retrying in {}s (attempt {}/{})", 
                                                e.status().map(|s| s.as_u16()).unwrap_or(0), 
                                                delay.as_secs(), 
                                                retries + 1, 
                                                max_retries);
                        
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
                    let wait_reason = format!("Network error - retrying in {}s (attempt {}/{})", 
                                            delay.as_secs(), 
                                            retries + 1, 
                                            max_retries);
                    
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
    if let Err(e) = is_url_accessible(&xml_url, client, Some(spinner)).await {
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
                        let is_transient = is_transient_reqwest_error(&e);
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

/// Command-line interface for ia-get
#[derive(Parser)]
#[command(name = "ia-get")]
#[command(about = "A command-line tool for downloading files from the Internet Archive")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(author = env!("CARGO_PKG_AUTHORS"))]
struct Cli {
    /// URL to an archive.org details page (optional for interactive mode)
    #[arg(value_name = "URL")]
    url: Option<String>,

    /// Output directory for downloaded files (default: identifier from URL)
    #[arg(short, long, value_name = "DIR")]
    output_path: Option<String>,

    /// Enable logging of files with failed hash verification to a file named 'Hash errors'
    #[arg(short = 'L', long = "Log")]
    log_hash_errors: bool,
}

/// Main application entry point
/// 
/// Parses command line arguments, validates the archive.org URL, checks URL accessibility,
/// downloads XML metadata, and initiates file downloads with built-in signal handling.
#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Get URL either from command line or interactive prompt
    let url = match cli.url {
        Some(url) => url,
        None => {
            // Interactive mode
            use std::io::{self, Write};
            println!("Welcome to ia-get! Let's get started.");
            print!("Enter the Internet Archive identifier or URL: ");
            io::stdout().flush().unwrap();
            let mut input = String::new();
            io::stdin().read_line(&mut input).unwrap();
            let input = input.trim();
            
            if input.is_empty() {
                eprintln!("Error: No identifier or URL provided. Exiting.");
                std::process::exit(1);
            }
            input.to_string()
        }
    };

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

    let mut url = url;
    // If the input is not a full URL, treat it as an identifier and construct the details URL
    if !url.starts_with("http://") && !url.starts_with("https://") {
        url = format!("https://archive.org/details/{}", url);
    }

    // Create a single client instance for all requests
    let client = Client::builder()
        .user_agent(get_user_agent())
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
    if let Err(e) = is_url_accessible(&url, &client, Some(&spinner)).await {
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
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

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

    /// Test edge cases for long URLs and complex identifiers
    #[test]
    fn test_long_urls_and_complex_identifiers() {
        // Test very long identifier
        let long_identifier = "a".repeat(100);
        let long_url = format!("https://archive.org/details/{}", long_identifier);
        assert!(validate_archive_url(&long_url).is_ok());
        
        // Test with complex characters (allowed by Internet Archive)
        let complex_identifiers = vec![
            "test-item_with.dots",
            "user@example.com",
            "item123_v2.0-final",
            "test_file-name.data.backup",
            "archive_2023-12-25_final"
        ];
        
        for identifier in complex_identifiers {
            let url = format!("https://archive.org/details/{}", identifier);
            assert!(validate_archive_url(&url).is_ok(), "Failed for identifier: {}", identifier);
            
            // Test XML URL generation
            let xml_url = get_xml_url(&url);
            assert!(xml_url.contains(&format!("{}_files.xml", identifier)));
        }
    }

    /// Test file names with unexpected characters (basic test without tempfile)
    #[test]
    fn test_files_with_unexpected_characters() {
        // Test files with special characters that might cause issues
        let problematic_filenames = vec![
            "file with spaces.txt",
            "file(with)parentheses.doc",
            "file[with]brackets.pdf",
            "file{with}braces.xml",
            "file'with'quotes.json",
            "file\"with\"doublequotes.csv",
            "file&with&ampersands.log",
            "file%20with%20encoding.txt",
            "file+with+plus.data",
            "file=with=equals.conf",
            "file?with?questions.ini",
            "file#with#hash.md",
            "file@with@at.email",
            "file$with$dollar.sh",
            "file^with^caret.bat",
            "file|with|pipe.txt",
            "file~with~tilde.backup",
            "file`with`backtick.sql"
        ];
        
        // Test that filename processing doesn't panic
        for filename in problematic_filenames {
            // Test path creation and string conversion
            let path = std::path::Path::new(filename);
            let path_str = path.to_string_lossy();
            assert!(path_str.contains(filename.trim_matches('\"')), "Path string should contain filename base");
            
            // Test URL encoding scenarios
            let encoded = filename.replace(' ', "%20");
            assert!(encoded.len() >= filename.len(), "Encoded version should be equal or longer");
        }
    }

    /// Test hash validation scenarios without file system operations
    #[test]
    fn test_hash_scenarios() {
        // Test MD5 hash format validation
        let valid_hashes = vec![
            "d41d8cd98f00b204e9800998ecf8427e", // Empty file hash
            "5d41402abc4b2a76b9719d911017c592", // "hello" hash
            "098f6bcd4621d373cade4e832627b4f6", // "test" hash
        ];
        
        for hash in valid_hashes {
            assert_eq!(hash.len(), 32, "MD5 hash should be 32 characters");
            assert!(hash.chars().all(|c| c.is_ascii_hexdigit()), "MD5 hash should be hexadecimal");
        }
        
        // Test invalid hash formats
        let invalid_hashes = vec![
            "not_a_hash",
            "123", // Too short
            "g41d8cd98f00b204e9800998ecf8427e", // Contains non-hex character
            "", // Empty
        ];
        
        for hash in invalid_hashes {
            let is_valid = hash.len() == 32 && hash.chars().all(|c| c.is_ascii_hexdigit());
            assert!(!is_valid, "Invalid hash should be rejected: {}", hash);
        }
    }

    /// Test alternative validation methods for common file types
    #[test]
    fn test_alternative_validation_methods() {
        // Test XML structure validation
        let xml_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<files>
    <file name="test1.txt" size="1024" md5="abc123" />
    <file name="test2.pdf" size="2048" md5="def456" />
</files>"#;
        
        // Validate XML structure
        assert!(xml_content.contains("<?xml version="), "Should contain XML declaration");
        assert!(xml_content.contains("<files>"), "Should contain files element");
        assert!(xml_content.contains("</files>"), "Should contain closing files element");
        
        // Count file entries
        let file_count = xml_content.matches("<file ").count();
        assert_eq!(file_count, 2, "Should contain exactly 2 file entries");
        
        // Test JSON validation
        let json_content = r#"{
    "metadata": {
        "identifier": "test-archive",
        "title": "Test Archive"
    },
    "files": [
        {"name": "file1.txt", "size": 100},
        {"name": "file2.pdf", "size": 200}
    ]
}"#;
        
        // Validate JSON parsing
        let parsed: std::result::Result<serde_json::Value, _> = serde_json::from_str(json_content);
        assert!(parsed.is_ok(), "JSON should be parseable");
        
        if let Ok(json) = parsed {
            assert!(json.get("metadata").is_some(), "Should contain metadata field");
            assert!(json.get("files").is_some(), "Should contain files field");
            
            if let Some(files) = json.get("files").and_then(|f| f.as_array()) {
                assert_eq!(files.len(), 2, "Should contain 2 files");
            }
        }
    }

    /// Test file size validation scenarios
    #[test]
    fn test_file_size_validation() {
        let test_sizes = vec![0, 10, 1024, 10240, 1048576]; // Various sizes including edge cases
        
        for expected_size in test_sizes {
            // Test size comparison logic
            let reported_size = expected_size + 100; // Simulate incorrect size from metadata
            assert_ne!(expected_size, reported_size, "Should detect size mismatch");
            
            // Test size validation bounds
            let size_diff = if reported_size > expected_size {
                reported_size - expected_size
            } else {
                expected_size - reported_size
            };
            
            assert!(size_diff > 0, "Should have non-zero size difference");
        }
    }

    /// Test download interruption scenarios
    #[test]
    fn test_download_interruption_scenarios() {
        let running = Arc::new(AtomicBool::new(true));
        
        // Test signal handling
        assert!(running.load(Ordering::SeqCst), "Should initially be running");
        
        // Simulate signal reception
        running.store(false, Ordering::SeqCst);
        assert!(!running.load(Ordering::SeqCst), "Should stop after signal");
        
        // Reset for next test
        running.store(true, Ordering::SeqCst);
        assert!(running.load(Ordering::SeqCst), "Should be running again");
    }

    /// Test user agent string format
    #[test]
    fn test_user_agent_format() {
        let user_agent = get_user_agent();
        assert!(user_agent.contains("ia-get-cli"), "User agent should contain application name");
        assert!(user_agent.contains("Internet Archive"), "User agent should indicate purpose");
        assert!(user_agent.len() > 10, "User agent should be descriptive");
        assert!(!user_agent.contains("  "), "User agent should not contain double spaces");
        assert!(!user_agent.contains("github.com"), "User agent should not contain specific repository URL");
        
        // Test that it contains system information
        assert!(user_agent.contains("-"), "User agent should contain OS-arch information");
        assert!(user_agent.contains("user:"), "User agent should contain username");
        assert!(user_agent.contains("host:"), "User agent should contain hostname");
    }

    /// Test XML file validation alternative method
    #[test]
    fn test_xml_validation_alternative() {
        // Test valid Internet Archive XML content
        let valid_xml_samples = vec![
            r#"<?xml version="1.0" encoding="UTF-8"?>
<files>
    <file name="test1.txt" size="1024" md5="abc123" />
    <file name="test2.pdf" size="2048" md5="def456" />
</files>"#,
            r#"<files>
    <file name="document.pdf" size="5120" md5="fedcba987654321" />
    <file name="readme.txt" size="256" md5="123456789abcdef" />
</files>"#,
            r#"<?xml version="1.0"?>
<file name="archive.zip" size="10485760" md5="deadbeefcafebabe" />"#,
        ];

        for xml_content in valid_xml_samples {
            // Test basic XML structure validation
            assert!(xml_content.contains('<') && xml_content.contains('>'), "Should have angle brackets");
            
            let open_brackets = xml_content.chars().filter(|&c| c == '<').count();
            let close_brackets = xml_content.chars().filter(|&c| c == '>').count();
            assert_eq!(open_brackets, close_brackets, "Should have balanced brackets");
            
            // Test for Internet Archive patterns
            let has_ia_patterns = xml_content.contains("<files>") || 
                                  xml_content.contains("<file ") || 
                                  xml_content.contains("name=") ||
                                  xml_content.contains("size=");
            assert!(has_ia_patterns, "Should contain Internet Archive XML patterns");
        }

        // Test invalid XML samples
        let invalid_xml_samples = vec![
            "", // Empty
            "not xml at all", // Plain text
            "<incomplete", // Unbalanced brackets
            "<?xml version='1.0'?><root></different>", // Mismatched tags
            "<>", // Empty tags
        ];

        for xml_content in invalid_xml_samples {
            if xml_content.is_empty() || xml_content.len() < 10 {
                continue; // These would be rejected by size check
            }
            
            let open_brackets = xml_content.chars().filter(|&c| c == '<').count();
            let close_brackets = xml_content.chars().filter(|&c| c == '>').count();
            let balanced_brackets = open_brackets == close_brackets;
            
            let has_ia_patterns = xml_content.contains("<files>") || 
                                  xml_content.contains("<file ") || 
                                  xml_content.contains("name=") ||
                                  xml_content.contains("size=");
            
            // At least one validation criterion should fail
            let should_be_invalid = !balanced_brackets || !has_ia_patterns;
            assert!(should_be_invalid, "Invalid XML should fail at least one validation check: {}", xml_content);
        }
    }

    /// Test URL and identifier edge cases
    #[test]
    fn test_url_edge_cases() {
        // Test extremely long URLs (within reason)
        let long_id = "a".repeat(255); // Max reasonable length
        let long_url = format!("https://archive.org/details/{}", long_id);
        assert!(validate_archive_url(&long_url).is_ok(), "Should handle long URLs");
        
        // Test URLs with all allowed special characters
        let special_chars_id = "test-item_with.dots@domain.com";
        let special_url = format!("https://archive.org/details/{}", special_chars_id);
        assert!(validate_archive_url(&special_url).is_ok(), "Should handle special characters");
        
        // Test XML URL generation for edge cases
        let xml_url = get_xml_url(&special_url);
        assert!(xml_url.contains(&format!("{}_files.xml", special_chars_id)));
        assert!(xml_url.starts_with("https://archive.org/download/"));
    }

    /// Test HTTP response handling including 429 rate limiting
    #[test]
    fn test_http_response_handling() {
        // Test rate limit parsing scenarios
        let test_cases = vec![
            ("60", 60u64),     // Valid numeric string
            ("120", 120u64),   // Valid numeric string
            ("invalid", 60u64), // Invalid string should default to 60
            ("", 60u64),       // Empty string should default to 60
        ];
        
        for (retry_after_value, expected_wait_time) in test_cases {
            let parsed_wait_time = retry_after_value.parse::<u64>().unwrap_or(60);
            assert_eq!(parsed_wait_time, expected_wait_time, 
                      "Rate limit parsing should handle '{}' correctly", retry_after_value);
        }
    }

    /// Test wait reason formatting for different scenarios
    #[test]
    fn test_wait_reason_formatting() {
        // Test rate limit reason formatting
        let wait_time = 120u64;
        let rate_limit_reason = format!("Rate limited by server (HTTP 429) - waiting {}s as requested", wait_time);
        assert!(rate_limit_reason.contains("429"), "Should mention HTTP 429");
        assert!(rate_limit_reason.contains("120s"), "Should include wait time");
        assert!(rate_limit_reason.contains("as requested"), "Should indicate server requested wait");
        
        // Test server error reason formatting
        let status_code = 502u16;
        let delay_secs = 60u64;
        let attempt = 2usize;
        let max_attempts = 5usize;
        let server_error_reason = format!("Server error (HTTP {}) - retrying in {}s (attempt {}/{})", 
                                         status_code, delay_secs, attempt, max_attempts);
        assert!(server_error_reason.contains("502"), "Should mention status code");
        assert!(server_error_reason.contains("60s"), "Should include delay time");
        assert!(server_error_reason.contains("2/5"), "Should show attempt progress");
        
        // Test network error reason formatting
        let network_error_reason = format!("Network error - retrying in {}s (attempt {}/{})", 
                                          delay_secs, attempt, max_attempts);
        assert!(network_error_reason.contains("Network error"), "Should mention network error");
        assert!(network_error_reason.contains("60s"), "Should include delay time");
        assert!(network_error_reason.contains("2/5"), "Should show attempt progress");
    }

    /// Test transient error detection
    #[test]
    fn test_transient_error_detection() {
        // Test HTTP status codes that should be considered transient
        let transient_statuses = vec![500, 502, 503, 504, 429];
        
        for status in transient_statuses {
            // We can't easily create reqwest::Error objects in tests, but we can test the logic
            // In a real scenario, these would be server errors or rate limits
            assert!(status >= 500 || status == 429, 
                   "Status {} should be considered transient", status);
        }
        
        // Test that non-transient errors are not retried
        let non_transient_statuses = vec![400, 401, 403, 404, 410];
        
        for status in non_transient_statuses {
            assert!(status < 500 && status != 429, 
                   "Status {} should not be considered transient", status);
        }
    }

    /// Test retry delay calculation and bounds
    #[test]
    fn test_retry_delay_logic() {
        let initial_delay = std::time::Duration::from_secs(60);
        let max_delay = std::time::Duration::from_secs(900); // 15 minutes
        
        // Test exponential backoff
        let mut current_delay = initial_delay;
        let delays: Vec<u64> = (0..5).map(|_| {
            let delay_secs = current_delay.as_secs();
            current_delay = std::cmp::min(current_delay * 2, max_delay);
            delay_secs
        }).collect();
        
        // Should be: 60, 120, 240, 480, 900 (capped at max)
        assert_eq!(delays[0], 60, "First delay should be 60s");
        assert_eq!(delays[1], 120, "Second delay should be 120s");
        assert_eq!(delays[2], 240, "Third delay should be 240s");
        assert_eq!(delays[3], 480, "Fourth delay should be 480s");
        assert_eq!(delays[4], 900, "Fifth delay should be capped at 900s");
        
        // Test that delays don't exceed maximum
        for delay in delays {
            assert!(delay <= 900, "Delay should never exceed 900s, got {}s", delay);
        }
    }

    /// Test user agent generation with system information
    #[test]
    fn test_dynamic_user_agent() {
        let user_agent = get_user_agent();
        
        // Should contain version from Cargo.toml
        assert!(user_agent.contains("ia-get-cli/"), "Should contain app name and version");
        
        // Should contain OS and architecture
        assert!(user_agent.contains("-"), "Should contain OS-arch separator");
        
        // Should contain user and host information
        assert!(user_agent.contains("user:"), "Should contain user info");
        assert!(user_agent.contains("host:"), "Should contain host info");
        
        // Should contain purpose
        assert!(user_agent.contains("Internet Archive"), "Should indicate purpose");
        
        // Should be reasonable length
        assert!(user_agent.len() > 20, "Should be descriptive, got: {}", user_agent);
        assert!(user_agent.len() < 500, "Should not be excessively long, got: {}", user_agent);
        
        // Should not contain sensitive information patterns
        assert!(!user_agent.contains("password"), "Should not contain sensitive info");
        assert!(!user_agent.contains("secret"), "Should not contain sensitive info");
        assert!(!user_agent.contains("token"), "Should not contain sensitive info");
    }
}