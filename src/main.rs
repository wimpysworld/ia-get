use futures::StreamExt;
use reqwest::Client;
use serde::Deserialize;
use serde_xml_rs::from_str;
use clap::{App, Arg};
use std::fs;
use std::io::Write;
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new();
    // Parse command line arguments
    let matches = App::new("Archive Downloader")
        .version("0.1.0")
        .author("Martin Wimpress")
        .about("Downloads XML and its referenced files from archive.org")
        .arg(Arg::with_name("URL")
             .help("URL of the XML file on archive.org")
             .required(true)
             .index(1))
        .get_matches();

    let xml_url = matches.value_of("URL").unwrap();
    // Get the base URL from the XML URL
    let base_url = reqwest::Url::parse(xml_url)?;

    // Download XML file
    let response = reqwest::get(xml_url).await?.text().await?;
    let files: XmlFiles = from_str(&response)?;

    // Iterate over the XML files struct and print every field
    for file in files.files {
        println!("------------------");
        println!("Name: {}", file.name);
        println!("Source: {}", file.source);
        if let Some(mtime) = file.mtime {
            println!("MTime: {}", mtime);
        }
        if let Some(size) = file.size {
            println!("Size: {}", size);
        }
        if let Some(format) = file.format {
            println!("Format: {}", format);
        }
        if let Some(rotation) = file.rotation {
            println!("Rotation: {}", rotation);
        }
        if let Some(md5) = file.md5 {
            println!("MD5: {}", md5);
        }
        if let Some(crc32) = file.crc32 {
            println!("CRC32: {}", crc32);
        }
        if let Some(sha1) = file.sha1 {
            println!("SHA1: {}", sha1);
        }
        if let Some(btih) = file.btih {
            println!("BTIH: {}", btih);
        }
        if let Some(summation) = file.summation {
            println!("Summation: {}", summation);
        }
        if let Some(original) = file.original {
            println!("Original: {}", original);
        }
        if let Some(old_version) = file.old_version {
            println!("Old Version: {}", old_version);
        }

        // Create a clone of the base URL
        let mut absolute_url = base_url.clone();

        // If the URL is relative, join it with the base_url to make it absolute
        match absolute_url.join(&file.name) {
            Ok(joined_url) => absolute_url = joined_url,
            Err(_) => {} // If it's an error, it might already be an absolute URL. Ignore.
        }

        // Check if the file already exists
        if Path::new(&file.name).exists() {
            println!("File already exists: {}", file.name);
            continue;
        }

        // Check if the file name includes a path
        if let Some(path) = std::path::Path::new(&file.name).parent() {
            // Create the local directory if it doesn't exist
            fs::create_dir_all(path)?;
            println!("Created directory: {:?}", path);
        }

        // Download the file
        let mut response = client.get(absolute_url).send().await?;
        let mut download = std::fs::File::create(&file.name)?;

        while let Some(chunk) = response.chunk().await? {
            download.write_all(&chunk)?;
        }

        println!("Downloaded: {}", file.name);
    }

    Ok(())
}
