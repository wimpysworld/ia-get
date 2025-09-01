//! Simple test program to verify the new JSON API functionality

use ia_get::{fetch_json_metadata, get_user_agent};
use indicatif::ProgressBar;
use reqwest::Client;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Test the mario archive we used earlier
    let test_url = "https://archive.org/details/mario";

    // Create HTTP client
    let client = Client::builder().user_agent(get_user_agent()).build()?;

    // Create progress spinner
    let progress = ProgressBar::new_spinner();
    progress.set_message("Testing JSON metadata API...");

    println!("🧪 Testing enhanced JSON metadata API");
    println!("URL: {}", test_url);

    // Fetch metadata using the new JSON API
    match fetch_json_metadata(test_url, &client, &progress).await {
        Ok((metadata, _base_url)) => {
            progress.finish_with_message("✓ Successfully fetched JSON metadata");

            println!("\n📊 Archive Information:");
            println!("  Identifier: mario");
            println!("  Total files: {}", metadata.files.len());
            println!("  Archive size: {} bytes", metadata.item_size);
            println!("  Server: {}", metadata.server);
            println!(
                "  Available servers: {}",
                metadata.workable_servers.join(", ")
            );
            println!("  Directory: {}", metadata.dir);

            println!("\n📁 Files in archive:");
            for (i, file) in metadata.files.iter().enumerate() {
                let size_str = file
                    .size
                    .map(|s| format!(" ({} bytes)", s))
                    .unwrap_or_default();
                let format_str = file.format.as_deref().unwrap_or("Unknown");
                let md5_str = file
                    .md5
                    .as_deref()
                    .map(|h| format!(" [MD5: {}]", h))
                    .unwrap_or_default();

                println!(
                    "  {}. {} - {}{}{}",
                    i + 1,
                    file.name,
                    format_str,
                    size_str,
                    md5_str
                );
            }

            println!("\n🌐 Download URLs using workable servers:");
            if let Some(first_file) = metadata.files.first() {
                for (i, server) in metadata.workable_servers.iter().enumerate() {
                    let download_url = first_file.get_download_url(server, &metadata.dir);
                    println!("  Server {}: {}", i + 1, download_url);
                }
            }

            println!("\n✅ JSON API test completed successfully!");
        }
        Err(e) => {
            progress.finish_with_message("✗ Failed to fetch metadata");
            eprintln!("❌ Error: {}", e);
            return Err(e.into());
        }
    }

    Ok(())
}
