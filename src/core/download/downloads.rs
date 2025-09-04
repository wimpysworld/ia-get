//! Downloads management module for ia-get
//!
//! Handles file download orchestration with retry logic.

use crate::{core::download::downloader, error::IaGetError, Result};
use colored::*;
use reqwest::Client;
use std::sync::{atomic::AtomicBool, Arc};

/// Downloads files with retry logic for transient network errors
pub async fn download_files_with_retries(
    client: &Client,
    download_data: Vec<(String, String, Option<String>)>,
    total_files: usize,
    log_hash_errors: bool,
    running: Arc<AtomicBool>,
) -> Result<()> {
    let max_retries = 3;
    let mut retries = 0;
    let mut delay = std::time::Duration::from_secs(60);
    let max_delay = std::time::Duration::from_secs(900); // 15 minutes
    loop {
        // Extract output directory from the first file path
        let result = downloader::download_files(
            client,
            download_data.clone(),
            total_files,
            log_hash_errors,
            running.clone(),
        )
        .await
        .map_err(|e| match e {
            IaGetError::Network(msg) => IaGetError::Network(msg),
            other => IaGetError::Network(format!("Other error: {}", other)),
        });
        match result {
            Ok(_) => return Ok(()),
            Err(e) => {
                // Only retry on transient network errors
                let is_transient = matches!(e, IaGetError::Network(_));
                if is_transient && retries < max_retries {
                    eprintln!(
                        "{} Network error: {}. Retrying in {}s...",
                        "▲".yellow(),
                        e,
                        delay.as_secs()
                    );
                    retries += 1;
                    tokio::time::sleep(delay).await;
                    delay = std::cmp::min(delay * 2, max_delay);
                    continue;
                } else {
                    return Err(e);
                }
            }
        }
    }
}
