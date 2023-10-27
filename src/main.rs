use reqwest;
use serde::Deserialize;
use serde_xml_rs::from_str;
use clap::{App, Arg};
use tokio::sync::Semaphore;
use std::sync::Arc;

#[derive(Deserialize, Debug)]
struct XmlRoot {
    file: Vec<File>,
}

#[derive(Deserialize, Debug)]
struct File {
    name: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Parse command line arguments
    let matches = App::new("Archive Downloader")
        .version("1.0")
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
        .file
        .into_iter()
        .map(|f| f.name)
        .collect();

    // Get the base URL from the XML URL
    let base_url = reqwest::Url::parse(xml_url)?;

    let semaphore = Arc::new(Semaphore::new(4)); // Allow 4 permits at a time

    let downloads: Vec<_> = file_urls.iter().map(|url| {
        let semaphore = Arc::clone(&semaphore);
        let base_url = base_url.clone();
        let url = url.clone();

        tokio::spawn(async move {
            use anyhow::Context;
            let _permit = semaphore.acquire().await;

            let mut absolute_url = base_url.clone();
            match absolute_url.join(&url) {
                Ok(joined_url) => absolute_url = joined_url,
                Err(_) => {}
            }

            let filename = url.split('/').last().unwrap_or("unknown_file");
            let content = reqwest::get(absolute_url.as_str()).await.context("Failed to download the URL")?.bytes().await.context("Failed to read bytes from the response")?;

            std::fs::write(filename, content).context("Failed to write the file")?;
            println!("Downloaded: {}", filename);

            Result::<(), anyhow::Error>::Ok(())
        })
    }).collect();

    // Wait for all downloads to complete
    let results: Vec<_> = futures::future::join_all(downloads).await;

    // Handle any errors that occurred during downloads
    for result in results {
        if let Err(e) = result {
            eprintln!("Error encountered during download: {:?}", e);
        }
    }

    Ok(())
}
