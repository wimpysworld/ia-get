use futures::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use md5;
use regex::Regex;
use reqwest::header::{HeaderValue, HeaderMap};
use reqwest::Client;
use serde::Deserialize;
use serde_xml_rs::from_str;
use clap::{App, Arg};
use std::error::Error;
use std::fs;
use std::io::{Seek, Write};
use std::process;
use std::path::Path;

#[derive(Deserialize, Debug)]
struct XmlFiles {
    #[serde(rename = "file")]
    files: Vec<XmlFile>,
}

#[derive(Deserialize, Debug)]
struct XmlFile {
    #[serde(rename = "name")]
    name: String,
    #[serde(rename = "source")]
    source: String,
    #[serde(rename = "mtime")]
    mtime: Option<u64>,
    #[serde(rename = "size")]
    size: Option<u64>,
    #[serde(rename = "format")]
    format: Option<String>,
    #[serde(rename = "rotation")]
    rotation: Option<u32>,
    #[serde(rename = "md5")]
    md5: Option<String>,
    #[serde(rename = "crc32")]
    crc32: Option<String>,
    #[serde(rename = "sha1")]
    sha1: Option<String>,
    #[serde(rename = "btih")]
    btih: Option<String>,
    #[serde(rename = "summation")]
    summation: Option<String>,
    #[serde(rename = "original")]
    original: Option<String>,
    #[serde(rename = "old_version")]
    old_version: Option<bool>,
}

async fn is_url_accessible(url: &str) -> Result<bool, Box<dyn Error>> {
    let client = reqwest::Client::new();
    let res = client.get(url).send().await;

    match res {
        Ok(response) => {
            if response.status().is_success() {
                Ok(true)
            } else {
                Ok(false)
            }
        },
        Err(_) => Ok(false),
    }
}

fn get_xml_url(original_url: &str) -> String {
    let base_new_url = original_url.replacen("details", "download", 1);
    if let Some(last_segment) = original_url.split('/').last() {
        format!("{}/{}_files.xml", base_new_url, last_segment)
    } else {
        base_new_url
    }
}

fn calculate_md5(file_path: &str) -> Result<String, std::io::Error> {
    let file_contents = fs::read(file_path)?;
    let hash = md5::compute(&file_contents);
    Ok(format!("{:x}", hash))
}

// Define the regular expression pattern for the expected format as a static constant
static PATTERN: &str = r"^https:\/\/archive\.org\/details\/[a-zA-Z0-9_-]+$";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new();

    let name = env!("CARGO_PKG_NAME");
    let version = env!("CARGO_PKG_VERSION");
    let authors = env!("CARGO_PKG_AUTHORS");
    let description = env!("CARGO_PKG_DESCRIPTION");

    // Parse command line arguments
    let matches = App::new(name)
        .version(version)
        .author(authors)
        .about(description)
        .arg(Arg::with_name("URL")
             .help("URL to an archive.org details page")
             .required(true)
             .index(1))
        .get_matches();

    let details_url = matches.value_of("URL").ok_or("Missing URL argument")?;

    // Create a regex object with the static pattern
    let regex = Regex::new(PATTERN)?;

    println!("Archive.org URL: {}", details_url);
    if !regex.is_match(details_url) {
        println!("├╼ Archive.org URL is not in the expected format");
        println!("╰╼ Expected format: https://archive.org/details/<identifier>/");
        process::exit(1);
    }

    match is_url_accessible(details_url).await {
        Ok(_) => println!("╰╼ Archive.org URL online: 🟢"),
        Err(e) => {
            println!("├╼ Archive.org URL online: 🔴");
            panic!  ("╰╼ Exiting due to error: {}", e);
        }
    }

    let xml_url = get_xml_url(details_url);
    println!("Archive.org XML: {}", xml_url);

    match is_url_accessible(&xml_url).await {
        Ok(_) => println!("├╼ Archive.org XML online: 🟢"),
        Err(e) => {
            println!("├╼ Archive.org XML online: 🔴");
            panic!  ("╰╼ Exiting due to error: {}", e);
        }
    }

    println!("├╼ Parsing XML file        👀");
    // Get the base URL from the XML URL
    let base_url = reqwest::Url::parse(&xml_url)?;

    // Download XML file
    let response = reqwest::get(xml_url).await?.text().await?;
    let files: XmlFiles = from_str(&response)?;
    println!("╰╼ Done                    👍️");

    // Iterate over the XML files struct and print every field
    for file in files.files {
        // Create a clone of the base URL
        let mut absolute_url = base_url.clone();

        // If the URL is relative, join it with the base_url to make it absolute
        match absolute_url.join(&file.name) {
            Ok(joined_url) => absolute_url = joined_url,
            Err(_) => {} // If it's an error, it might already be an absolute URL. Ignore.
        }
        println!(" ");
        println!("📦️ Filename     {}", file.name);
        let mut download_action = "╰╼ Downloading  ";
        let mut download_complete = "├╼ Downloading  ";

        // Check if the file already exists
        if Path::new(&file.name).exists() {
            println!("├╼ Hash Check   🧮");
            // Calculate the MD5 hash of the local file
            let mut local_md5 = calculate_md5(&file.name).expect("╰╼ Failed to calculate MD5 hash");
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
        let mut local_md5 = calculate_md5(&file.name).expect("╰╼ Failed to calculate MD5 hash");
        let expected_md5 = file.md5.as_ref().unwrap();
        if &local_md5 != expected_md5 {
            println!("╰╼ Failure:     ❌");
        } else {
            println!("╰╼ Success:     ✅");
        }
    }

    Ok(())
}
