use futures::StreamExt;
use reqwest::Client;
use serde::Deserialize;
use serde_xml_rs::from_str;
use clap::{App, Arg};
use std::io::Write;

#[derive(Deserialize, Debug)]
struct XmlRoot {
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
    old_version: Option<String>,
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
    let parsed_xml: XmlRoot = from_str(&response)?;

    // 3. Extract file URLs
    let file_urls: Vec<String> = parsed_xml
        .files
        .into_iter()
        .map(|f| f.name)
        .collect();

    // Get the base URL from the XML URL
    let base_url = reqwest::Url::parse(xml_url)?;

    // 4. Download each file
    for url in file_urls {
        let mut absolute_url = base_url.clone();

        // If the URL is relative, join it with the base_url to make it absolute
        match absolute_url.join(&url) {
            Ok(joined_url) => absolute_url = joined_url,
            Err(_) => {} // If it's an error, it might already be an absolute URL. Ignore.
        }

        let filename = url.split('/').last().unwrap_or("unknown_file");
        let mut response = client.get(absolute_url).send().await?;
        let mut file = std::fs::File::create(filename)?;

        while let Some(chunk) = response.chunk().await? {
            file.write_all(&chunk)?;
        }

        println!("Downloaded: {}", filename);
    }

    Ok(())
}
