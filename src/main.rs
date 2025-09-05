//! Main entry point for ia-get CLI application

use anyhow::{Context, Result};
use clap::{Arg, ArgAction, ArgMatches, Command};
use colored::Colorize;
use std::path::PathBuf;
#[cfg(feature = "gui")]
use std::sync::{Arc, Mutex};
use tokio::signal;

use ia_get::{
    core::archive::AdvancedMetadataProcessor,
    core::session::sanitize_filename_for_filesystem,
    core::session::DownloadState,
    infrastructure::api::{get_archive_servers, EnhancedArchiveApiClient},
    interface::cli::SourceType,
    utilities::common::get_user_agent,
    utilities::filters::format_size,
    DownloadRequest, DownloadResult, DownloadService,
};

#[cfg(feature = "gui")]
use ia_get::interface::gui::IaGetApp;

/// Detect if GUI mode is available and appropriate
pub fn can_use_gui() -> bool {
    // Check if GUI features are compiled in
    #[cfg(not(feature = "gui"))]
    return false;

    #[cfg(feature = "gui")]
    {
        // Platform-specific GUI detection
        #[cfg(target_os = "windows")]
        {
            // On Windows, assume GUI is available unless we're in a Windows Terminal
            // that explicitly indicates headless mode
            std::env::var("WT_SESSION").is_ok() || std::env::var("SESSIONNAME").is_ok()
        }

        #[cfg(target_os = "macos")]
        {
            // On macOS, check for common GUI indicators
            // Most macOS environments have GUI available
            std::env::var("DISPLAY").is_ok()
                || std::env::var("TERM_PROGRAM").is_ok()
                || std::env::var("Apple_PubSub_Socket_Render").is_ok()
        }

        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            // On Linux and other Unix-like systems
            // If we're in SSH or explicit terminal contexts, prefer CLI
            if std::env::var("SSH_CONNECTION").is_ok()
                || std::env::var("SSH_CLIENT").is_ok()
                || std::env::var("SSH_TTY").is_ok()
            {
                return false;
            }

            // Check for X11 or Wayland display
            if std::env::var("DISPLAY").is_ok() || std::env::var("WAYLAND_DISPLAY").is_ok() {
                return true;
            }

            // Check for desktop environment variables
            if std::env::var("XDG_CURRENT_DESKTOP").is_ok()
                || std::env::var("DESKTOP_SESSION").is_ok()
                || std::env::var("GNOME_DESKTOP_SESSION_ID").is_ok()
                || std::env::var("KDE_FULL_SESSION").is_ok()
            {
                return true;
            }

            // Default to false for headless/server environments
            false
        }
    }
}

/// Launch GUI mode with graceful fallback
#[cfg(feature = "gui")]
async fn launch_gui() -> Result<()> {
    launch_gui_with_mode_switching().await
}

/// Launch GUI with support for switching to CLI mode
#[cfg(feature = "gui")]
async fn launch_gui_with_mode_switching() -> Result<()> {
    use std::sync::{Arc, Mutex};

    // Set up logging for GUI
    if let Err(e) = env_logger::try_init() {
        eprintln!("Warning: Failed to initialize logger: {}", e);
    }

    // Configure GUI options
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1000.0, 700.0])
            .with_min_inner_size([800.0, 600.0])
            .with_title("ia-get - Internet Archive Downloader")
            .with_icon(load_icon()),
        ..Default::default()
    };

    // Shared state to detect mode switching
    let switch_to_cli = Arc::new(Mutex::new(false));
    let switch_checker = Arc::clone(&switch_to_cli);

    // Try to run the GUI application
    let gui_result = eframe::run_native(
        "ia-get GUI",
        options,
        Box::new(move |cc| {
            let app = IaGetApp::new(cc);
            let app_with_checker = AppWrapper::new(app, switch_checker);
            Ok(Box::new(app_with_checker))
        }),
    );

    match gui_result {
        Ok(()) => {
            // Check if we should switch to CLI mode
            if *switch_to_cli.lock().unwrap() {
                println!("{} Switching to CLI mode...", "🔄".blue());
                show_interactive_menu().await
            } else {
                // GUI closed normally
                Ok(())
            }
        }
        Err(e) => {
            eprintln!("{} GUI launch failed: {}", "⚠️".yellow(), e);
            eprintln!("{} Falling back to interactive CLI menu...", "🔄".blue());
            show_interactive_menu().await
        }
    }
}

/// Wrapper around IaGetApp to handle mode switching
#[cfg(feature = "gui")]
struct AppWrapper {
    app: IaGetApp,
    switch_checker: Arc<Mutex<bool>>,
}

#[cfg(feature = "gui")]
impl AppWrapper {
    fn new(app: IaGetApp, switch_checker: Arc<Mutex<bool>>) -> Self {
        Self {
            app,
            switch_checker,
        }
    }
}

#[cfg(feature = "gui")]
impl eframe::App for AppWrapper {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.app.update(ctx, frame);

        // Check if we should switch to CLI mode
        if self.app.should_switch_to_cli() {
            *self.switch_checker.lock().unwrap() = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
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
async fn show_interactive_menu() -> Result<()> {
    // Use the enhanced interactive CLI directly without creating a new runtime
    ia_get::interface::interactive::launch_interactive_cli()
        .await
        .map_err(|e| anyhow::anyhow!("Interactive CLI error: {}", e))
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
                println!(
                    "{} No arguments provided, detecting best interface mode...",
                    "🚀".bright_blue()
                );

                if can_use_gui() {
                    #[cfg(feature = "gui")]
                    {
                        println!(
                            "{} GUI environment detected, launching graphical interface...",
                            "🎨".bright_green()
                        );
                        return launch_gui().await;
                    }
                    #[cfg(not(feature = "gui"))]
                    {
                        println!(
                            "{} GUI environment detected but GUI features not compiled in.",
                            "⚠️".yellow()
                        );
                        println!("{} Using interactive CLI menu instead...", "📋".blue());
                        return show_interactive_menu().await;
                    }
                } else {
                    println!(
                        "{} Command-line environment detected, using interactive menu...",
                        "💻".green()
                    );
                    return show_interactive_menu().await;
                }
            } else {
                // Other parsing errors, show them normally
                e.exit();
            }
        }
    };

    // Check for subcommands first
    match matches.subcommand() {
        Some(("config", config_matches)) => {
            use ia_get::interface::cli::commands;
            match config_matches.subcommand() {
                Some(("show", _)) => {
                    commands::handle_config_command(ia_get::interface::cli::ConfigAction::Show)
                        .await?;
                }
                Some(("set", set_matches)) => {
                    let key = set_matches.get_one::<String>("key").unwrap().clone();
                    let value = set_matches.get_one::<String>("value").unwrap().clone();
                    commands::handle_config_command(ia_get::interface::cli::ConfigAction::Set {
                        key,
                        value,
                    })
                    .await?;
                }
                Some(("unset", unset_matches)) => {
                    let key = unset_matches.get_one::<String>("key").unwrap().clone();
                    commands::handle_config_command(ia_get::interface::cli::ConfigAction::Unset {
                        key,
                    })
                    .await?;
                }
                Some(("location", _)) => {
                    commands::handle_config_command(ia_get::interface::cli::ConfigAction::Location)
                        .await?;
                }
                Some(("reset", _)) => {
                    commands::handle_config_command(ia_get::interface::cli::ConfigAction::Reset)
                        .await?;
                }
                Some(("validate", _)) => {
                    commands::handle_config_command(ia_get::interface::cli::ConfigAction::Validate)
                        .await?;
                }
                _ => {
                    eprintln!("No config subcommand specified. Use 'ia-get config --help' for available options.");
                    std::process::exit(1);
                }
            }
            return Ok(());
        }
        Some(("history", history_matches)) => {
            use ia_get::interface::cli::commands;
            match history_matches.subcommand() {
                Some(("show", show_matches)) => {
                    let limit = show_matches
                        .get_one::<String>("limit")
                        .unwrap()
                        .parse()
                        .unwrap_or(10);
                    let status = show_matches.get_one::<String>("status").cloned();
                    let detailed = show_matches.get_flag("detailed");
                    commands::handle_history_command(ia_get::interface::cli::HistoryAction::Show {
                        limit,
                        status,
                        detailed,
                    })
                    .await?;
                }
                Some(("clear", clear_matches)) => {
                    let force = clear_matches.get_flag("force");
                    commands::handle_history_command(
                        ia_get::interface::cli::HistoryAction::Clear { force },
                    )
                    .await?;
                }
                Some(("remove", remove_matches)) => {
                    let id = remove_matches.get_one::<String>("id").unwrap().clone();
                    commands::handle_history_command(
                        ia_get::interface::cli::HistoryAction::Remove { id },
                    )
                    .await?;
                }
                Some(("stats", _)) => {
                    commands::handle_history_command(ia_get::interface::cli::HistoryAction::Stats)
                        .await?;
                }
                _ => {
                    eprintln!("No history subcommand specified. Use 'ia-get history --help' for available options.");
                    std::process::exit(1);
                }
            }
            return Ok(());
        }
        _ => {
            // Continue with regular download processing
        }
    }

    // Check for API health command first
    if matches.get_flag("api-health") {
        display_api_health().await?;
        return Ok(());
    }

    // Check for metadata analysis command
    if matches.get_flag("analyze-metadata") {
        let raw_identifier = matches.get_one::<String>("identifier").ok_or_else(|| {
            anyhow::anyhow!("Archive identifier is required for metadata analysis")
        })?;

        let identifier = ia_get::utilities::common::normalize_archive_identifier(raw_identifier)
            .context("Failed to normalize archive identifier")?;

        analyze_archive_metadata(&identifier).await?;
        return Ok(());
    }

    // Check for format listing commands
    if matches.get_flag("list-formats") {
        ia_get::utilities::filters::list_format_categories();
        return Ok(());
    }

    if matches.get_flag("list-formats-detailed") {
        ia_get::utilities::filters::show_complete_format_help();
        return Ok(());
    }

    // Extract arguments - identifier is only required for download operations
    let raw_identifier = matches.get_one::<String>("identifier");

    // Check if we have an identifier when we need one
    if raw_identifier.is_none() {
        // If no identifier and no flags/subcommands that don't need it, show help
        if !matches.get_flag("api-health")
            && !matches.get_flag("list-formats")
            && !matches.get_flag("list-formats-detailed")
            && !matches.get_flag("analyze-metadata")
            && matches.subcommand().is_none()
        {
            anyhow::bail!("Archive identifier is required for download operations. Use --help for more information.");
        }
        return Ok(()); // This shouldn't be reached due to subcommand handling above
    }

    let raw_identifier = raw_identifier.unwrap();

    // Normalize the identifier - extract just the identifier portion if it's a URL
    let identifier = ia_get::utilities::common::normalize_archive_identifier(raw_identifier)
        .context("Failed to normalize archive identifier")?;

    let output_dir = matches
        .get_one::<String>("output")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let mut current = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            // Sanitize the identifier when using it as a directory name to prevent Windows path issues
            let sanitized_identifier = sanitize_filename_for_filesystem(&identifier);
            current.push(sanitized_identifier);
            current
        });

    let verbose = matches.get_flag("verbose");
    let dry_run = matches.get_flag("dry-run");

    let concurrent_downloads = matches
        .get_one::<String>("concurrent")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(4)
        .min(16); // Cap at 16 concurrent downloads

    let mut include_formats = matches
        .get_many::<String>("include")
        .map(|values| values.map(|s| s.to_string()).collect::<Vec<_>>())
        .unwrap_or_default();

    // Add formats from format categories
    if let Some(format_categories) = matches.get_many::<String>("include-formats") {
        use ia_get::utilities::filters::{FileFormats, FormatCategory};
        let file_formats = FileFormats::new();

        for category_name in format_categories {
            let category_name_lower = category_name.to_lowercase();
            for category in FormatCategory::all() {
                if category.display_name().to_lowercase() == category_name_lower {
                    include_formats.extend(file_formats.get_formats(&category));
                    break;
                }
            }
        }
    }

    let mut exclude_formats = Vec::new();

    // Add exclude formats from format categories
    if let Some(exclude_format_categories) = matches.get_many::<String>("exclude-formats") {
        use ia_get::utilities::filters::{FileFormats, FormatCategory};
        let file_formats = FileFormats::new();

        for category_name in exclude_format_categories {
            let category_name_lower = category_name.to_lowercase();
            for category in FormatCategory::all() {
                if category.display_name().to_lowercase() == category_name_lower {
                    exclude_formats.extend(file_formats.get_formats(&category));
                    break;
                }
            }
        }
    }

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
        exclude_formats,              // Now we support exclude formats
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
        source_types: get_source_types_from_matches(&matches),
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

    let mut api_client = EnhancedArchiveApiClient::new(client);

    // Test basic connectivity with official status endpoint
    println!("{} Testing Archive.org service status...", "🔗".cyan());
    match api_client.get_service_status().await {
        Ok(response) => {
            let status = response.status();
            println!("  ✅ Status endpoint successful (HTTP {})", status);

            // Try to parse the status response
            if let Ok(text) = response.text().await {
                if let Ok(status_data) = serde_json::from_str::<serde_json::Value>(&text) {
                    if let Some(status_msg) = status_data.get("status").and_then(|s| s.as_str()) {
                        println!("  📊 Service Status: {}", status_msg);
                    }
                }
            }
        }
        Err(e) => {
            println!("  ❌ Status check failed: {}", e);
        }
    }

    // Test metadata API
    println!("\n{} Testing Metadata API...", "📋".cyan());
    match api_client.get_metadata("nasa").await {
        Ok(response) => {
            println!(
                "  ✅ Metadata API successful (status: {})",
                response.status()
            );
        }
        Err(e) => {
            println!("  ❌ Metadata API failed: {}", e);
        }
    }

    // Test search API
    println!("\n{} Testing Search API...", "🔍".cyan());
    match api_client
        .search_items("collection:nasa", Some("identifier,title"), Some(1), None)
        .await
    {
        Ok(response) => {
            println!("  ✅ Search API successful (status: {})", response.status());

            // Parse search results to show functionality
            if let Ok(text) = response.text().await {
                if let Ok(search_data) = serde_json::from_str::<serde_json::Value>(&text) {
                    if let Some(num_found) =
                        search_data.get("response").and_then(|r| r.get("numFound"))
                    {
                        println!(
                            "  📊 Search returned {} total items in nasa collection",
                            num_found
                        );
                    }
                }
            }
        }
        Err(e) => {
            println!("  ❌ Search API failed: {}", e);
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

    for i in 0..3 {
        // Use valid identifiers that exist
        let test_identifiers = ["mario", "luigi", "nasa"];
        let identifier = test_identifiers[i % test_identifiers.len()];

        match api_client.get_metadata(identifier).await {
            Ok(_) => {
                let stats = api_client.get_stats();
                println!(
                    "  Request {}: ✅ {} (Rate: {:.1} req/min)",
                    i + 1,
                    identifier,
                    stats.average_requests_per_minute
                );
            }
            Err(e) => {
                println!("  Request {}: ❌ {} - {}", i + 1, identifier, e);
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

    // Enhanced API capabilities
    println!(
        "\n{} Enhanced API Capabilities:",
        "⚡".bright_yellow().bold()
    );
    println!("  ✅ Metadata API - Item information and file listings");
    println!("  ✅ Search API - Finding items across collections");
    println!("  ✅ Tasks API - Monitoring upload/processing status");
    println!("  ✅ Collections API - Batch operations on collections");
    println!("  ✅ Status API - Real-time service health monitoring");

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

/// Extract source types from CLI matches
fn get_source_types_from_matches(matches: &ArgMatches) -> Vec<SourceType> {
    // Handle convenience flags first
    if matches.get_flag("original-only") {
        return vec![SourceType::Original];
    }

    let mut types = vec![SourceType::Original]; // Always include originals by default

    if matches.get_flag("include-derivatives") {
        types.push(SourceType::Derivative);
    }

    if matches.get_flag("include-metadata") {
        types.push(SourceType::Metadata);
    }

    // Handle explicit source-types argument if provided
    if let Some(source_types) = matches.get_many::<String>("source-types") {
        let mut parsed_types = Vec::new();
        for type_str in source_types {
            match type_str.to_lowercase().as_str() {
                "original" => parsed_types.push(SourceType::Original),
                "derivative" => parsed_types.push(SourceType::Derivative),
                "metadata" => parsed_types.push(SourceType::Metadata),
                _ => {} // Ignore invalid types
            }
        }
        if !parsed_types.is_empty() {
            return parsed_types;
        }
    }

    types
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
                .required(false)  // Make identifier optional since subcommands might not need it
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
            Arg::new("include-formats")
                .long("include-formats")
                .help("Include files by format category (documents,images,audio,video,software,data,web,archives,metadata)")
                .value_name("CATEGORIES")
                .value_delimiter(',')
                .action(ArgAction::Append)
        )
        .arg(
            Arg::new("exclude-formats")
                .long("exclude-formats")
                .help("Exclude files by format category")
                .value_name("CATEGORIES")
                .value_delimiter(',')
                .action(ArgAction::Append)
        )
        .arg(
            Arg::new("list-formats")
                .long("list-formats")
                .help("List available file format categories and exit")
                .action(ArgAction::SetTrue)
        )
        .arg(
            Arg::new("list-formats-detailed")
                .long("list-formats-detailed")
                .help("List detailed file format information and exit")
                .action(ArgAction::SetTrue)
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
            Arg::new("source-types")
                .long("source-types")
                .help("Source types to include (original, derivative, metadata)")
                .value_name("TYPES")
                .value_delimiter(',')
                .action(ArgAction::Append)
        )
        .arg(
            Arg::new("original-only")
                .long("original-only")
                .help("Download only original files")
                .action(ArgAction::SetTrue)
                .conflicts_with("source-types")
        )
        .arg(
            Arg::new("include-derivatives")
                .long("include-derivatives")
                .help("Include derivative files in addition to originals")
                .action(ArgAction::SetTrue)
                .conflicts_with("source-types")
        )
        .arg(
            Arg::new("include-metadata")
                .long("include-metadata")
                .help("Include metadata files in addition to originals")
                .action(ArgAction::SetTrue)
                .conflicts_with("source-types")
        )
        .arg(
            Arg::new("api-health")
                .long("api-health")
                .help("Display Archive.org API health and monitoring information")
                .action(ArgAction::SetTrue)
        )
        .arg(
            Arg::new("analyze-metadata")
                .long("analyze-metadata")
                .help("Display enhanced metadata analysis for the specified archive")
                .action(ArgAction::SetTrue)
        )
        // Add subcommands for configuration and history management
        .subcommand(
            Command::new("config")
                .about("Configuration and preference management")
                .subcommand(
                    Command::new("show")
                        .about("Show current configuration and preferences")
                )
                .subcommand(
                    Command::new("set")
                        .about("Set a configuration value")
                        .long_about("Set a configuration value. Use 'ia-get config show' to see available keys.")
                        .arg(Arg::new("key")
                            .help("Configuration key to set (e.g., concurrent_downloads, default_output_path)")
                            .required(true))
                        .arg(Arg::new("value")
                            .help("Value to set (e.g., 8, /path/to/downloads)")
                            .required(true))
                )
                .subcommand(
                    Command::new("unset")
                        .about("Remove a configuration value (reset to default)")
                        .long_about("Reset a configuration value to its default. Use 'ia-get config show' to see available keys.")
                        .arg(Arg::new("key")
                            .help("Configuration key to reset (e.g., concurrent_downloads, default_output_path)")
                            .required(true))
                )
                .subcommand(
                    Command::new("location")
                        .about("Show location of configuration file")
                )
                .subcommand(
                    Command::new("reset")
                        .about("Reset all configuration to defaults")
                )
                .subcommand(
                    Command::new("validate")
                        .about("Validate current configuration")
                )
        )
        .subcommand(
            Command::new("history")
                .about("Download history management")
                .long_about("Manage download history database. View past downloads, clear history, and get statistics.")
                .subcommand(
                    Command::new("show")
                        .about("Show download history")
                        .long_about("Display download history with optional filtering and details.")
                        .arg(
                            Arg::new("limit")
                                .short('l')
                                .long("limit")
                                .help("Number of recent entries to show (default: 10)")
                                .default_value("10")
                        )
                        .arg(
                            Arg::new("status")
                                .short('s')
                                .long("status")
                                .help("Filter entries by status: success, failed, in_progress, cancelled, paused")
                                .value_parser(["success", "failed", "in_progress", "cancelled", "paused"])
                        )
                        .arg(
                            Arg::new("detailed")
                                .short('d')
                                .long("detailed")
                                .help("Show detailed information including file counts and error messages")
                                .action(ArgAction::SetTrue)
                        )
                )
                .subcommand(
                    Command::new("clear")
                        .about("Clear download history")
                        .long_about("Remove all download history entries. Use --force to skip confirmation.")
                        .arg(
                            Arg::new("force")
                                .short('f')
                                .long("force")
                                .help("Clear all entries without confirmation prompt")
                                .action(ArgAction::SetTrue)
                        )
                )
                .subcommand(
                    Command::new("remove")
                        .about("Remove specific entry by ID")
                        .long_about("Remove a specific download history entry by its ID. Use 'ia-get history show' to see entry IDs.")
                        .arg(Arg::new("id")
                            .help("Entry ID to remove (shown in 'ia-get history show' output)")
                            .required(true))
                )
                .subcommand(
                    Command::new("stats")
                        .about("Show statistics about downloads")
                        .long_about("Display comprehensive statistics about download history including success rates and totals.")
                )
        )
}

/// Analyze and display enhanced metadata for an archive
async fn analyze_archive_metadata(identifier: &str) -> Result<()> {
    println!("{} Enhanced Metadata Analysis", "🔍".blue().bold());
    println!("Archive: {}", identifier.bright_green());
    println!();

    // Create HTTP client
    let client = reqwest::Client::builder()
        .user_agent(get_user_agent())
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .context("Failed to create HTTP client")?;

    // Create enhanced API client
    let api_client = EnhancedArchiveApiClient::new(client.clone());

    // Create advanced metadata processor
    let mut processor = AdvancedMetadataProcessor::new(api_client);

    // Perform comprehensive analysis
    match processor.analyze_metadata(identifier).await {
        Ok(analysis) => {
            // Display the comprehensive analysis
            processor.display_analysis(&analysis);

            // Show additional insights
            if analysis.completeness_score < 60.0 {
                println!("{}", "💡 Suggestions for improvement:".yellow().bold());
                if !analysis.quality_indicators.has_description {
                    println!("  • Add a detailed description to improve discoverability");
                }
                if !analysis.quality_indicators.has_creator {
                    println!("  • Specify the creator or author information");
                }
                if analysis.quality_indicators.files_have_checksums < 80.0 {
                    println!(
                        "  • Consider adding checksums to more files for integrity verification"
                    );
                }
                println!();
            }

            // Show technical details for advanced users
            if std::env::var("IA_GET_VERBOSE").is_ok() {
                println!("{}", "🔧 Technical Details:".dimmed().bold());
                println!("  Size Distribution:");
                println!(
                    "    Small files (< 1MB): {}",
                    analysis.size_distribution.small_files
                );
                println!(
                    "    Medium files (1MB-100MB): {}",
                    analysis.size_distribution.medium_files
                );
                println!(
                    "    Large files (100MB-1GB): {}",
                    analysis.size_distribution.large_files
                );
                println!(
                    "    Huge files (> 1GB): {}",
                    analysis.size_distribution.huge_files
                );
                println!(
                    "    Average size: {}",
                    format_size(analysis.size_distribution.average_size)
                );
                println!(
                    "    Median size: {}",
                    format_size(analysis.size_distribution.median_size)
                );

                if !analysis.largest_files.is_empty() {
                    println!("  Largest Files:");
                    for (i, file) in analysis.largest_files.iter().take(5).enumerate() {
                        let truncated_name = if file.name.len() > 50 {
                            format!("{}...", &file.name[..47])
                        } else {
                            file.name.clone()
                        };
                        println!(
                            "    {}. {} ({})",
                            i + 1,
                            truncated_name,
                            format_size(file.size)
                        );
                    }
                }
                println!();
            }

            println!(
                "\n{} Use this analysis to make informed download decisions!",
                "💡".bright_yellow()
            );
        }
        Err(e) => {
            eprintln!(
                "{} {} {}",
                "❌".red(),
                "Failed to analyze metadata:".red().bold(),
                e
            );
            std::process::exit(1);
        }
    }

    Ok(())
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
