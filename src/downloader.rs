//! Module for handling file downloads, verification, and related operations.

use std::fs::{self, File};
use std::io::{BufReader, Read, Write, Seek, SeekFrom};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::Client;

use crate::Result;
use crate::utils::{create_progress_bar, format_duration, format_size, format_transfer_rate};

/// Buffer size for file operations (8KB)
const BUFFER_SIZE: usize = 8192;

/// File size threshold for showing hash progress bar (16MB)
const LARGE_FILE_THRESHOLD: u64 = 2 * 1024 * 1024;

/// Sets up signal handling for graceful shutdown on Ctrl+C
/// 
/// Returns an Arc<AtomicBool> that can be checked to see if the process
/// should stop. When Ctrl+C is pressed, this will be set to false.
fn setup_signal_handler() -> Arc<AtomicBool> {
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    
    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
        println!("\nReceived Ctrl+C, finishing current operation...");
    }).expect("Error setting Ctrl+C handler");
    
    running
}

/// Calculates the MD5 hash of a file
fn calculate_md5(file_path: &str, running: &Arc<AtomicBool>) -> Result<String> {
    let file = File::open(file_path)?;
    let file_size = file.metadata()?.len();
    let is_large_file = file_size > LARGE_FILE_THRESHOLD;
    
    let mut reader = BufReader::with_capacity(BUFFER_SIZE, file);
    let mut context = md5::Context::new();
    let mut buffer = [0; BUFFER_SIZE];
    
    let pb = if is_large_file {
        let progress_bar = create_progress_bar(file_size, "╰╼ Hashing      ", Some("cyan/blue"), false);
        Some(progress_bar)
    } else {
        None
    };
    
    let mut bytes_processed: u64 = 0;
    
    loop {
        let bytes_read = reader.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        
        if !running.load(Ordering::SeqCst) {
            if let Some(ref progress_bar) = pb {
                progress_bar.finish_and_clear();
            }
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "Hash calculation interrupted by signal",
            ).into());
        }
        
        context.consume(&buffer[..bytes_read]);
        
        if let Some(ref progress_bar) = pb {
            bytes_processed += bytes_read as u64;
            progress_bar.set_position(bytes_processed);
        }
    }
    
    if let Some(progress_bar) = pb {
        progress_bar.finish_and_clear();
    }
    
    let hash = context.compute();
    Ok(format!("{:x}", hash))
}

/// Check if an existing file has the correct hash
fn check_existing_file(file_path: &str, expected_md5: Option<&str>, running: &Arc<AtomicBool>) -> Result<Option<bool>> {
    if !Path::new(file_path).exists() {
        return Ok(None);
    }

    if expected_md5.is_none() {
        return Ok(Some(true)); 
    }

    let local_md5 = match calculate_md5(file_path, running) {
        Ok(hash) => hash,
        Err(e) => {
            if e.to_string().contains("interrupted by signal") {
                return Err(e);
            }
            println!("╰╼ Failed to calculate MD5 hash: {}", e);
            return Ok(Some(false));
        }
    };

    Ok(Some(local_md5 == expected_md5.unwrap()))
}

/// Ensure parent directories exist for a file
fn ensure_parent_directories(file_path: &str) -> Result<()> {
    if let Some(path) = Path::new(file_path).parent() {
        if path.file_name().is_some() && !path.exists() {
            fs::create_dir_all(path)?;
        }
    }
    Ok(())
}

/// Prepare a file for download
fn prepare_file_for_download(file_path: &str) -> Result<File> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(file_path)?;
    
    // Seek to the end of the file for resume capability
    file.seek(SeekFrom::End(0))?;
    
    Ok(file)
}

/// Download file content with progress reporting
async fn download_file_content(
    client: &Client, 
    url: &str, 
    file_size: u64, 
    file: &mut File,
    running: &Arc<AtomicBool>,
    is_resuming: bool
) -> Result<u64> {
    let download_action = if is_resuming { "╰╼ Resuming     " } else { "╰╼ Downloading  " };
    
    let mut response = if file_size > 0 {
        // Resume download with range request
        let range_header = format!("bytes={}-", file_size);
        let mut headers = HeaderMap::new();
        headers.insert(
            reqwest::header::RANGE, 
            HeaderValue::from_str(&range_header).map_err(|e| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput, 
                    format!("Invalid header value: {}", e)
                )
            })?
        );
        
        client.get(url).headers(headers).send().await?
    } else {
        // Fresh download
        client.get(url).send().await?
    };

    let content_length = response.content_length().unwrap_or(0);
    let total_expected_size = content_length + file_size;
    
    let pb = create_progress_bar(
        total_expected_size,
        download_action,
        None, 
        true
    );

    // Set initial progress to current file size for resumed downloads
    pb.set_position(file_size);

    let start_time = std::time::Instant::now();
    let mut total_bytes: u64 = file_size;
    let mut downloaded_bytes: u64 = 0;
    
    while let Some(chunk) = response.chunk().await? {
        if !running.load(Ordering::SeqCst) {
            pb.finish_and_clear();
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "Download interrupted during file transfer",
            ).into());
        }
        
        file.write_all(&chunk)?;
        downloaded_bytes += chunk.len() as u64;
        total_bytes += chunk.len() as u64;
        pb.set_position(total_bytes);
    }

    // Ensure data is written to disk
    file.flush()?;

    let elapsed = start_time.elapsed();
    let elapsed_secs = elapsed.as_secs_f64();
    
    let transfer_rate = if elapsed_secs > 0.0 {
        downloaded_bytes as f64 / elapsed_secs
    } else {
        0.0 
    };
    
    let (rate, unit) = format_transfer_rate(transfer_rate);
    
    pb.finish_and_clear();
    
    if downloaded_bytes > 0 {
        println!("├╼ Downloaded   ⤵️ {} in {} ({:.2} {}/s)", 
            format_size(downloaded_bytes),
            format_duration(elapsed),
            rate,
            unit);
    }
    
    Ok(total_bytes)
}

/// Verify a downloaded file's hash against an expected value
fn verify_downloaded_file(file_path: &str, expected_md5: Option<&str>, running: &Arc<AtomicBool>) -> Result<bool> {   
    let local_md5 = match calculate_md5(file_path, running) {
        Ok(hash) => hash,
        Err(e) => {
            println!("╰╼ Failed to calculate MD5 hash: {}", e);
            return Ok(false);
        }
    };
    
    match expected_md5 {
        Some(expected) => {
            let matches = local_md5 == expected;
            if !matches {
                println!("╰╼ Hash         ❌");
            } else {
                println!("╰╼ Hash         ✅");
            }
            Ok(matches)
        },
        None => {
            println!("╰╼ No MD5:      ⚠️");
            Ok(true) 
        },
    }
}

/// Download a file from archive.org with resume capability
/// 
/// This function handles signal setup internally and manages graceful shutdown
/// during download operations.
pub async fn download_file(
    client: &Client,
    url: &str,
    file_path: &str,
    expected_md5: Option<&str>,
) -> Result<()> {
    // Set up signal handling for this download session
    let running = setup_signal_handler();
    
    println!(" ");
    println!("📦️ Filename     {}", file_path);
    
    if let Some(is_valid) = check_existing_file(file_path, expected_md5, &running)? {
        if is_valid {
            println!("╰╼ Downloaded   ✅");
            return Ok(());
        } else {
            println!("├╼ Partial      🔄");
        }
    }

    ensure_parent_directories(file_path)?;
    
    let mut file = prepare_file_for_download(file_path)?;
    
    let file_size = file.metadata()?.len();        
    let is_resuming = file_size > 0;
    download_file_content(client, url, file_size, &mut file, &running, is_resuming).await?;
    verify_downloaded_file(file_path, expected_md5, &running)?;
    
    Ok(())
}

/// Download multiple files with shared signal handling
/// 
/// This function sets up signal handling once for the entire download session
/// and allows for graceful interruption between files.
pub async fn download_files<I>(
    client: &Client,
    files: I,
    total_files: usize,
) -> Result<()> 
where
    I: IntoIterator<Item = (String, String, Option<String>)>, // (url, filename, md5)
{
    // Set up signal handling for the entire download session
    let running = setup_signal_handler();
    
    for (index, (url, file_path, expected_md5)) in files.into_iter().enumerate() {
        // Check if we should stop due to signal
        if !running.load(Ordering::SeqCst) {
            println!("\nDownload interrupted. Run the command again to resume remaining files.");
            break;
        }
        
        println!(" ");
        println!("📦️ Filename     {}", file_path);
        println!("├╼ Count        {} of {}", index + 1, total_files);
        
        if let Some(is_valid) = check_existing_file(&file_path, expected_md5.as_deref(), &running)? {
            if is_valid {
                println!("╰╼ Downloaded   ✅");
                continue;
            } else {
                println!("├╼ Partial      🔄");
            }
        }

        ensure_parent_directories(&file_path)?;
        
        let mut file = prepare_file_for_download(&file_path)?;
        
        let file_size = file.metadata()?.len();        
        let is_resuming = file_size > 0;
        download_file_content(client, &url, file_size, &mut file, &running, is_resuming).await?;
        verify_downloaded_file(&file_path, expected_md5.as_deref(), &running)?;
    }
    
    Ok(())
}
