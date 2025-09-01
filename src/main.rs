//! Main entry point for ia-get CLI application

use anyhow::{Context, Result};
use clap::{Arg, ArgAction, Command};
use colored::Colorize;
use reqwest::Client;
use std::path::{Path, PathBuf};
use tokio::signal;

use ia_get::{
    constants::get_user_agent,
    enhanced_downloader::ArchiveDownloader,
    fetch_json_metadata,
    filters::{format_size, parse_size_string},
    metadata_storage::{ArchiveFile, DownloadConfig, DownloadSession, DownloadState},
    IaGetError,
};

use indicatif::{ProgressBar, ProgressStyle};

/// Entry point for the ia-get CLI application
#[tokio::main]
async fn main() -> Result<()> {
    // Set up signal handling for graceful shutdown
    tokio::spawn(async {
        signal::ctrl_c().await.expect("Failed to listen for Ctrl+C");
        println!("\n{} Download interrupted by user", "⚠️".yellow());
        std::process::exit(0);
    });

    // Parse command line arguments
    let matches = build_cli().get_matches();

    // Extract arguments
    let identifier = matches
        .get_one::<String>("identifier")
        .ok_or_else(|| anyhow::anyhow!("Archive identifier is required"))?;

    let output_dir = matches
        .get_one::<String>("output")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let mut current = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            current.push(identifier);
            current
        });

    let _verbose = matches.get_flag("verbose");
    let dry_run = matches.get_flag("dry-run");

    let concurrent_downloads = matches
        .get_one::<String>("concurrent")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(4)
        .min(16); // Cap at 16 concurrent downloads

    let include_formats = matches
        .get_many::<String>("include")
        .map(|values| values.map(|s| s.to_string()).collect::<Vec<_>>())
        .unwrap_or_default();

    let max_file_size = matches
        .get_one::<String>("max-size")
        .and_then(|s| parse_size_string(s).ok());

    // Compression settings - enable by default as requested
    let enable_compression = !matches.get_flag("no-compress"); // Default to true unless --no-compress is specified
    let auto_decompress = matches.get_flag("decompress");
    let decompress_formats = matches
        .get_many::<String>("decompress-formats")
        .map(|values| values.map(|s| s.to_string()).collect::<Vec<_>>())
        .unwrap_or_default();

    // Create HTTP client
    let client = Client::builder()
        .user_agent(get_user_agent())
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .context("Failed to create HTTP client")?;

    // Create download configuration (for future use)
    let _config = DownloadConfig {
        output_dir: output_dir.to_string_lossy().to_string(),
        max_concurrent: concurrent_downloads as u32,
        format_filters: include_formats.clone(),
        min_size: None,
        max_size: max_file_size,
        verify_md5: true,
        preserve_mtime: true,
        user_agent: get_user_agent(),
        enable_compression,
        auto_decompress,
        decompress_formats: decompress_formats.clone(),
    };

    // Create session directory
    let session_dir = output_dir.join(".ia-get-sessions");

    // Initialize the archive downloader
    let downloader = ArchiveDownloader::new(
        client,
        concurrent_downloads,
        true, // verify_md5
        true, // preserve_mtime
        session_dir,
        enable_compression,
        auto_decompress,
    );

    println!(
        "{} Initializing download for archive: {}",
        "🚀".blue(),
        identifier.bright_cyan().bold()
    );

    if dry_run {
        println!(
            "{} DRY RUN MODE - fetching metadata only",
            "🔍".yellow().bold()
        );
    }

    // Construct the archive URL
    let archive_url = format!("https://archive.org/details/{}", identifier);

    // Use our test API function structure but with the enhanced downloader
    match fetch_and_display_metadata(
        &archive_url,
        &downloader,
        &output_dir,
        &include_formats,
        max_file_size,
        concurrent_downloads,
        dry_run,
        enable_compression,
        auto_decompress,
        &decompress_formats,
    )
    .await
    {
        Ok(_) => {
            if !dry_run {
                println!("\n{} Download completed successfully!", "✅".green().bold());
                println!(
                    "📁 Output directory: {}",
                    output_dir.display().to_string().bright_green()
                );
            }
        }
        Err(e) => {
            eprintln!("{} Error: {}", "✘".red().bold(), e);
            std::process::exit(1);
        }
    }

    Ok(())
}

/// Apply file filters based on CLI arguments
fn apply_file_filters(
    files: &[ArchiveFile],
    include_formats: &[String],
    max_file_size: Option<u64>,
) -> Vec<ArchiveFile> {
    files
        .iter()
        .filter(|file| {
            // Apply format filter
            if !include_formats.is_empty() {
                let file_format = file.format.as_deref().unwrap_or("");

                // Check both the format field and file extension
                let file_extension = std::path::Path::new(&file.name)
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .unwrap_or("");

                let matches_format = include_formats.iter().any(|fmt| {
                    fmt.eq_ignore_ascii_case(file_format)
                        || fmt.eq_ignore_ascii_case(file_extension)
                });

                if !matches_format {
                    return false;
                }
            }

            // Apply size filter
            if let Some(max_size) = max_file_size {
                if file.size.unwrap_or(0) > max_size {
                    return false;
                }
            }

            true
        })
        .cloned()
        .collect()
}

/// Create download configuration from CLI arguments
fn create_download_config(
    output_dir: &Path,
    concurrent_downloads: usize,
    include_formats: &[String],
    max_file_size: Option<u64>,
    enable_compression: bool,
    auto_decompress: bool,
    decompress_formats: &[String],
) -> Result<DownloadConfig> {
    Ok(DownloadConfig {
        output_dir: output_dir.to_string_lossy().to_string(),
        max_concurrent: concurrent_downloads as u32,
        format_filters: include_formats.to_vec(),
        min_size: None,
        max_size: max_file_size,
        verify_md5: true,
        preserve_mtime: true,
        user_agent: get_user_agent(),
        enable_compression,
        auto_decompress,
        decompress_formats: decompress_formats.to_vec(),
    })
}

/// Create a progress bar for download tracking
fn create_progress_bar(file_count: usize) -> ProgressBar {
    let pb = ProgressBar::new(file_count as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} files ({msg})")
            .unwrap()
            .progress_chars("##-")
    );
    pb.set_message("Downloading...");
    pb
}

/// Display download summary after completion
fn display_download_summary(session: &DownloadSession, output_dir: &Path) {
    let completed_files = session
        .file_status
        .values()
        .filter(|status| {
            matches!(
                status.status,
                ia_get::metadata_storage::DownloadState::Completed
            )
        })
        .count();
    let total_files = session.file_status.len();
    let total_bytes: u64 = session
        .file_status
        .values()
        .filter(|status| {
            matches!(
                status.status,
                ia_get::metadata_storage::DownloadState::Completed
            )
        })
        .map(|status| status.file_info.size.unwrap_or(0))
        .sum();

    println!("\n{} Download Summary:", "📋".blue().bold());
    println!("  📂 Archive: {}", session.identifier);
    println!(
        "  📁 Output directory: {}",
        output_dir.display().to_string().bright_green()
    );
    println!("  📊 Files downloaded: {}/{}", completed_files, total_files);
    println!(
        "  💾 Total size: {}",
        format_size(total_bytes).bright_blue()
    );

    if completed_files < total_files {
        println!("\n{} Some files were not downloaded:", "⚠️".yellow());
        for (filename, status) in &session.file_status {
            if !matches!(
                status.status,
                ia_get::metadata_storage::DownloadState::Completed
            ) {
                println!("  • {} - {:?}", filename, status.status);
            }
        }
        println!(
            "\n💡 Use {} to retry failed downloads",
            "--resume".bright_blue()
        );
    }
}

/// Fetch metadata and optionally download files
#[allow(clippy::too_many_arguments)]
async fn fetch_and_display_metadata(
    archive_url: &str,
    downloader: &ArchiveDownloader,
    output_dir: &Path,
    include_formats: &[String],
    max_file_size: Option<u64>,
    concurrent_downloads: usize,
    dry_run: bool,
    enable_compression: bool,
    auto_decompress: bool,
    decompress_formats: &[String],
) -> Result<()> {
    // Create a client for metadata operations
    let client = Client::builder()
        .user_agent(get_user_agent())
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .context("Failed to create HTTP client")?;

    // Create progress spinner for metadata fetching
    let progress = indicatif::ProgressBar::new_spinner();
    progress.set_style(
        indicatif::ProgressStyle::default_spinner()
            .template("{spinner:.green} {msg}")
            .unwrap(),
    );

    let identifier = archive_url
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("unknown")
        .to_string();

    println!("{} Fetching metadata for: {}", "📡".cyan(), identifier);

    // Use the existing fetch_json_metadata function instead of duplicating logic
    let (metadata, _base_url) = fetch_json_metadata(archive_url, &client, &progress)
        .await
        .context("Failed to fetch metadata using JSON API")?;

    progress.finish_and_clear();

    // Display metadata information
    println!("\n{} Archive Information:", "📊".blue().bold());
    println!("  Identifier: {}", identifier);
    println!("  Total files: {}", metadata.files.len());
    println!("  Archive size: {}", format_size(metadata.item_size));
    println!("  Server: {}", metadata.server);
    println!(
        "  Available servers: {}",
        metadata.workable_servers.join(", ")
    );
    println!("  Directory: {}", metadata.dir);

    if dry_run {
        // Apply filtering for dry-run display too
        let filtered_files = apply_file_filters(&metadata.files, include_formats, max_file_size);

        println!("\n{} Files in archive:", "📋".cyan().bold());

        // Show filtering info if filters are applied
        if !include_formats.is_empty() || max_file_size.is_some() {
            println!(
                "  {}: {} → {} files",
                "After filtering".yellow(),
                metadata.files.len(),
                filtered_files.len()
            );
            if !include_formats.is_empty() {
                println!(
                    "  {}: {}",
                    "Format filter".yellow(),
                    include_formats.join(", ").cyan()
                );
            }
            if let Some(max_size) = max_file_size {
                println!(
                    "  {}: {}",
                    "Size limit".yellow(),
                    format_size(max_size).cyan()
                );
            }
            println!();
        }

        let display_files = if filtered_files.is_empty() {
            &metadata.files
        } else {
            &filtered_files
        };

        for (i, file) in display_files.iter().enumerate().take(10) {
            println!(
                "  {:<3} {} ({})",
                format!("{}.", i + 1).dimmed(),
                file.name.green(),
                format_size(file.size.unwrap_or(0)).cyan()
            );
        }
        if display_files.len() > 10 {
            println!("  ... and {} more files", display_files.len() - 10);
        }

        if filtered_files.is_empty() && (!include_formats.is_empty() || max_file_size.is_some()) {
            println!("\n{} No files match the specified filters", "⚠️".yellow());
        }

        println!("\n{} Use without --dry-run to download", "💡".yellow());
    } else {
        // Implement actual downloading with the enhanced downloader
        println!("\n{} Starting download...", "🚀".green().bold());

        // Apply filtering based on CLI arguments
        let filtered_files = apply_file_filters(&metadata.files, include_formats, max_file_size);

        if filtered_files.is_empty() {
            println!("{} No files match the specified filters", "⚠️".yellow());
            println!("💡 Try adjusting your --include filters or --max-size limits");
            return Ok(());
        }

        println!("📋 {} files selected for download", filtered_files.len());

        // Calculate total download size
        let total_size: u64 = filtered_files.iter().map(|f| f.size.unwrap_or(0)).sum();
        println!("📊 Total download size: {}", format_size(total_size));

        // Show download configuration
        println!("⚙️  Configuration:");
        println!(
            "   • Output directory: {}",
            output_dir.display().to_string().cyan()
        );
        println!(
            "   • Concurrent downloads: {}",
            concurrent_downloads.to_string().cyan()
        );
        if !include_formats.is_empty() {
            println!("   • Format filters: {}", include_formats.join(", ").cyan());
        }
        if let Some(max_size) = max_file_size {
            println!("   • Max file size: {}", format_size(max_size).cyan());
        }

        // Create proper download configuration
        let download_config = create_download_config(
            output_dir,
            concurrent_downloads,
            include_formats,
            max_file_size,
            enable_compression,
            auto_decompress,
            decompress_formats,
        )?;

        // Get list of file names to download
        let requested_files: Vec<String> = filtered_files.iter().map(|f| f.name.clone()).collect();

        // Create progress bar with better styling
        let progress_bar = create_progress_bar(filtered_files.len());
        progress_bar.set_message("Initializing download session...".yellow().to_string());

        println!("\n{} Beginning file downloads...", "🔥".green().bold());

        // Execute download with enhanced downloader and detailed error handling
        match downloader
            .download_with_metadata(
                archive_url.to_string(),
                identifier,
                metadata,
                download_config,
                requested_files,
                &progress_bar,
            )
            .await
        {
            Ok(session) => {
                progress_bar.finish_with_message(
                    "✅ Download completed successfully!"
                        .green()
                        .bold()
                        .to_string(),
                );
                display_download_summary(&session, output_dir);

                // Provide next steps if session has failed files
                let failed_files: Vec<_> = session
                    .file_status
                    .values()
                    .filter(|status| matches!(status.status, DownloadState::Failed))
                    .collect();

                if !failed_files.is_empty() {
                    println!(
                        "\n{} {} files failed to download",
                        "⚠️".yellow(),
                        failed_files.len()
                    );
                    println!("💡 You can retry the download with the same command to resume");
                }
            }
            Err(e) => {
                progress_bar.finish_with_message("❌ Download failed".red().bold().to_string());

                // Enhanced error reporting with specific guidance
                match &e {
                    IaGetError::Network(msg) => {
                        eprintln!("\n{} Network Error:", "🌐".red().bold());
                        eprintln!("   {}", msg);
                        eprintln!("💡 Suggestions:");
                        eprintln!("   • Check your internet connection");
                        eprintln!("   • Try again in a few minutes");
                        eprintln!("   • Use --concurrent 1 for slower but more reliable downloads");
                    }
                    IaGetError::FileSystem(msg) => {
                        eprintln!("\n{} File System Error:", "📁".red().bold());
                        eprintln!("   {}", msg);
                        eprintln!("💡 Suggestions:");
                        eprintln!("   • Check available disk space");
                        eprintln!("   • Verify write permissions to output directory");
                        eprintln!("   • Try a different output directory with --output");
                    }
                    IaGetError::HashMismatch(msg) => {
                        eprintln!("\n{} File Integrity Error:", "🔍".red().bold());
                        eprintln!("   {}", msg);
                        eprintln!(
                            "💡 This usually indicates network issues. Try downloading again."
                        );
                    }
                    _ => {
                        eprintln!("\n{} Error: {}", "❌".red().bold(), e);
                        eprintln!("💡 Try running the command again to resume the download");
                    }
                }

                return Err(anyhow::Error::from(e));
            }
        }
    }

    Ok(())
}

/// Build the CLI interface
fn build_cli() -> Command {
    Command::new("ia-get")
        .version(env!("CARGO_PKG_VERSION"))
        .about("Download files from the Internet Archive")
        .long_about("A CLI tool for downloading files from the Internet Archive with comprehensive metadata support, resume functionality, and progress tracking.")
        .arg(
            Arg::new("identifier")
                .help("Internet Archive identifier")
                .required(true)
                .value_name("IDENTIFIER")
                .index(1)
        )
        .arg(
            Arg::new("output")
                .short('o')
                .long("output")
                .help("Output directory")
                .value_name("DIR")
        )
        .arg(
            Arg::new("verbose")
                .short('v')
                .long("verbose")
                .help("Enable verbose output")
                .action(ArgAction::SetTrue)
        )
        .arg(
            Arg::new("dry-run")
                .long("dry-run")
                .help("Show what would be downloaded without actually downloading")
                .action(ArgAction::SetTrue)
        )
        .arg(
            Arg::new("concurrent")
                .short('c')
                .long("concurrent")
                .help("Number of concurrent downloads (1-16)")
                .value_name("NUM")
                .default_value("4")
        )
        .arg(
            Arg::new("include")
                .short('i')
                .long("include")
                .help("Include only files with these formats (can be used multiple times)")
                .value_name("FORMAT")
                .action(ArgAction::Append)
        )
        .arg(
            Arg::new("max-size")
                .long("max-size")
                .help("Maximum file size to download (e.g., 100MB, 1GB)")
                .value_name("SIZE")
        )
        .arg(
            Arg::new("no-compress")
                .long("no-compress")
                .help("Disable HTTP compression during downloads (compression is enabled by default)")
                .action(ArgAction::SetTrue)
        )
        .arg(
            Arg::new("decompress")
                .long("decompress")
                .help("Automatically decompress downloaded files")
                .action(ArgAction::SetTrue)
        )
        .arg(
            Arg::new("decompress-formats")
                .long("decompress-formats")
                .help("Compression formats to auto-decompress (comma-separated: gzip,bzip2,xz,tar)")
                .value_name("FORMATS")
                .value_delimiter(',')
                .action(ArgAction::Append)
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_parsing() {
        let cmd = build_cli();

        // Test basic usage
        let matches = cmd
            .clone()
            .try_get_matches_from(vec!["ia-get", "test-archive"])
            .unwrap();
        assert_eq!(
            matches.get_one::<String>("identifier").unwrap(),
            "test-archive"
        );

        // Test with options
        let matches = cmd
            .try_get_matches_from(vec![
                "ia-get",
                "test-archive",
                "--verbose",
                "--concurrent",
                "8",
                "--include",
                "pdf",
                "--include",
                "txt",
            ])
            .unwrap();

        assert_eq!(
            matches.get_one::<String>("identifier").unwrap(),
            "test-archive"
        );
        assert!(matches.get_flag("verbose"));
        assert_eq!(matches.get_one::<String>("concurrent").unwrap(), "8");

        let includes: Vec<_> = matches.get_many::<String>("include").unwrap().collect();
        assert_eq!(includes, vec!["pdf", "txt"]);
    }
}
