use futures::StreamExt;
use reqwest::Client;
use serde::Deserialize;
use serde_xml_rs::from_str;
use clap::{App, Arg};
use std::io::Write;
use std::path::Path;

#[derive(Deserialize, Debug)]
struct Files {
    #[serde(rename = "file")]
    files: Vec<File>,
}

#[derive(Deserialize, Debug)]
struct File {
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

    // 1. Download XML file
    let response = reqwest::get(xml_url).await?.text().await?;

    // 2. Parse XML
    let files: Files = from_str(&response)?;

    // Get the base URL from the XML URL
    let base_url = reqwest::Url::parse(xml_url)?;

    // Iterate over the files and print every field
    for file in files.files {
        // Get the filename and strip the path
        let filename = file.name.split('/').last().unwrap_or("unknown_file");

        println!("------------------");
        println!("Name: {}", filename);
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

        // 4. Download each file
        let mut absolute_url = base_url.clone();

        // If the URL is relative, join it with the base_url to make it absolute
        match absolute_url.join(filename) {
            Ok(joined_url) => absolute_url = joined_url,
            Err(_) => {} // If it's an error, it might already be an absolute URL. Ignore.
        }

        // Check if the file already exists
        if Path::new(filename).exists() {
            println!("File already exists: {}", filename);
            continue;
        }

        let mut response = client.get(absolute_url).send().await?;
        let mut download = std::fs::File::create(filename)?;

        while let Some(chunk) = response.chunk().await? {
            download.write_all(&chunk)?;
        }

        println!("Downloaded: {}", filename);
    }

    Ok(())
}
