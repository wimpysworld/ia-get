//! # ia-get
//!
//! A command-line tool for downloading files from the Internet Archive.
//! 
//! This tool takes an archive.org details URL and downloads all associated files,
//! with support for resumable downloads and MD5 hash verification.

use indicatif::{ProgressBar, ProgressStyle};
use regex::Regex;
use reqwest::header::{HeaderValue, HeaderMap};
use reqwest::Client;
use serde::Deserialize;
use serde_xml_rs::from_str;
use clap::Parser;
use std::error::Error;
use std::fs::{self, File};
use std::io::{BufReader, Read, Seek, Write};
use std::process;
use std::path::Path;
use md5;

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
async fn is_url_accessible(url: &str) -> Result<(), Box<dyn Error>> {
    let client = reqwest::Client::builder()
        .user_agent("ia-get")
        .build()?;
    let response = client.head(url).send().await?;
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

/// Calculates the MD5 hash of a file
/// 
/// Uses a streaming approach to compute the MD5 hash by reading the file in chunks,
/// which is memory-efficient for large files. This approach avoids loading the entire
/// file into memory at once.
/// 
/// # Arguments
/// * `file_path` - Path to the file to hash
/// 
/// # Returns
/// * `Ok(String)` - The MD5 hash as a lowercase hexadecimal string
/// * `Err(std::io::Error)` - If the file cannot be read
fn calculate_md5(file_path: &str) -> Result<String, std::io::Error> {
    let file = File::open(file_path)?;
    let file_size = file.metadata()?.len();
    let is_large_file = file_size > 100 * 1024 * 1024; // 100 MB threshold
    
    let mut reader = BufReader::with_capacity(8192, file);
    let mut context = md5::Context::new();
    let mut buffer = [0; 8192]; // 8KB buffer size
    
    // Only show progress bar for large files to avoid UI clutter
    let pb = if is_large_file {
        let progress_bar = ProgressBar::new(file_size);
        progress_bar.set_style(
            ProgressStyle::default_bar()
                .template("╰╼ Hashing      {elapsed_precise}     {bar:40.cyan/blue} {bytes}/{total_bytes}")
                .expect("Failed to set progress bar style")
                .progress_chars("▓▒░"),
        );
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

/// Define the regular expression pattern for the expected format as a static constant
static PATTERN: &str = r"^https://archive\.org/details/[a-zA-Z0-9_\-.@]+$";

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
/// downloads XML metadata, and iterates through files to download them with resume capability
/// and hash verification.
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    
    let client = Client::builder()
        .user_agent("ia-get")
        .build()?;

    // Create a regex object with the static pattern
    let regex = Regex::new(PATTERN)?;

    println!("Archive.org URL: {}", cli.url);
    if !regex.is_match(&cli.url) {
        println!("├╼ Archive.org URL is not in the expected format");
        println!("╰╼ Expected format: https://archive.org/details/<identifier>/");
        process::exit(1);
    }

    match is_url_accessible(&cli.url).await {
        Ok(()) => println!("╰╼ Archive.org URL online: 🟢"),
        Err(e) => {
            println!("├╼ Archive.org URL online: 🔴");
            eprintln!("╰╼ Exiting due to error: {}", e);
            process::exit(1);
        }
    }

    let xml_url = get_xml_url(&cli.url);
    println!("Archive.org XML: {}", xml_url);

    match is_url_accessible(&xml_url).await {
        Ok(()) => println!("├╼ Archive.org XML online: 🟢"),
        Err(e) => {
            println!("├╼ Archive.org XML online: 🔴");
            eprintln!("╰╼ Exiting due to error: {}", e);
            process::exit(1);
        }
    }

    println!("├╼ Parsing XML file        👀");
    // Get the base URL from the XML URL
    let base_url = reqwest::Url::parse(&xml_url)?;

    // Download XML file
    let response = client.get(xml_url).send().await?.text().await?;
    
    let files: XmlFiles = from_str(&response).map_err(|e| {
        eprintln!("XML parsing error: {}", e);
        eprintln!("XML content: {}", &response[..response.len().min(1000)]);
        e
    })?;
    println!("╰╼ Done                    👍️");

    // Iterate over the XML files struct and print every field
    for file in files.files {
        // Create a clone of the base URL
        let mut absolute_url = base_url.clone();

        // If the URL is relative, join it with the base_url to make it absolute
        if let Ok(joined_url) = absolute_url.join(&file.name) {
            absolute_url = joined_url;
        }
        // If it's an error, it might already be an absolute URL. Ignore.
        
        println!(" ");
        println!("📦️ Filename     {}", file.name);
        let mut download_action = "╰╼ Downloading  ";
        let mut download_complete = "├╼ Downloading  ";

        // Check if the file already exists
        if Path::new(&file.name).exists() {
            println!("├╼ Hash Check   🧮");
            // Calculate the MD5 hash of the local file
            let local_md5 = calculate_md5(&file.name).expect("╰╼ Failed to calculate MD5 hash");
            let expected_md5 = file.md5.as_ref().unwrap();
            if &local_md5 != expected_md5 {
                download_action = "╰╼ Resuming     ";
                download_complete = "├╼ Resuming     ";
            } else {
                println!("╰╼ Completed:   ✅");
                continue;
            }
        }

        // Check if file.name includes a path
        if let Some(path) = std::path::Path::new(&file.name).parent() {
            // Create the local directory if it doesn't exist and path has a file name
            if path.file_name().is_some() && !path.exists() {
                fs::create_dir_all(path)?;
            }
        }

        // Create a new file for writing
        let mut download = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&file.name)?;

        // Get the size of the local file if it already exists
        let file_size = download.metadata()?.len();
        if file_size > 0 {
            // Set the starting position for resuming the download
            download.seek(std::io::SeekFrom::Start(file_size))?;
        }

        // Set the Range header to specify the starting offset
        let mut initial_request = client.get(absolute_url);
        let range_header = format!("bytes={}-", file_size);
        let mut headers = HeaderMap::new();
        headers.insert(reqwest::header::RANGE, HeaderValue::from_str(&range_header)?);
        initial_request = initial_request.headers(headers);

        let mut response = initial_request.send().await?;

        // Get the content length from the response headers
        let content_length = response.content_length().unwrap_or(0);
        let pb = ProgressBar::new(content_length + file_size);
        pb.set_style(
            ProgressStyle::default_bar()
                .template(format!("{}{{elapsed_precise}}     {{bar:40.green/green}} {{bytes}}/{{total_bytes}} (ETA: {{eta}})", download_action).as_str()).expect("REASON")
                .progress_chars("▓▒░"),
        );

        // Download the remaining chunks and update the progress bar
        let mut total_bytes: u64 = file_size;
        while let Some(chunk) = response.chunk().await? {
            download.write_all(&chunk)?;
            total_bytes += chunk.len() as u64;
            pb.set_position(total_bytes);
        }

        pb.set_style(
            ProgressStyle::default_bar()
                .template(format!("{}{{elapsed_precise}}     {{bar:40.green/green}} {{total_bytes}}", download_complete).as_str()).expect("REASON")
        );
        pb.finish();

        println!("├╼ Hash Check   🧮");
        // Calculate the MD5 hash of the local file
        let local_md5 = calculate_md5(&file.name).expect("╰╼ Failed to calculate MD5 hash");
        
        match &file.md5 {
            Some(expected_md5) => {
                if local_md5 != *expected_md5 {
                    println!("╰╼ Failure:     ❌");
                } else {
                    println!("╰╼ Success:     ✅");
                }
            },
            None => println!("╰╼ No MD5:      ⚠️"),
        }
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
