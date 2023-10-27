use reqwest;
use serde::Deserialize;
use serde_xml_rs::from_str;
use clap::{App, Arg};

#[derive(Deserialize, Debug)]
struct XmlRoot {
    file: Vec<File>,
}

#[derive(Deserialize, Debug)]
struct File {
    #[serde(rename = "location")]
    location: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Parse command line arguments
    let matches = App::new("Archive Downloader")
        .version("1.0")
        .author("Your Name")
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
        .map(|f| f.location)
        .collect();

    // 4. Download each file
    for url in file_urls {
        let filename = url.split('/').last().unwrap_or("unknown_file");
        let content = reqwest::get(&url).await?.bytes().await?;
        std::fs::write(filename, content)?;
        println!("Downloaded: {}", filename);
    }

    Ok(())
}
