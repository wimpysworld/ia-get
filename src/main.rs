//! # ia-get
//!
//! A command-line tool for downloading files from the Internet Archive.
//! 
//! This tool takes an archive.org details URL and downloads all associated files,
//! with support for resumable downloads and MD5 hash verification.

use ia_get::{IaGetError, Result};
use indicatif::{ProgressBar, ProgressStyle};
use regex::Regex;
use reqwest::header::{HeaderValue, HeaderMap};
use reqwest::Client;
use serde::Deserialize;
use serde_xml_rs::from_str;
use clap::Parser;
use std::fs::{self, File};
use std::io::{BufReader, Read, Write}; // Removed unused Seek import
use std::process;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use md5;

/// Buffer size for file operations (8KB)
const BUFFER_SIZE: usize = 8192;

/// File size threshold for showing hash progress bar (16MB)
const LARGE_FILE_THRESHOLD: u64 = 16 * 1024 * 1024;

/// User agent string for HTTP requests
const USER_AGENT: &str = "ia-get";

/// Default timeout for HTTP requests in seconds
const DEFAULT_HTTP_TIMEOUT: u64 = 60;

/// Timeout for URL accessibility checks in seconds
const URL_CHECK_TIMEOUT: u64 = 30;

/// Spinner tick interval in milliseconds
const SPINNER_TICK_INTERVAL: u64 = 100;

/// Regex pattern for validating archive.org details URLs
const PATTERN: &str = r"^https://archive\.org/details/[a-zA-Z0-9_\-.@]+$";

/// Root structure for parsing the XML files list from archive.org
/// The actual XML structure has a `files` root element containing multiple `file` elements
#[derive(Deserialize, Debug)]
struct XmlFiles {
    #[serde(rename = "file", default)]
    files: Vec<XmlFile>,
}

/// Represents a single file entry from the archive.org XML metadata
/// 
/// Archive.org XML structure has both attributes and nested elements:
/// ```xml
/// <file name="..." source="...">
///   <mtime>...</mtime>
///   <size>...</size>
///   <md5>...</md5>
///   ...
/// </file>
/// ```
#[allow(dead_code)]
#[derive(Deserialize, Debug)]
struct XmlFile {
    #[serde(rename = "@name")]
    name: String,
    #[serde(rename = "@source")]
    source: String,
    mtime: Option<u64>,
    size: Option<u64>,
    format: Option<String>,
    rotation: Option<u32>,
    md5: Option<String>,
    crc32: Option<String>,
    sha1: Option<String>,
    btih: Option<String>,
    summation: Option<String>,
    original: Option<String>,
}

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

/// Create a progress bar with consistent styling
/// 
/// # Arguments
/// * `total` - Total value for the progress bar
/// * `action` - Action text to show at the beginning (e.g., "╰╼ Downloading  ")
/// * `color` - Optional color style (defaults to "green/green")
/// * `with_eta` - Whether to include ETA in the template
/// 
/// # Returns
/// A configured progress bar
fn create_progress_bar(total: u64, action: &str, color: Option<&str>, with_eta: bool) -> ProgressBar {
    let pb = ProgressBar::new(total);
    let color_str = color.unwrap_or("green/green");
    
    let template = if with_eta {
        format!("{action}{{elapsed_precise}}     {{bar:40.{color_str}}} {{bytes}}/{{total_bytes}} (ETA: {{eta}})")
    } else {
        format!("{action}{{elapsed_precise}}     {{bar:40.{color_str}}} {{bytes}}/{{total_bytes}}")
    };
    
    pb.set_style(
        ProgressStyle::default_bar()
            .template(&template)
            .expect("Failed to set progress bar style")
            .progress_chars("▓▒░"),
    );
    
    pb
}

/// Create a spinner with braille animation
/// 
/// # Arguments
/// * `message` - Message to display next to the spinner
/// 
/// # Returns
/// A configured spinner
fn create_spinner(message: &str) -> ProgressBar {
    let spinner = ProgressBar::new_spinner();
    spinner.set_style(
        ProgressStyle::default_spinner()
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏")
            .template(&format!("{{spinner}} {message}"))
            .expect("Failed to set spinner style")
    );
    spinner.enable_steady_tick(std::time::Duration::from_millis(SPINNER_TICK_INTERVAL));
    spinner
}

/// Calculates the MD5 hash of a file
/// 
/// Uses a streaming approach to compute the MD5 hash by reading the file in chunks,
/// which is memory-efficient for large files. This approach avoids loading the entire
/// file into memory at once.
/// 
/// # Arguments
/// * `file_path` - Path to the file to hash
/// * `running` - Signal handler to check for interruption
/// 
/// # Returns
/// * `Ok(String)` - The MD5 hash as a lowercase hexadecimal string
/// * `Err(IaGetError)` - If the file cannot be read
fn calculate_md5(file_path: &str, running: &Arc<AtomicBool>) -> Result<String> {
    let file = File::open(file_path)?;
    let file_size = file.metadata()?.len();
    let is_large_file = file_size > LARGE_FILE_THRESHOLD;
    
    let mut reader = BufReader::with_capacity(BUFFER_SIZE, file);
    let mut context = md5::Context::new();
    let mut buffer = [0; BUFFER_SIZE];
    
    // Only show progress bar for large files to avoid UI clutter
    let pb = if is_large_file {
        let progress_bar = create_progress_bar(file_size, "╰╼ Hashing      ", Some("cyan/blue"), false);
        Some(progress_bar)
    } else {
        None
    };
    
    let mut bytes_processed: u64 = 0;
    
    loop {
        let bytes_read = reader.read(&mut buffer)?;
        if bytes_read == 0 {
            // End of file reached
            break;
        }
        
        // Check for signal interruption during hash calculation
        if !running.load(Ordering::SeqCst) {
            if let Some(ref progress_bar) = pb {
                progress_bar.finish_and_clear();
            }
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "Hash calculation interrupted by signal",
            ).into());
        }
        
        context.consume(&buffer[..bytes_read]);
        
        // Update progress if we're showing it
        if let Some(ref progress_bar) = pb {
            bytes_processed += bytes_read as u64;
            progress_bar.set_position(bytes_processed);
        }
    }
    
    // Finalize progress bar if it exists
    if let Some(progress_bar) = pb {
        progress_bar.finish_and_clear();
    }
    
    let hash = context.compute();
    Ok(format!("{:x}", hash))
}

/// Check if an existing file has the correct hash
///
/// # Arguments
/// * `file_path` - Path to the file to check
/// * `expected_md5` - Expected MD5 hash, if available
/// * `running` - Signal handler to check for interruption
///
/// # Returns
/// * `Ok(Some(bool))` - Whether the file exists and its hash matches or not
/// * `Ok(None)` - If the file doesn't exist
/// * `Err(IaGetError)` - If there was an error checking the file
fn check_existing_file(file_path: &str, expected_md5: Option<&str>, running: &Arc<AtomicBool>) -> Result<Option<bool>> {
    if !Path::new(file_path).exists() {
        return Ok(None);
    }

    if expected_md5.is_none() {
        return Ok(Some(true)); // No MD5 to check against, assume file is valid
    }

    // Calculate the MD5 hash of the local file
    let local_md5 = match calculate_md5(file_path, running) {
        Ok(hash) => hash,
        Err(e) => {
            // Check if this is an interruption by looking at the error message
            if e.to_string().contains("interrupted by signal") {
                return Err(e);
            }
            println!("╰╼ Failed to calculate MD5 hash: {}", e);
            return Ok(Some(false));
        }
    };

    Ok(Some(local_md5 == expected_md5.unwrap()))
}

/// Ensure parent directories exist for a file
///
/// # Arguments
/// * `file_path` - Path to the file
///
/// # Returns
/// * `Ok(())` - If directories were created or already exist
/// * `Err(IaGetError)` - If directories couldn't be created
fn ensure_parent_directories(file_path: &str) -> Result<()> {
    // Check if file_name includes a path
    if let Some(path) = Path::new(file_path).parent() {
        // Create the local directory if it doesn't exist and path has a file name
        if path.file_name().is_some() && !path.exists() {
            fs::create_dir_all(path)?;
        }
    }
    Ok(())
}

/// Prepare a file for download, potentially resuming an existing download
///
/// # Arguments
/// * `file_path` - Path to the file
///
/// # Returns
/// * `Ok(File)` - Open file handle ready for writing
/// * `Err(IaGetError)` - If the file couldn't be opened
fn prepare_file_for_download(file_path: &str) -> Result<File> {
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(file_path)?;
    
    Ok(file)
}

/// Download file content with progress reporting
///
/// # Arguments
/// * `client` - HTTP client
/// * `url` - URL to download from
/// * `file_size` - Current size of local file (for resuming)
/// * `file` - Open file handle to write to
/// * `running` - Signal handler to check for interruption
/// * `is_resuming` - Whether this is a resumed download or fresh download
///
/// # Returns
/// * `Ok(u64)` - Total bytes downloaded
/// * `Err(IaGetError)` - If download failed
async fn download_file_content(
    client: &Client, 
    url: &str, 
    file_size: u64, 
    file: &mut File,
    running: &Arc<AtomicBool>,
    is_resuming: bool
) -> Result<u64> {
    let download_action = if is_resuming { "╰╼ Resuming     " } else { "╰╼ Downloading  " };
    
    // Set the Range header to specify the starting offset
    let mut initial_request = client.get(url);
    let range_header = format!("bytes={}-", file_size);
    let mut headers = HeaderMap::new();
    // Fixed: Handle the HeaderValue::from_str error explicitly
    headers.insert(
        reqwest::header::RANGE, 
        HeaderValue::from_str(&range_header).map_err(|e| IaGetError::UrlError(format!("Invalid header value: {}", e)))?
    );
    initial_request = initial_request.headers(headers);

    let mut response = initial_request.send().await?;

    // Get the content length from the response headers
    let content_length = response.content_length().unwrap_or(0);
    let pb = create_progress_bar(
        content_length + file_size,
        download_action,
        None, // Default to green/green
        true  // Include ETA
    );

    // Record start time to calculate transfer rate later
    let start_time = std::time::Instant::now();

    // Download the remaining chunks and update the progress bar
    let mut total_bytes: u64 = file_size;
    let mut downloaded_bytes: u64 = 0;
    
    while let Some(chunk) = response.chunk().await? {
        // Check for signal interruption during download
        if !running.load(Ordering::SeqCst) {
            pb.finish_and_clear();
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "Download interrupted during file transfer",
            ).into());
        }
        
        file.write_all(&chunk)?;
        downloaded_bytes += chunk.len() as u64;
        total_bytes += chunk.len() as u64;
        pb.set_position(total_bytes);
    }

    // Calculate elapsed time and transfer rate
    let elapsed = start_time.elapsed();
    let elapsed_secs = elapsed.as_secs_f64();
    
    // Calculate transfer rate (bytes per second)
    let transfer_rate = if elapsed_secs > 0.0 {
        downloaded_bytes as f64 / elapsed_secs
    } else {
        0.0 // Avoid division by zero
    };
    
    // Format the transfer rate
    let (rate, unit) = format_transfer_rate(transfer_rate);
    
    // Clear the progress bar and show completion details
    pb.finish_and_clear();
    
    // Show completion message with size, time and speed
    println!("├╼ Downloaded   ⤵️ {} in {} ({:.2} {}/s)", 
        format_size(downloaded_bytes),
        format_duration(elapsed),
        rate,
        unit);

    
    Ok(total_bytes)
}

/// Format a duration into a human-readable string
fn format_duration(duration: std::time::Duration) -> String {
    let total_secs = duration.as_secs();
    if total_secs < 60 {
        return format!("{}.{:02}s", total_secs, duration.subsec_millis() / 10);
    }
    
    let hours = total_secs / 3600;
    let mins = (total_secs % 3600) / 60;
    let secs = total_secs % 60;
    
    if hours > 0 {
        format!("{}h {}m {}s", hours, mins, secs)
    } else {
        format!("{}m {}s", mins, secs)
    }
}

/// Format a size in bytes to a human-readable string
fn format_size(size: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    
    if size < KB {
        format!("{}B", size)
    } else if size < MB {
        format!("{:.2}KB", size as f64 / KB as f64)
    } else if size < GB {
        format!("{:.2}MB", size as f64 / MB as f64)
    } else {
        format!("{:.2}GB", size as f64 / GB as f64)
    }
}

/// Format transfer rate to appropriate units
fn format_transfer_rate(bytes_per_sec: f64) -> (f64, &'static str) {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    
    if bytes_per_sec < KB {
        (bytes_per_sec, "B")
    } else if bytes_per_sec < MB {
        (bytes_per_sec / KB, "KB")
    } else if bytes_per_sec < GB {
        (bytes_per_sec / MB, "MB")
    } else {
        (bytes_per_sec / GB, "GB")
    }
}

/// Verify a downloaded file's hash against an expected value
///
/// # Arguments
/// * `file_path` - Path to the file
/// * `expected_md5` - Expected MD5 hash, if available
/// * `running` - Signal handler to check for interruption
///
/// # Returns
/// * `Ok(bool)` - Whether the hash matches
/// * `Err(IaGetError)` - If verification failed
fn verify_downloaded_file(file_path: &str, expected_md5: Option<&str>, running: &Arc<AtomicBool>) -> Result<bool> {   
    // Calculate the MD5 hash of the local file
    let local_md5 = match calculate_md5(file_path, running) {
        Ok(hash) => hash,
        Err(e) => {
            println!("╰╼ Failed to calculate MD5 hash: {}", e);
            return Ok(false);
        }
    };
    
    match expected_md5 {
        Some(expected) => {
            let matches = local_md5 == expected;
            if !matches {
                println!("╰╼ Hash         ❌");
            } else {
                println!("╰╼ Hash         ✅");
            }
            Ok(matches)
        },
        None => {
            println!("╰╼ No MD5:      ⚠️");
            Ok(true) // Consider it valid if no expected hash
        },
    }
}

/// Download a file from archive.org with resume capability
///
/// # Arguments
/// * `client` - HTTP client
/// * `url` - URL to download from
/// * `file_path` - Path to save the file
/// * `expected_md5` - Expected MD5 hash, if available
/// * `running` - Signal handler to check for interruption
///
/// # Returns
/// * `Ok(())` - If download was successful or file already exists with correct hash
/// * `Err(IaGetError)` - If download failed
async fn download_file(
    client: &Client,
    url: &str,
    file_path: &str,
    expected_md5: Option<&str>,
    running: &Arc<AtomicBool>
) -> Result<()> {
    println!(" ");
    println!("📦️ Filename     {}", file_path);
    
    // Check if the file exists and has correct hash
    if let Some(is_valid) = check_existing_file(file_path, expected_md5, running)? {
        if is_valid {
            println!("╰╼ Completed:   ✅");
            return Ok(());
        }
    }
    
    // Create directories if needed
    ensure_parent_directories(file_path)?;
    
    // Open the file for writing/appending
    let mut file = prepare_file_for_download(file_path)?;
    
    // Get the size of the local file if it already exists
    let file_size = file.metadata()?.len();
    let is_resuming = file_size > 0;
    
    // Download the file content
    download_file_content(client, url, file_size, &mut file, running, is_resuming).await?;
    
    // Verify the downloaded file
    verify_downloaded_file(file_path, expected_md5, running)?;
    
    Ok(())
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
        download_file(
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
