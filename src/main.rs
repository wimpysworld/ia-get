//! Main entry point for ia-get CLI application

use anyhow::{Context, Result};
use clap::{Arg, ArgAction, Command};
use colored::Colorize;
use std::path::PathBuf;
use tokio::signal;

use ia_get::{
    archive_api::{get_archive_servers, ArchiveOrgApiClient},
    constants::get_user_agent,
    filters::format_size,
    metadata_storage::DownloadState,
    DownloadRequest, DownloadResult, DownloadService,
};

/// Detect if GUI mode is available and appropriate
fn can_use_gui() -> bool {
    // Check if GUI features are compiled in
    #[cfg(not(feature = "gui"))]
    return false;

    #[cfg(feature = "gui")]
    {
        // Check if we have a display available
        if std::env::var("DISPLAY").is_err() && std::env::var("WAYLAND_DISPLAY").is_err() {
            // On Windows, we don't need DISPLAY
            #[cfg(not(target_os = "windows"))]
            return false;
        }

        // Check if we're in a TTY (interactive terminal)
        // For now, just check if we have environment variables that suggest interactivity
        if std::env::var("TERM").is_err() {
            return false;
        }

        true
    }
}

/// Launch GUI mode
#[cfg(feature = "gui")]
async fn launch_gui() -> Result<()> {
    use ia_get::gui::IaGetApp;

    // Set up logging for GUI
    if let Err(e) = env_logger::try_init() {
        eprintln!("Warning: Failed to initialize logger: {}", e);
    }

    // Create a tokio runtime for async operations
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("Error: Failed to create Tokio runtime: {}", e);
            eprintln!("This error might occur if the system cannot create threads.");
            return Err(e.into());
        }
    };

    // Configure GUI options
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1000.0, 700.0])
            .with_min_inner_size([800.0, 600.0])
            .with_title("ia-get - Internet Archive Downloader")
            .with_icon(load_icon()),
        ..Default::default()
    };

    // Enter the async runtime context and run the GUI
    let _guard = rt.enter();

    // Run the GUI application
    eframe::run_native(
        "ia-get GUI",
        options,
        Box::new(|cc| Ok(Box::new(IaGetApp::new(cc)))),
    )
    .map_err(|e| {
        eprintln!("Error starting GUI: {}", e);
        eprintln!("This might be due to missing graphics drivers or display issues.");
        anyhow::anyhow!("Failed to start GUI: {}", e)
    })
}

#[cfg(feature = "gui")]
fn load_icon() -> egui::IconData {
    // Create a simple icon (you can replace this with an actual icon file)
    let icon_data = vec![
        255, 255, 255, 255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255, 255, 0, 0, 0, 255, 255, 255,
        255, 255, 255, 255, 255, 255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255, 255, 255, 255,
        255, 255, 0, 0, 0, 255, 255, 255, 255, 255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255, 255,
    ];

    egui::IconData {
        rgba: icon_data,
        width: 4,
        height: 4,
    }
}

/// Show an interactive menu when no arguments are provided
fn show_interactive_menu() -> Result<()> {
    println!(
        "{}",
        "🚀 ia-get - Internet Archive Downloader"
            .bright_cyan()
            .bold()
    );
    println!();
    println!("No arguments provided. Choose an option:");
    println!();
    println!("  {} Launch GUI interface", "1.".bright_green());
    println!("  {} Show command-line help", "2.".bright_green());
    println!("  {} Exit", "3.".bright_green());
    println!();
    print!("Enter your choice (1-3): ");

    use std::io::{self, Write};
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    match input.trim() {
        "1" => {
            #[cfg(feature = "gui")]
            {
                if can_use_gui() {
                    println!("{} Launching GUI...", "🎨".bright_blue());
                    // We need to switch to async context for GUI
                    let rt = tokio::runtime::Runtime::new()?;
                    rt.block_on(launch_gui())?;
                } else {
                    println!("{} GUI not available in this environment.", "⚠️".yellow());
                    println!("Try running with command-line arguments instead.");
                    println!("Use 'ia-get --help' for more information.");
                }
            }
            #[cfg(not(feature = "gui"))]
            {
                println!("{} GUI not compiled in this build.", "⚠️".yellow());
                println!("Try running with command-line arguments instead.");
                println!("Use 'ia-get --help' for more information.");
            }
        }
        "2" => {
            build_cli().print_help()?;
        }
        "3" => {
            println!("Goodbye! 👋");
            return Ok(());
        }
        _ => {
            println!(
                "{} Invalid choice. Use 'ia-get --help' for command-line usage.",
                "⚠️".yellow()
            );
            return Err(anyhow::anyhow!("Invalid menu choice"));
        }
    }

    Ok(())
}

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
    let matches = build_cli().try_get_matches();

    // Handle parsing errors gracefully
    let matches = match matches {
        Ok(matches) => matches,
        Err(e) => {
            // Check if this is a "missing arguments" error and we have no args at all
            let args: Vec<String> = std::env::args().collect();
            if args.len() == 1 {
                // No arguments provided - use smart detection
                if can_use_gui() {
                    #[cfg(feature = "gui")]
                    {
                        println!(
                            "{} No arguments provided, launching GUI mode...",
                            "🎨".bright_blue()
                        );
                        return launch_gui().await;
                    }
                    #[cfg(not(feature = "gui"))]
                    {
                        return show_interactive_menu();
                    }
                } else {
                    return show_interactive_menu();
                }
            } else {
                // Other parsing errors, show them normally
                e.exit();
            }
        }
    };

    // Check for API health command first
    if matches.get_flag("api-health") {
        display_api_health().await?;
        return Ok(());
    }

    // Extract arguments - these are now guaranteed to be present due to CLI parsing
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

    let verbose = matches.get_flag("verbose");
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

    let max_file_size = matches.get_one::<String>("max-size").map(|s| s.to_string());

    // Compression settings - enable by default as requested
    let enable_compression = !matches.get_flag("no-compress"); // Default to true unless --no-compress is specified
    let auto_decompress = matches.get_flag("decompress");
    let decompress_formats = matches
        .get_many::<String>("decompress-formats")
        .map(|values| values.map(|s| s.to_string()).collect::<Vec<_>>())
        .unwrap_or_default();

    // Create unified download request
    let request = DownloadRequest {
        identifier: identifier.clone(),
        output_dir: output_dir.clone(),
        include_formats,
        exclude_formats: Vec::new(), // CLI doesn't support exclude yet, but unified API does
        min_file_size: String::new(), // CLI doesn't support min size yet, but unified API does
        max_file_size,
        concurrent_downloads,
        enable_compression,
        auto_decompress,
        decompress_formats,
        dry_run,
        verify_md5: true,
        preserve_mtime: true,
        verbose,
        resume: true,
    };

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

    // Create download service
    let service = DownloadService::new().context("Failed to create download service")?;

    // Execute download using unified API
    match service.download(request.clone(), None).await {
        Ok(DownloadResult::Success(session, api_stats)) => {
            if !dry_run {
                println!("\n{} Download completed successfully!", "✅".green().bold());
                println!(
                    "📁 Output directory: {}",
                    output_dir.display().to_string().bright_green()
                );
                DownloadService::display_download_summary(&session, &request);

                // Display Archive.org API statistics
                if let Some(stats) = api_stats {
                    println!("\n{} Archive.org API Usage:", "📊".blue().bold());
                    println!("  {}", stats);
                    if verbose {
                        println!(
                            "  Session healthy: {}",
                            if stats.average_requests_per_minute < 30.0 {
                                "✅ Yes"
                            } else {
                                "⚠️ High rate"
                            }
                        );
                    }
                }

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
            } else {
                // Display dry run results
                println!("\n{} Archive Information:", "📊".blue().bold());
                println!("  Identifier: {}", session.identifier);
                println!("  Total files: {}", session.archive_metadata.files.len());
                println!(
                    "  Archive size: {}",
                    format_size(session.archive_metadata.item_size)
                );
                println!("  Server: {}", session.archive_metadata.server);
                println!(
                    "  Available servers: {}",
                    session.archive_metadata.workable_servers.join(", ")
                );
                println!("  Directory: {}", session.archive_metadata.dir);

                println!("\n{} Files selected for download:", "📋".cyan().bold());
                println!("  Selected: {} files", session.requested_files.len());

                for (i, filename) in session.requested_files.iter().enumerate().take(10) {
                    println!(
                        "  {:<3} {}",
                        format!("{}.", i + 1).dimmed(),
                        filename.green()
                    );
                }
                if session.requested_files.len() > 10 {
                    println!(
                        "  ... and {} more files",
                        session.requested_files.len() - 10
                    );
                }

                println!("\n{} Use without --dry-run to download", "💡".yellow());

                // Display Archive.org API statistics for dry run too
                if let Some(stats) = api_stats {
                    println!("\n{} Archive.org API Usage:", "📊".blue().bold());
                    println!("  {}", stats);
                }
            }
        }
        Ok(DownloadResult::Error(error)) => {
            eprintln!("{} Error: {}", "✘".red().bold(), error);
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("{} Error: {}", "✘".red().bold(), e);
            std::process::exit(1);
        }
    }

    Ok(())
}

/// Display Archive.org API health and monitoring information
async fn display_api_health() -> Result<()> {
    println!("{} Archive.org API Health Status", "🏥".blue().bold());
    println!();

    // Create a test API client
    let client = reqwest::Client::builder()
        .user_agent(get_user_agent())
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .context("Failed to create HTTP client")?;

    let mut api_client = ArchiveOrgApiClient::new(client);

    // Test basic connectivity
    println!("{} Testing Archive.org connectivity...", "🔗".cyan());
    match api_client
        .make_request("https://archive.org/metadata/nasa")
        .await
    {
        Ok(response) => {
            println!("  ✅ Connection successful (status: {})", response.status());
        }
        Err(e) => {
            println!("  ❌ Connection failed: {}", e);
        }
    }

    // Display server list
    println!("\n{} Available Archive.org Servers:", "🌐".green().bold());
    let servers = get_archive_servers();
    for (i, server) in servers.iter().enumerate() {
        println!(
            "  {:<2} {}",
            format!("{}.", i + 1).dimmed(),
            server.bright_blue()
        );
    }

    // Test multiple requests to show rate limiting
    println!("\n{} Testing API rate limiting...", "⏱️".yellow());
    let _start_time = std::time::Instant::now();

    for i in 0..3 {
        let test_url = format!("https://archive.org/metadata/test{}", i);
        match api_client.make_request(&test_url).await {
            Ok(_) => {
                let stats = api_client.get_stats();
                println!(
                    "  Request {}: ✅ (Rate: {:.1} req/min)",
                    i + 1,
                    stats.average_requests_per_minute
                );
            }
            Err(e) => {
                println!("  Request {}: ❌ {}", i + 1, e);
            }
        }
    }

    // Display final statistics
    println!("\n{} API Session Statistics:", "📊".purple().bold());
    let final_stats = api_client.get_stats();
    println!("  {}", final_stats);

    // Health assessment
    println!("\n{} Health Assessment:", "🎯".bright_green().bold());
    if api_client.is_rate_healthy() {
        println!("  ✅ Request rate is healthy and Archive.org compliant");
    } else {
        println!("  ⚠️  Request rate is high - consider slowing down requests");
    }

    println!(
        "\n{} Archive.org API Guidelines:",
        "📋".bright_cyan().bold()
    );
    println!("  • Keep concurrent connections ≤ 5 for respectful usage");
    println!("  • Include descriptive User-Agent with contact information");
    println!("  • Implement retry logic for transient failures");
    println!("  • Honor rate limiting (429) and retry-after headers");
    println!("  • Use appropriate timeouts for large file downloads");

    println!("\n{} Current Configuration:", "⚙️".bright_magenta().bold());
    println!("  User Agent: {}", get_user_agent().bright_green());
    println!("  Default Timeout: 30 seconds");
    println!("  Min Request Delay: 100ms");
    println!("  Max Concurrent: 5 connections");

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
                .required_unless_present("api-health")
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
        .arg(
            Arg::new("api-health")
                .long("api-health")
                .help("Display Archive.org API health and monitoring information")
                .action(ArgAction::SetTrue)
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
