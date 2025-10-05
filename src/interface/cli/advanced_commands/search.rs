//! Search functionality for Internet Archive
//!
//! Provides comprehensive search capabilities with filtering, sorting,
//! and detailed result display.

use anyhow::{Context, Result};
use colored::*;
use serde::{Deserialize, Serialize};

/// Search results from Internet Archive
#[derive(Debug, Deserialize, Serialize)]
pub struct SearchResults {
    pub response: SearchResponse,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SearchResponse {
    pub docs: Vec<SearchDoc>,
    #[serde(rename = "numFound")]
    pub num_found: usize,
    pub start: usize,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SearchDoc {
    pub identifier: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub mediatype: Option<String>,
    pub creator: Option<Vec<String>>,
    pub date: Option<String>,
    pub downloads: Option<i64>,
    pub item_size: Option<i64>,
    pub publicdate: Option<String>,
}

/// Search the Internet Archive
pub async fn search_archive(
    query: &str,
    media_type: Option<&str>,
    year: Option<&str>,
    sort: Option<&str>,
    limit: usize,
) -> Result<SearchResults> {
    let client = reqwest::Client::new();

    // Build search query
    let mut full_query = query.to_string();

    if let Some(mt) = media_type {
        full_query.push_str(&format!(" AND mediatype:({})", mt));
    }

    if let Some(y) = year {
        if y.contains('-') {
            let parts: Vec<&str> = y.split('-').collect();
            if parts.len() == 2 {
                full_query.push_str(&format!(" AND year:[{} TO {}]", parts[0], parts[1]));
            }
        } else {
            full_query.push_str(&format!(" AND year:{}", y));
        }
    }

    // Build URL
    let url = format!(
        "https://archive.org/advancedsearch.php?q={}&output=json&rows={}{}",
        urlencoding::encode(&full_query),
        limit,
        sort.map(|s| format!("&sort[]={}", s)).unwrap_or_default()
    );

    // Make request
    let response = client
        .get(&url)
        .header(
            "User-Agent",
            "ia-get-cli/1.6.0 (https://github.com/Gameaday/ia-get-cli)",
        )
        .send()
        .await
        .context("Failed to search Internet Archive")?;

    if !response.status().is_success() {
        anyhow::bail!("Search failed with status: {}", response.status());
    }

    let results: SearchResults = response
        .json()
        .await
        .context("Failed to parse search results")?;

    Ok(results)
}

/// Display search results in a formatted table
pub fn display_search_results(results: &SearchResults) {
    println!("\n{}", "=".repeat(80).cyan());
    println!(
        "{} {}",
        "Search Results:".bright_cyan().bold(),
        format!("{} items found", results.response.num_found).yellow()
    );
    println!("{}\n", "=".repeat(80).cyan());

    if results.response.docs.is_empty() {
        println!("{}", "No results found.".yellow());
        return;
    }

    for (idx, doc) in results.response.docs.iter().enumerate() {
        println!(
            "{}",
            format!("{}. {}", idx + 1, doc.identifier)
                .bright_white()
                .bold()
        );

        if let Some(title) = &doc.title {
            println!("   {}: {}", "Title".green(), title);
        }

        if let Some(mediatype) = &doc.mediatype {
            println!("   {}: {}", "Type".green(), mediatype);
        }

        if let Some(creators) = &doc.creator {
            if !creators.is_empty() {
                println!("   {}: {}", "Creator".green(), creators.join(", "));
            }
        }

        if let Some(date) = &doc.date {
            println!("   {}: {}", "Date".green(), date);
        }

        if let Some(downloads) = doc.downloads {
            println!("   {}: {}", "Downloads".green(), format_number(downloads));
        }

        if let Some(size) = doc.item_size {
            println!("   {}: {}", "Size".green(), format_size(size as usize));
        }

        if let Some(desc) = &doc.description {
            let truncated = if desc.len() > 150 {
                format!("{}...", &desc[..150])
            } else {
                desc.clone()
            };
            println!("   {}: {}", "Description".green(), truncated.dimmed());
        }

        println!(
            "   {}: {}",
            "URL".green(),
            format!("https://archive.org/details/{}", doc.identifier)
                .blue()
                .underline()
        );
        println!();
    }
}

/// Format a number with thousand separators
fn format_number(n: i64) -> String {
    let s = n.to_string();
    let mut result = String::new();
    let mut count = 0;

    for c in s.chars().rev() {
        if count == 3 {
            result.push(',');
            count = 0;
        }
        result.push(c);
        count += 1;
    }

    result.chars().rev().collect()
}

/// Format size in human-readable format
fn format_size(bytes: usize) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit_idx = 0;

    while size >= 1024.0 && unit_idx < UNITS.len() - 1 {
        size /= 1024.0;
        unit_idx += 1;
    }

    format!("{:.2} {}", size, UNITS[unit_idx])
}
