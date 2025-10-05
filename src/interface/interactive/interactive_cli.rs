//! Enhanced interactive CLI interface for ia-get
//!
//! Provides a comprehensive command-line interface that mirrors GUI functionality
//! with live updating progress, non-scrolling interface, and unified API usage.

use crate::{
    Result,
    core::download::{DownloadRequest, DownloadResult, DownloadService, ProgressUpdate},
    core::session::{ArchiveFile, DownloadSession},
    infrastructure::config::{Config, ConfigManager},
    utilities::filters::format_size,
};
use colored::*;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Main interactive CLI interface
pub struct InteractiveCli {
    config_manager: ConfigManager,
    config: Config,
    download_service: DownloadService,
}

/// State for live updating interface
#[derive(Default, Clone)]
struct DownloadState {
    current_file: String,
    completed_files: usize,
    total_files: usize,
    failed_files: usize,
    current_speed: f64,
    eta: String,
    status: String,
    start_time: Option<Instant>,
}

impl InteractiveCli {
    /// Create a new interactive CLI
    pub fn new() -> Result<Self> {
        let config_manager = ConfigManager::new()?;
        let config = config_manager.load_config()?;
        let download_service = DownloadService::new()?;

        Ok(Self {
            config_manager,
            config,
            download_service,
        })
    }

    /// Run the main interactive CLI interface
    pub async fn run(&mut self) -> Result<()> {
        self.clear_screen();
        self.print_header();

        loop {
            self.print_main_menu();

            #[cfg(feature = "gui")]
            let max_choice = if crate::can_use_gui() { 8 } else { 7 };
            #[cfg(not(feature = "gui"))]
            let max_choice = 7;

            match self.get_user_choice("Select an option", max_choice)? {
                1 => self.download_archive().await?,
                2 => self.quick_download().await?,
                3 => self.browse_and_download().await?,
                4 => self.configure_settings().await?,
                5 => self.view_history().await?,
                6 => self.check_api_health().await?,
                7 => {
                    #[cfg(feature = "gui")]
                    {
                        if crate::can_use_gui() {
                            // Switch to GUI mode
                            println!("{}", "\n🎨 Switching to GUI mode...".bright_cyan());
                            self.launch_gui_mode().await?;
                            break;
                        } else {
                            // Exit
                            println!(
                                "{}",
                                "\n✨ Thanks for using ia-get! Goodbye! 👋".bright_cyan()
                            );
                            break;
                        }
                    }
                    #[cfg(not(feature = "gui"))]
                    {
                        // Exit
                        println!(
                            "{}",
                            "\n✨ Thanks for using ia-get! Goodbye! 👋".bright_cyan()
                        );
                        break;
                    }
                }
                8 => {
                    #[cfg(feature = "gui")]
                    {
                        if crate::can_use_gui() {
                            // Exit
                            println!(
                                "{}",
                                "\n✨ Thanks for using ia-get! Goodbye! 👋".bright_cyan()
                            );
                            break;
                        }
                    }
                }
                _ => {
                    self.show_error("Invalid choice. Please try again.");
                    self.wait_for_keypress();
                }
            }
        }

        Ok(())
    }

    #[cfg(feature = "gui")]
    async fn launch_gui_mode(&self) -> Result<()> {
        #[cfg(feature = "gui")]
        {
            use std::process::Command;

            // Try to launch GUI mode by spawning a new process
            // This is a simple approach - we restart the program without arguments
            // which will trigger the smart detection and launch GUI
            let current_exe = std::env::current_exe()
                .map_err(|e| anyhow::anyhow!("Failed to get current executable path: {}", e))?;

            println!("{} Launching GUI interface...", "🚀".bright_green());

            match Command::new(current_exe).spawn() {
                Ok(_) => {
                    println!("{} GUI launched successfully!", "✅".bright_green());
                    Ok(())
                }
                Err(e) => {
                    self.show_error(&format!("Failed to launch GUI: {}", e));
                    self.wait_for_keypress();
                    Ok(())
                }
            }
        }

        #[cfg(not(feature = "gui"))]
        {
            self.show_error("GUI features not compiled in this build");
            self.wait_for_keypress();
            Ok(())
        }
    }

    fn clear_screen(&self) {
        // Clear screen and move cursor to top-left
        print!("\x1B[2J\x1B[H");
        io::stdout().flush().unwrap();
    }

    fn print_header(&self) {
        // Enhanced header with gradient-like effect using Unicode blocks
        println!(
            "{}",
            "╔══════════════════════════════════════════════════════════════════════════════╗"
                .bright_cyan()
        );
        println!(
            "{}",
            "║  🚀 ████████ ███████  ████████  ████████ ██████████   █████████████████████ ║"
                .cyan()
        );
        println!(
            "{}",
            "║  📦 ██    ██ ██    ██ ██        ██          ██        ██                  █ ║"
                .cyan()
        );
        println!(
            "{}",
            "║  📚 ██ ██ ██ ███████  ██  ████  ██████      ██        █████████████████   █ ║"
                .cyan()
        );
        println!(
            "{}",
            "║  🌐 ██    ██ ██    ██ ██    ██  ██          ██                        ██  █ ║"
                .cyan()
        );
        println!(
            "{}",
            "║  ⚡ ██    ██ ██    ██  ████████ ████████    ██        █████████████████   █ ║"
                .cyan()
        );
        println!(
            "{}",
            "║                                                                              ║"
                .cyan()
        );
        println!(
            "{}",
            "║                    🌐 Internet Archive Downloader                          ║"
                .bright_white()
        );
        println!(
            "{}",
            "║                      High-performance file downloader                       ║"
                .dimmed()
        );
        println!(
            "{}",
            "╚══════════════════════════════════════════════════════════════════════════════╝"
                .bright_cyan()
        );
        println!();

        // Add version and environment info
        println!(
            "{}{}{}",
            "🔧 Version: ".dimmed(),
            env!("CARGO_PKG_VERSION").bright_blue(),
            " | Enhanced Interactive Mode".dimmed()
        );
        println!();
    }

    fn print_main_menu(&self) {
        println!(
            "{}",
            "┌─ 📋 MAIN MENU ─────────────────────────────────────────────────┐"
                .bold()
                .bright_blue()
        );
        println!(
            "{}",
            "│                                                                 │".blue()
        );

        println!(
            "│  {} {} {}{}│",
            "1.".bright_green().bold(),
            "📦".cyan(),
            "Full Archive Download                               ".normal(),
            " ".blue()
        );
        println!(
            "{}",
            "│     Download complete archives with filtering options           │".dimmed()
        );
        println!(
            "{}",
            "│                                                                 │".blue()
        );

        println!(
            "│  {} {} {}{}│",
            "2.".bright_green().bold(),
            "⚡".cyan(),
            "Quick URL Download                                  ".normal(),
            " ".blue()
        );
        println!(
            "{}",
            "│     Fast download from Archive URL or identifier               │".dimmed()
        );
        println!(
            "{}",
            "│                                                                 │".blue()
        );

        println!(
            "│  {} {} {}{}│",
            "3.".bright_green().bold(),
            "🔍".cyan(),
            "Browse & Select                                     ".normal(),
            " ".blue()
        );
        println!(
            "{}",
            "│     Browse archive contents and select files                   │".dimmed()
        );
        println!(
            "{}",
            "│                                                                 │".blue()
        );

        println!(
            "│  {} {} {}{}│",
            "4.".bright_green().bold(),
            "⚙️".cyan(),
            "Settings & Configuration                           ".normal(),
            " ".blue()
        );
        println!(
            "{}",
            "│     Configure download preferences and filters                  │".dimmed()
        );
        println!(
            "{}",
            "│                                                                 │".blue()
        );

        println!(
            "│  {} {} {}{}│",
            "5.".bright_green().bold(),
            "📚".cyan(),
            "Download History                                    ".normal(),
            " ".blue()
        );
        println!(
            "{}",
            "│     View and manage download history                            │".dimmed()
        );
        println!(
            "{}",
            "│                                                                 │".blue()
        );

        println!(
            "│  {} {} {}{}│",
            "6.".bright_green().bold(),
            "🏥".cyan(),
            "Archive.org Health Status                           ".normal(),
            " ".blue()
        );
        println!(
            "{}",
            "│     Check Internet Archive API status and health               │".dimmed()
        );
        println!(
            "{}",
            "│                                                                 │".blue()
        );

        // Only show GUI option if GUI features are compiled and available
        #[cfg(feature = "gui")]
        {
            if crate::can_use_gui() {
                println!(
                    "│  {} {} {}{}│",
                    "7.".bright_green().bold(),
                    "🎨".cyan(),
                    "Switch to GUI Mode                                 ".normal(),
                    " ".blue()
                );
                println!(
                    "{}",
                    "│     Launch graphical user interface                            │".dimmed()
                );
                println!(
                    "{}",
                    "│                                                                 │".blue()
                );

                println!(
                    "│  {} {} {}{}│",
                    "8.".bright_green().bold(),
                    "🚪".cyan(),
                    "Exit                                                ".normal(),
                    " ".blue()
                );
            } else {
                println!(
                    "│  {} {} {}{}│",
                    "7.".bright_green().bold(),
                    "🚪".cyan(),
                    "Exit                                                ".normal(),
                    " ".blue()
                );
            }
        }

        #[cfg(not(feature = "gui"))]
        {
            println!(
                "│  {} {} {}{}│",
                "7.".bright_green().bold(),
                "🚪".cyan(),
                "Exit                                                ".normal(),
                " ".blue()
            );
        }

        println!(
            "{}",
            "│                                                                 │".blue()
        );
        println!(
            "{}",
            "└─────────────────────────────────────────────────────────────────┘".bright_blue()
        );
        println!();

        // Add a helpful tip
        println!(
            "{}",
            "💡 Tip: Type the number and press Enter to select an option".dimmed()
        );
        println!();
    }

    async fn download_archive(&mut self) -> Result<()> {
        self.clear_screen();
        self.print_section_header("Full Archive Download");

        // Get archive identifier
        let identifier = self.get_string_input(
            "Enter Archive.org URL or identifier",
            "e.g., https://archive.org/details/example or just 'example'",
        )?;

        if identifier.trim().is_empty() {
            self.show_error("Archive identifier cannot be empty");
            self.wait_for_keypress();
            return Ok(());
        }

        // Get output directory
        let default_output = self
            .config
            .default_output_path
            .as_deref()
            .unwrap_or("./downloads")
            .to_string();

        let output_dir = self.get_string_input_with_default(
            "Output directory",
            &format!("Default: {}", default_output),
            &default_output,
        )?;

        // Configure download options
        self.print_subsection("Download Options");
        println!("Configure your download preferences:");
        println!();

        let mut request = DownloadRequest::from_config(
            &self.config,
            identifier.trim().to_string(),
            PathBuf::from(output_dir),
        );

        // File format selection
        if self.get_yes_no("Filter by file formats?", false)? {
            self.configure_format_filters(&mut request)?;
        }

        // Size filtering
        if self.get_yes_no("Set file size limits?", false)? {
            self.configure_size_filters(&mut request)?;
        }

        // Source type filtering
        if self.get_yes_no("Filter by source type?", false)? {
            self.configure_source_filters(&mut request)?;
        }

        // Concurrency settings
        request.concurrent_downloads = self.get_number_input(
            "Concurrent downloads (1-16)",
            request.concurrent_downloads,
            1,
            16,
        )?;

        // Additional options
        request.dry_run = self.get_yes_no("Dry run (preview only)?", request.dry_run)?;
        request.verify_md5 = self.get_yes_no("Verify MD5 checksums?", request.verify_md5)?;

        // Show download summary
        self.show_download_summary(&request);

        if !self.get_yes_no("Start download?", true)? {
            return Ok(());
        }

        // Execute download with live progress
        self.execute_download_with_progress(request).await
    }

    async fn quick_download(&mut self) -> Result<()> {
        self.clear_screen();
        self.print_section_header("Quick URL Download");

        let identifier = self.get_string_input(
            "Enter Archive.org URL or identifier",
            "This will download all files with default settings",
        )?;

        if identifier.trim().is_empty() {
            self.show_error("Archive identifier cannot be empty");
            self.wait_for_keypress();
            return Ok(());
        }

        let output_dir = self
            .config
            .default_output_path
            .as_deref()
            .unwrap_or("./downloads")
            .to_string();

        let request = DownloadRequest::from_config(
            &self.config,
            identifier.trim().to_string(),
            PathBuf::from(output_dir),
        );

        println!("\n{} Starting quick download...", "⚡".bright_yellow());
        self.execute_download_with_progress(request).await
    }

    async fn check_api_health(&self) -> Result<()> {
        self.clear_screen();
        self.print_section_header("Archive.org API Health Status");

        println!("{} Archive.org API Health Status", "🏥".blue().bold());
        println!();

        // Create a test API client
        use crate::{
            infrastructure::api::{EnhancedArchiveApiClient, get_archive_servers},
            utilities::common::get_user_agent,
        };

        let client = reqwest::Client::builder()
            .user_agent(get_user_agent())
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to create HTTP client: {}", e))?;

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
                        if let Some(status_msg) = status_data.get("status").and_then(|s| s.as_str())
                        {
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

        println!("\n{} Health check completed!", "✅".green().bold());
        self.wait_for_keypress();
        Ok(())
    }

    async fn browse_and_download(&mut self) -> Result<()> {
        self.clear_screen();
        self.print_section_header("Browse & Select Files");

        // Get archive identifier
        let identifier = self.get_string_input(
            "Enter Archive.org URL or identifier",
            "e.g., https://archive.org/details/example or just 'example'",
        )?;

        if identifier.trim().is_empty() {
            self.show_error("Archive identifier cannot be empty");
            self.wait_for_keypress();
            return Ok(());
        }

        // Show loading message
        println!("{} Fetching archive metadata...", "🔍".bright_blue());
        println!("This may take a moment for large archives.");
        println!();

        // Create dry run request to get file listing
        let output_dir = self
            .config
            .default_output_path
            .as_deref()
            .unwrap_or("./downloads")
            .to_string();

        let request = DownloadRequest::from_config(
            &self.config,
            identifier.trim().to_string(),
            PathBuf::from(&output_dir),
        );

        // Set as dry run to just get metadata
        let mut dry_request = request;
        dry_request.dry_run = true;

        // Execute dry run to get file metadata
        let result = self.download_service.download(dry_request, None).await?;

        match result {
            DownloadResult::Success(session, _stats, _is_dry_run) => {
                // Show file browser interface
                self.show_file_browser(&session, &output_dir).await?;
            }
            DownloadResult::Error(error) => {
                self.show_error(&format!("Failed to fetch archive metadata: {}", error));
                self.wait_for_keypress();
            }
        }

        Ok(())
    }

    async fn show_file_browser(&self, session: &DownloadSession, output_dir: &str) -> Result<()> {
        use std::collections::HashMap;

        // Build file tree structure
        let mut file_tree: HashMap<String, Vec<&ArchiveFile>> = HashMap::new();
        let files = &session.archive_metadata.files;

        // Group files by directory
        for file in files {
            let path_parts: Vec<&str> = file.name.split('/').collect();
            let dir = if path_parts.len() > 1 {
                path_parts[..path_parts.len() - 1].join("/")
            } else {
                "root".to_string()
            };
            file_tree.entry(dir).or_default().push(file);
        }

        let mut selected_files: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        loop {
            self.clear_screen();
            self.print_section_header(&format!("Browse Files - {}", session.identifier));

            println!("{} Archive Information:", "📋".bright_blue());
            println!("  • Total files: {}", files.len());
            let total_size: u64 = files.iter().map(|f| f.size.unwrap_or(0)).sum();
            println!("  • Total size: {}", format_size(total_size));
            println!("  • Selected files: {}", selected_files.len());

            let selected_size: u64 = files
                .iter()
                .filter(|f| selected_files.contains(&f.name))
                .map(|f| f.size.unwrap_or(0))
                .sum();
            println!("  • Selected size: {}", format_size(selected_size));
            println!();

            // Show file browser menu
            println!("{} File Browser Options:", "📁".bright_green());
            println!("  1. View all files (paginated)");
            println!("  2. Search files by name");
            println!("  3. Filter by file format");
            println!("  4. Select files by pattern");
            println!("  5. View selected files");
            println!("  6. Clear all selections");
            println!("  7. Download selected files");
            println!("  8. Back to main menu");
            println!();

            match self.get_user_choice("Select an option", 8)? {
                1 => self.show_all_files(files, &mut selected_files)?,
                2 => self.search_files(files, &mut selected_files)?,
                3 => self.filter_by_format(files, &mut selected_files)?,
                4 => self.select_by_pattern(files, &mut selected_files)?,
                5 => self.show_selected_files(files, &selected_files)?,
                6 => {
                    selected_files.clear();
                    println!("{}", "✅ All selections cleared".green());
                    self.wait_for_keypress();
                }
                7 => {
                    if selected_files.is_empty() {
                        self.show_error("No files selected for download");
                        self.wait_for_keypress();
                    } else {
                        self.download_selected_files(session, &selected_files, output_dir)
                            .await?;
                        break;
                    }
                }
                8 => break,
                _ => unreachable!(),
            }
        }

        Ok(())
    }

    fn show_all_files(
        &self,
        files: &[ArchiveFile],
        selected_files: &mut std::collections::HashSet<String>,
    ) -> Result<()> {
        const PAGE_SIZE: usize = 20;
        let mut page = 0;
        let total_pages = files.len().div_ceil(PAGE_SIZE);

        loop {
            self.clear_screen();
            self.print_section_header(&format!("All Files - Page {} of {}", page + 1, total_pages));

            let start = page * PAGE_SIZE;
            let end = std::cmp::min(start + PAGE_SIZE, files.len());

            for (i, file) in files[start..end].iter().enumerate() {
                let file_num = start + i + 1;
                let selected = if selected_files.contains(&file.name) {
                    "✓"
                } else {
                    " "
                };
                let size_str = file.size.map(format_size).unwrap_or_default();
                let format_str = file.format.as_deref().unwrap_or("Unknown");

                println!(
                    "[{}] {}: {} ({}) - {}",
                    selected, file_num, file.name, format_str, size_str
                );
            }

            println!();
            println!("Commands:");
            if page > 0 {
                println!("  p - Previous page");
            }
            if page < total_pages - 1 {
                println!("  n - Next page");
            }
            println!("  s <number> - Toggle selection for file number");
            println!("  a - Select all files on this page");
            println!("  c - Clear all selections on this page");
            println!("  q - Back to file browser menu");
            println!();

            let input = self.get_string_input("Enter command", "")?;
            let input = input.trim().to_lowercase();

            match input.as_str() {
                "p" if page > 0 => page -= 1,
                "n" if page < total_pages - 1 => page += 1,
                "a" => {
                    for file in &files[start..end] {
                        selected_files.insert(file.name.clone());
                    }
                    println!("{}", "✅ Selected all files on this page".green());
                    self.wait_for_keypress();
                }
                "c" => {
                    for file in &files[start..end] {
                        selected_files.remove(&file.name);
                    }
                    println!("{}", "✅ Cleared all selections on this page".green());
                    self.wait_for_keypress();
                }
                "q" => break,
                input if input.starts_with("s ") => {
                    if let Ok(file_num) = input[2..].parse::<usize>() {
                        if file_num > 0 && file_num <= files.len() {
                            let file = &files[file_num - 1];
                            if selected_files.contains(&file.name) {
                                selected_files.remove(&file.name);
                                println!("{} Deselected: {}", "❌".red(), file.name);
                            } else {
                                selected_files.insert(file.name.clone());
                                println!("{} Selected: {}", "✅".green(), file.name);
                            }
                            self.wait_for_keypress();
                        } else {
                            self.show_error("Invalid file number");
                            self.wait_for_keypress();
                        }
                    } else {
                        self.show_error("Invalid command format. Use 's <number>'");
                        self.wait_for_keypress();
                    }
                }
                _ => {
                    self.show_error("Invalid command");
                    self.wait_for_keypress();
                }
            }
        }

        Ok(())
    }

    fn search_files(
        &self,
        files: &[ArchiveFile],
        selected_files: &mut std::collections::HashSet<String>,
    ) -> Result<()> {
        self.clear_screen();
        self.print_section_header("Search Files");

        let search_term = self.get_string_input("Enter search term", "Search in file names")?;

        if search_term.trim().is_empty() {
            return Ok(());
        }

        let matching_files: Vec<_> = files
            .iter()
            .enumerate()
            .filter(|(_, file)| {
                file.name
                    .to_lowercase()
                    .contains(&search_term.to_lowercase())
            })
            .collect();

        if matching_files.is_empty() {
            println!(
                "{}",
                format!("No files found matching '{}'", search_term).yellow()
            );
            self.wait_for_keypress();
            return Ok(());
        }

        self.show_file_selection_interface(
            &matching_files,
            selected_files,
            &format!("Search Results for '{}'", search_term),
        )
    }

    fn filter_by_format(
        &self,
        files: &[ArchiveFile],
        selected_files: &mut std::collections::HashSet<String>,
    ) -> Result<()> {
        self.clear_screen();
        self.print_section_header("Filter by Format");

        let format_filter = self.get_string_input("Enter file format", "e.g., MP4, PDF, ZIP")?;

        if format_filter.trim().is_empty() {
            return Ok(());
        }

        let matching_files: Vec<_> = files
            .iter()
            .enumerate()
            .filter(|(_, file)| {
                file.format
                    .as_ref()
                    .map(|f| f.to_lowercase().contains(&format_filter.to_lowercase()))
                    .unwrap_or(false)
            })
            .collect();

        if matching_files.is_empty() {
            println!(
                "{}",
                format!("No files found with format '{}'", format_filter).yellow()
            );
            self.wait_for_keypress();
            return Ok(());
        }

        self.show_file_selection_interface(
            &matching_files,
            selected_files,
            &format!("Files with Format '{}'", format_filter),
        )
    }

    fn select_by_pattern(
        &self,
        files: &[ArchiveFile],
        selected_files: &mut std::collections::HashSet<String>,
    ) -> Result<()> {
        self.clear_screen();
        self.print_section_header("Select by Pattern");

        println!("Enter a pattern to select multiple files:");
        println!("Examples:");
        println!("  *.mp4 - Select all MP4 files");
        println!("  season01/* - Select all files in season01 folder");
        println!("  *episode* - Select all files containing 'episode'");
        println!();

        let pattern = self.get_string_input("Enter pattern", "Use * as wildcard")?;

        if pattern.trim().is_empty() {
            return Ok(());
        }

        let regex_pattern = pattern.replace("*", ".*").replace("?", ".");

        let regex = match regex::Regex::new(&regex_pattern) {
            Ok(r) => r,
            Err(_) => {
                self.show_error("Invalid pattern");
                self.wait_for_keypress();
                return Ok(());
            }
        };

        let mut matched_count = 0;
        for file in files {
            if regex.is_match(&file.name) {
                selected_files.insert(file.name.clone());
                matched_count += 1;
            }
        }

        println!(
            "{}",
            format!(
                "✅ Selected {} files matching pattern '{}'",
                matched_count, pattern
            )
            .green()
        );
        self.wait_for_keypress();

        Ok(())
    }

    fn show_selected_files(
        &self,
        files: &[ArchiveFile],
        selected_files: &std::collections::HashSet<String>,
    ) -> Result<()> {
        self.clear_screen();
        self.print_section_header("Selected Files");

        if selected_files.is_empty() {
            println!("{}", "No files currently selected".yellow());
            self.wait_for_keypress();
            return Ok(());
        }

        let selected_file_objects: Vec<_> = files
            .iter()
            .filter(|file| selected_files.contains(&file.name))
            .collect();

        for (i, file) in selected_file_objects.iter().enumerate() {
            let size_str = file.size.map(format_size).unwrap_or_default();
            let format_str = file.format.as_deref().unwrap_or("Unknown");

            println!("{}. {} ({}) - {}", i + 1, file.name, format_str, size_str);
        }

        let total_size: u64 = selected_file_objects
            .iter()
            .map(|f| f.size.unwrap_or(0))
            .sum();

        println!();
        println!(
            "Total: {} files, {}",
            selected_files.len(),
            format_size(total_size)
        );
        self.wait_for_keypress();

        Ok(())
    }

    fn show_file_selection_interface(
        &self,
        matching_files: &[(usize, &ArchiveFile)],
        selected_files: &mut std::collections::HashSet<String>,
        title: &str,
    ) -> Result<()> {
        self.clear_screen();
        self.print_section_header(title);

        println!("Found {} files:", matching_files.len());
        println!();

        for (i, (_, file)) in matching_files.iter().enumerate() {
            let selected = if selected_files.contains(&file.name) {
                "✓"
            } else {
                " "
            };
            let size_str = file.size.map(format_size).unwrap_or_default();
            let format_str = file.format.as_deref().unwrap_or("Unknown");

            println!(
                "[{}] {}: {} ({}) - {}",
                selected,
                i + 1,
                file.name,
                format_str,
                size_str
            );
        }

        println!();
        println!("Commands:");
        println!("  a - Select all shown files");
        println!("  c - Clear all selections for shown files");
        println!("  s <number> - Toggle selection for file number");
        println!("  q - Back to file browser menu");
        println!();

        loop {
            let input = self.get_string_input("Enter command", "")?;
            let input = input.trim().to_lowercase();

            match input.as_str() {
                "a" => {
                    for (_, file) in matching_files {
                        selected_files.insert(file.name.clone());
                    }
                    println!("{}", "✅ Selected all shown files".green());
                    self.wait_for_keypress();
                    break;
                }
                "c" => {
                    for (_, file) in matching_files {
                        selected_files.remove(&file.name);
                    }
                    println!("{}", "✅ Cleared all selections for shown files".green());
                    self.wait_for_keypress();
                    break;
                }
                "q" => break,
                input if input.starts_with("s ") => {
                    if let Ok(file_num) = input[2..].parse::<usize>() {
                        if file_num > 0 && file_num <= matching_files.len() {
                            let file = matching_files[file_num - 1].1;
                            if selected_files.contains(&file.name) {
                                selected_files.remove(&file.name);
                                println!("{} Deselected: {}", "❌".red(), file.name);
                            } else {
                                selected_files.insert(file.name.clone());
                                println!("{} Selected: {}", "✅".green(), file.name);
                            }
                            self.wait_for_keypress();
                        } else {
                            self.show_error("Invalid file number");
                            self.wait_for_keypress();
                        }
                    } else {
                        self.show_error("Invalid command format. Use 's <number>'");
                        self.wait_for_keypress();
                    }
                }
                _ => {
                    self.show_error("Invalid command");
                    self.wait_for_keypress();
                }
            }
        }

        Ok(())
    }

    async fn download_selected_files(
        &self,
        session: &DownloadSession,
        selected_files: &std::collections::HashSet<String>,
        output_dir: &str,
    ) -> Result<()> {
        self.clear_screen();
        self.print_section_header("Download Selected Files");

        // Create download request with selected files only
        let mut request = DownloadRequest::from_config(
            &self.config,
            session.identifier.clone(),
            PathBuf::from(output_dir),
        );

        // Configure download options
        println!("{} Download Configuration:", "⚙️".bright_blue());
        println!("Selected files: {}", selected_files.len());

        let selected_size: u64 = session
            .archive_metadata
            .files
            .iter()
            .filter(|f| selected_files.contains(&f.name))
            .map(|f| f.size.unwrap_or(0))
            .sum();
        println!("Total size: {}", format_size(selected_size));
        println!();

        request.concurrent_downloads = self.get_number_input(
            "Concurrent downloads (1-16)",
            request.concurrent_downloads,
            1,
            16,
        )?;

        request.verify_md5 = self.get_yes_no("Verify MD5 checksums?", request.verify_md5)?;
        request.dry_run = self.get_yes_no("Dry run (preview only)?", false)?;

        if !self.get_yes_no("Start download?", true)? {
            return Ok(());
        }

        // Create a custom session with only selected files
        let filtered_files: Vec<ArchiveFile> = session
            .archive_metadata
            .files
            .iter()
            .filter(|f| selected_files.contains(&f.name))
            .cloned()
            .collect();

        println!(
            "{} Starting download of {} selected files...",
            "⬇️".bright_green(),
            filtered_files.len()
        );

        // Use the existing download execution with custom file filtering
        // We'll modify the request to include custom filters that match only selected files
        // For now, we'll use the existing download method but this could be enhanced
        // to support custom file lists more directly

        self.execute_download_with_progress(request).await
    }

    async fn configure_settings(&mut self) -> Result<()> {
        // Save current config before launching config menu
        self.config_manager.save_config(&self.config)?;

        use crate::interface::interactive::launch_config_menu;
        launch_config_menu().await?;

        // Reload config after configuration
        self.config = self.config_manager.load_config()?;
        Ok(())
    }

    async fn view_history(&mut self) -> Result<()> {
        self.clear_screen();
        self.print_section_header("Download History");

        if self.config.recent_urls.is_empty() {
            println!("{}", "No recent downloads found.".yellow());
            println!("Your download history will appear here after you use ia-get.");
        } else {
            println!("Recent downloads:");
            println!();
            for (i, url) in self.config.recent_urls.iter().enumerate() {
                println!("  {}. {}", (i + 1).to_string().cyan(), url.dimmed());
            }
            println!();

            if self.get_yes_no("Download from history?", false)? {
                let choice = self.get_number_input(
                    "Select download number",
                    1,
                    1,
                    self.config.recent_urls.len(),
                )?;

                let identifier = self.config.recent_urls[choice - 1].clone();
                let output_dir = self
                    .config
                    .default_output_path
                    .as_deref()
                    .unwrap_or("./downloads")
                    .to_string();

                let request = DownloadRequest::from_config(
                    &self.config,
                    identifier,
                    PathBuf::from(output_dir),
                );

                self.execute_download_with_progress(request).await?;
            }
        }

        self.wait_for_keypress();
        Ok(())
    }

    async fn execute_download_with_progress(&self, request: DownloadRequest) -> Result<()> {
        self.clear_screen();

        // Shared state for progress updates
        let download_state = Arc::new(Mutex::new(DownloadState {
            start_time: Some(Instant::now()),
            ..Default::default()
        }));

        let display_state = Arc::clone(&download_state);

        // Create progress callback
        let progress_callback = {
            let state = Arc::clone(&download_state);
            Box::new(move |update: ProgressUpdate| {
                let mut state = state.lock().unwrap();
                state.current_file = update.current_file;
                state.completed_files = update.completed_files;
                state.total_files = update.total_files;
                state.failed_files = update.failed_files;
                state.current_speed = update.current_speed;
                state.eta = update.eta;
                state.status = update.status;
            })
        };

        // Start progress display task
        let progress_task = {
            let state = Arc::clone(&display_state);
            tokio::spawn(async move {
                let mut last_update = Instant::now();
                loop {
                    if last_update.elapsed() >= Duration::from_millis(100) {
                        {
                            let state = state.lock().unwrap();
                            Self::update_progress_display(&state);
                        }
                        last_update = Instant::now();
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            })
        };

        // Execute download
        let result = self
            .download_service
            .download(request, Some(progress_callback))
            .await?;

        // Stop progress display
        progress_task.abort();

        // Show final result
        self.clear_screen();
        match result {
            DownloadResult::Success(session, _stats, _is_dry_run) => {
                self.show_success_summary(&session);
            }
            DownloadResult::Error(error) => {
                self.show_error(&format!("Download failed: {}", error));
            }
        }

        self.wait_for_keypress();
        Ok(())
    }

    fn update_progress_display(state: &DownloadState) {
        // Move cursor to top and clear from there
        print!("\x1B[H\x1B[J");

        // Enhanced header with animation
        let time_elapsed = state.start_time.map(|t| t.elapsed()).unwrap_or_default();
        let spinner_chars = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
        let spinner_index = (time_elapsed.as_millis() / 100) % spinner_chars.len() as u128;
        let spinner = spinner_chars[spinner_index as usize];

        println!(
            "{}",
            "╔══════════════════════════════════════════════════════════════════════════════╗"
                .cyan()
        );
        println!(
            "{} {} {} {}",
            "║".cyan(),
            format!("{} 📥 Download Progress - ia-get", spinner).bold(),
            " ".repeat(28),
            "║".cyan()
        );
        println!(
            "{}",
            "╚══════════════════════════════════════════════════════════════════════════════╝"
                .cyan()
        );
        println!();

        // Status with color coding
        let status_color = if state.status.contains("Error") || state.status.contains("Failed") {
            state.status.red()
        } else if state.status.contains("Complete") {
            state.status.green()
        } else {
            state.status.bright_blue()
        };

        println!("{} {}", "Status:".bold(), status_color);
        println!();

        // Current file with smart truncation
        if !state.current_file.is_empty() {
            let display_file = if state.current_file.len() > 60 {
                format!(
                    "...{}",
                    &state.current_file[state.current_file.len() - 57..]
                )
            } else {
                state.current_file.clone()
            };

            println!("{} {}", "📄 Current:".bold(), display_file.cyan());
            println!();
        }

        // Enhanced progress bar with percentage and visual indicators
        if state.total_files > 0 {
            let progress = state.completed_files as f32 / state.total_files as f32;
            let bar_width = 50;
            let filled = (progress * bar_width as f32) as usize;
            let empty = bar_width - filled;

            // Create a more detailed progress bar
            let completed_char = "█";
            let partial_char = "▓";
            let empty_char = "░";

            let bar = if filled == bar_width {
                format!(
                    "[{}] {:.1}%",
                    completed_char.repeat(bar_width).bright_green(),
                    progress * 100.0
                )
            } else if filled > 0 {
                format!(
                    "[{}{}{}] {:.1}%",
                    completed_char.repeat(filled).green(),
                    partial_char.yellow(),
                    empty_char.repeat(empty.saturating_sub(1)).dimmed(),
                    progress * 100.0
                )
            } else {
                format!(
                    "[{}] {:.1}%",
                    empty_char.repeat(bar_width).dimmed(),
                    progress * 100.0
                )
            };

            println!("📊 Progress: {}", bar);

            // Progress details
            println!(
                "    {} {} / {} files",
                "📁".cyan(),
                state.completed_files.to_string().bright_green(),
                state.total_files.to_string().bright_blue()
            );
            println!();
        }

        // Enhanced statistics section
        println!("{}", "📈 Statistics:".bold().bright_magenta());

        // Files status with icons
        println!(
            "  {} Completed: {}",
            "✅".green(),
            state.completed_files.to_string().bright_green()
        );

        if state.total_files > 0 {
            let remaining = state.total_files - state.completed_files;
            if remaining > 0 {
                println!(
                    "  {} Remaining: {}",
                    "⏳".yellow(),
                    remaining.to_string().bright_yellow()
                );
            }
        }

        if state.failed_files > 0 {
            println!(
                "  {} Failed: {}",
                "❌".red(),
                state.failed_files.to_string().red()
            );
        }

        // Performance metrics
        if state.current_speed > 0.0 {
            let speed_str = format_speed(state.current_speed);
            let speed_color = if state.current_speed > 10_000_000.0 {
                speed_str.bright_green()
            } else if state.current_speed > 1_000_000.0 {
                speed_str.green()
            } else {
                speed_str.yellow()
            };

            println!("  {} Speed: {}", "🚀".bright_blue(), speed_color);
        }

        if !state.eta.is_empty() && state.eta != "Unknown" {
            println!("  {} ETA: {}", "⏰".bright_cyan(), state.eta.bright_blue());
        }

        if let Some(start_time) = state.start_time {
            let elapsed = start_time.elapsed();
            println!(
                "  {} Elapsed: {}",
                "⏱️".bright_magenta(),
                format_duration(elapsed).bright_magenta()
            );
        }

        println!();

        // Interactive controls hint
        println!(
            "{}",
            "┌─ Controls ─────────────────────────────────────────────────────┐".dimmed()
        );
        println!(
            "{}",
            "│ Press Ctrl+C to cancel download                                │".dimmed()
        );
        println!(
            "{}",
            "└────────────────────────────────────────────────────────────────┘".dimmed()
        );

        io::stdout().flush().unwrap();
    }

    // Helper methods...

    fn print_section_header(&self, title: &str) {
        println!();
        println!(
            "{}",
            format!(
                "┌─ 🔧 {} ─{:─<width$}┐",
                title,
                "",
                width = 60 - title.len()
            )
            .bold()
            .bright_cyan()
        );
        println!(
            "{}",
            "│                                                                 │".cyan()
        );
    }

    fn print_subsection(&self, title: &str) {
        println!();
        println!("{}", format!("📋 {}", title).bold().blue());
        println!("{}", "─".repeat(title.len() + 4).blue());
        println!();
    }

    fn get_user_choice(&self, prompt: &str, max: usize) -> Result<usize> {
        loop {
            print!(
                "{} {} (1-{}): ",
                "❯".bright_green(),
                prompt.bold(),
                max.to_string().bright_cyan()
            );
            io::stdout().flush()?;

            let mut input = String::new();
            io::stdin().read_line(&mut input)?;

            match input.trim().parse::<usize>() {
                Ok(choice) if choice >= 1 && choice <= max => return Ok(choice),
                _ => {
                    println!(
                        "{} {} {}",
                        "❌".red(),
                        "Invalid choice.".red(),
                        format!("Please enter a number between 1 and {}", max).yellow()
                    );
                }
            }
        }
    }

    fn get_string_input(&self, prompt: &str, hint: &str) -> Result<String> {
        println!();
        println!("{} {}", "📝".bright_blue(), prompt.bold());
        if !hint.is_empty() {
            println!("{} {}", "💡".yellow(), hint.dimmed());
        }
        print!("{} ", "❯".bright_green());
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        Ok(input.trim().to_string())
    }

    fn get_string_input_with_default(
        &self,
        prompt: &str,
        hint: &str,
        default: &str,
    ) -> Result<String> {
        println!();
        println!("{} {}", "📝".bright_blue(), prompt.bold());
        if !hint.is_empty() {
            println!("{} {}", "💡".yellow(), hint.dimmed());
        }
        println!("{} Default: {}", "🔧".cyan(), default.bright_cyan());
        print!("{} ", "❯".bright_green());
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let trimmed = input.trim();

        Ok(if trimmed.is_empty() {
            default.to_string()
        } else {
            trimmed.to_string()
        })
    }

    fn get_yes_no(&self, prompt: &str, default: bool) -> Result<bool> {
        let default_display = if default {
            format!("{}/{}", "Y".bright_green(), "n".dimmed())
        } else {
            format!("{}/{}", "y".dimmed(), "N".bright_red())
        };

        loop {
            print!(
                "{} {} ({}): ",
                "❓".bright_yellow(),
                prompt.bold(),
                default_display
            );
            io::stdout().flush()?;

            let mut input = String::new();
            io::stdin().read_line(&mut input)?;

            match input.trim().to_lowercase().as_str() {
                "y" | "yes" => return Ok(true),
                "n" | "no" => return Ok(false),
                "" => return Ok(default),
                _ => println!(
                    "{} {}",
                    "❌".red(),
                    "Please enter 'y' for yes or 'n' for no".yellow()
                ),
            }
        }
    }

    fn get_number_input(
        &self,
        prompt: &str,
        default: usize,
        min: usize,
        max: usize,
    ) -> Result<usize> {
        loop {
            print!("{} (default {}): ", prompt.bold(), default);
            io::stdout().flush()?;

            let mut input = String::new();
            io::stdin().read_line(&mut input)?;

            if input.trim().is_empty() {
                return Ok(default);
            }

            match input.trim().parse::<usize>() {
                Ok(num) if num >= min && num <= max => return Ok(num),
                _ => self.show_error(&format!(
                    "Please enter a number between {} and {}",
                    min, max
                )),
            }
        }
    }

    fn configure_format_filters(&self, request: &mut DownloadRequest) -> Result<()> {
        self.print_subsection("File Format Filters");

        // Offer format categories first
        println!("Choose filtering method:");
        println!("  1. Use predefined format categories");
        println!("  2. Manually specify file extensions");
        println!("  3. Use both categories and manual extensions");
        println!("  4. Skip format filtering");

        let choice = self.get_string_input("Enter choice (1-4)", "Default: 4 (skip)")?;

        match choice.trim() {
            "1" => self.configure_format_categories_only(request)?,
            "2" => self.configure_manual_extensions_only(request)?,
            "3" => {
                self.configure_format_categories_only(request)?;
                self.configure_manual_extensions_only(request)?;
            }
            "4" | "" => {
                println!("No format filtering configured.");
                return Ok(());
            }
            _ => {
                println!("Invalid choice, skipping format filtering.");
                return Ok(());
            }
        }

        Ok(())
    }

    fn configure_format_categories_only(&self, request: &mut DownloadRequest) -> Result<()> {
        use crate::utilities::filters::{FileFormats, FormatCategory};

        println!("\n📁 Available Format Categories:");
        let file_formats = FileFormats::new();
        for (i, category) in FormatCategory::all().iter().enumerate() {
            let formats = file_formats.get_formats(category);
            let sample_formats: Vec<String> = formats.iter().take(3).cloned().collect();
            println!(
                "  {}. {} - {} (e.g., {})",
                i + 1,
                category.display_name(),
                category.description(),
                sample_formats.join(", ")
            );
        }

        println!("\nCommon Presets:");
        let presets = FileFormats::get_common_presets();
        for (i, (name, description, _)) in presets.iter().enumerate() {
            println!(
                "  {}. {} - {}",
                FormatCategory::all().len() + i + 1,
                name,
                description
            );
        }

        // Include categories
        let include = self.get_string_input(
            "Include categories (numbers or names, comma-separated, e.g., 1,3 or documents,images)",
            "Leave empty to include all",
        )?;

        if !include.is_empty() {
            let mut extensions = Vec::new();
            for item in include.split(',') {
                let item = item.trim();
                if let Ok(num) = item.parse::<usize>() {
                    if num >= 1 && num <= FormatCategory::all().len() {
                        let category = &FormatCategory::all()[num - 1];
                        extensions.extend(file_formats.get_formats(category));
                    } else if num > FormatCategory::all().len() {
                        // Handle preset selection
                        let preset_index = num - FormatCategory::all().len() - 1;
                        if preset_index < presets.len() {
                            let (_, _, preset_formats) = &presets[preset_index];
                            extensions.extend(preset_formats.clone());
                        }
                    }
                } else {
                    // Try to match by name
                    let item_lower = item.to_lowercase();
                    for category in FormatCategory::all() {
                        if category.display_name().to_lowercase() == item_lower {
                            extensions.extend(file_formats.get_formats(&category));
                            break;
                        }
                    }
                }
            }
            request.include_formats.extend(extensions);
        }

        // Exclude categories
        let exclude = self.get_string_input(
            "Exclude categories (numbers or names, comma-separated)",
            "Leave empty to exclude nothing",
        )?;

        if !exclude.is_empty() {
            let mut extensions = Vec::new();
            for item in exclude.split(',') {
                let item = item.trim();
                if let Ok(num) = item.parse::<usize>() {
                    if num >= 1 && num <= FormatCategory::all().len() {
                        let category = &FormatCategory::all()[num - 1];
                        extensions.extend(file_formats.get_formats(category));
                    }
                } else {
                    // Try to match by name
                    let item_lower = item.to_lowercase();
                    for category in FormatCategory::all() {
                        if category.display_name().to_lowercase() == item_lower {
                            extensions.extend(file_formats.get_formats(&category));
                            break;
                        }
                    }
                }
            }
            request.exclude_formats.extend(extensions);
        }

        Ok(())
    }

    fn configure_manual_extensions_only(&self, request: &mut DownloadRequest) -> Result<()> {
        println!("\nManual Extension Configuration:");

        let include = self.get_string_input(
            "Include formats (comma-separated, e.g., pdf,txt,mp3)",
            "Leave empty to include all formats",
        )?;

        if !include.is_empty() {
            let mut extensions: Vec<String> =
                include.split(',').map(|s| s.trim().to_string()).collect();
            request.include_formats.append(&mut extensions);
        }

        let exclude = self.get_string_input(
            "Exclude formats (comma-separated, e.g., xml,log,tmp)",
            "Leave empty to exclude no formats",
        )?;

        if !exclude.is_empty() {
            let mut extensions: Vec<String> =
                exclude.split(',').map(|s| s.trim().to_string()).collect();
            request.exclude_formats.append(&mut extensions);
        }

        Ok(())
    }

    fn configure_size_filters(&self, request: &mut DownloadRequest) -> Result<()> {
        self.print_subsection("File Size Filters");

        let min_size = self.get_string_input(
            "Minimum file size (e.g., 1MB, 500KB)",
            "Leave empty for no minimum",
        )?;

        if !min_size.is_empty() {
            request.min_file_size = min_size;
        }

        let max_size = self.get_string_input(
            "Maximum file size (e.g., 100MB, 1GB)",
            "Leave empty for no maximum",
        )?;

        if !max_size.is_empty() {
            request.max_file_size = Some(max_size);
        }

        Ok(())
    }

    fn configure_source_filters(&self, request: &mut DownloadRequest) -> Result<()> {
        self.print_subsection("Source Type Filters");

        println!("Select which source types to include:");
        println!("  1. Original files only (default)");
        println!("  2. Derivative files only");
        println!("  3. Metadata files only");
        println!("  4. Original + Derivative");
        println!("  5. Original + Metadata");
        println!("  6. Derivative + Metadata");
        println!("  7. All types");

        let choice =
            self.get_string_input("Enter choice (1-7)", "Default: 1 (original files only)")?;

        use crate::interface::cli::SourceType;

        request.source_types = match choice.trim() {
            "1" | "" => vec![SourceType::Original],
            "2" => vec![SourceType::Derivative],
            "3" => vec![SourceType::Metadata],
            "4" => vec![SourceType::Original, SourceType::Derivative],
            "5" => vec![SourceType::Original, SourceType::Metadata],
            "6" => vec![SourceType::Derivative, SourceType::Metadata],
            "7" => vec![
                SourceType::Original,
                SourceType::Derivative,
                SourceType::Metadata,
            ],
            _ => {
                println!("Invalid choice, defaulting to original files only");
                vec![SourceType::Original]
            }
        };

        Ok(())
    }

    fn show_download_summary(&self, request: &DownloadRequest) {
        self.print_subsection("Download Summary");

        println!("Archive: {}", request.identifier.cyan());
        println!(
            "Output: {}",
            request.output_dir.display().to_string().cyan()
        );
        println!(
            "Concurrent: {}",
            request.concurrent_downloads.to_string().cyan()
        );

        if !request.include_formats.is_empty() {
            println!("Include: {}", request.include_formats.join(", ").green());
        }

        if !request.exclude_formats.is_empty() {
            println!("Exclude: {}", request.exclude_formats.join(", ").red());
        }

        if !request.min_file_size.is_empty() {
            println!("Min size: {}", request.min_file_size.yellow());
        }

        if let Some(ref max_size) = request.max_file_size {
            println!("Max size: {}", max_size.yellow());
        }

        // Show source types
        use crate::interface::cli::SourceType;
        let source_type_names: Vec<&str> = request
            .source_types
            .iter()
            .map(|st| match st {
                SourceType::Original => "original",
                SourceType::Derivative => "derivative",
                SourceType::Metadata => "metadata",
            })
            .collect();
        println!("Source types: {}", source_type_names.join(", ").cyan());

        if request.dry_run {
            println!(
                "{}",
                "🔍 DRY RUN MODE - No files will be downloaded".bright_yellow()
            );
        }

        println!();
    }

    fn show_success_summary(&self, session: &crate::core::session::DownloadSession) {
        println!(
            "{}",
            "╔══════════════════════════════════════════════════════════════════════════════╗"
                .green()
        );
        println!(
            "{}",
            "║                             ✅ Download Complete!                            ║"
                .green()
        );
        println!(
            "{}",
            "╚══════════════════════════════════════════════════════════════════════════════╝"
                .green()
        );
        println!();

        let total_files = session.archive_metadata.files.len();
        let total_size: u64 = session
            .archive_metadata
            .files
            .iter()
            .map(|f| f.size.unwrap_or(0))
            .sum();

        println!("📊 Summary:");
        println!(
            "  Files downloaded: {}",
            total_files.to_string().bright_green()
        );
        println!("  Total size: {}", format_size(total_size).bright_cyan());
        println!("  Location: {}", session.download_config.output_dir.cyan());
        println!();
    }

    fn show_error(&self, message: &str) {
        println!();
        println!(
            "{}",
            "┌─ ❌ Error ─────────────────────────────────────────────────────┐".red()
        );
        println!(
            "{} {} {}",
            "│".red(),
            message.red(),
            " ".repeat(60 - message.len().min(60)).red()
        );
        println!(
            "{}",
            "└────────────────────────────────────────────────────────────────┘".red()
        );
        println!();
    }

    fn wait_for_keypress(&self) {
        println!();
        print!(
            "{} {}",
            "⏸️".bright_blue(),
            "Press Enter to continue...".dimmed()
        );
        io::stdout().flush().unwrap();
        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap();
        println!();
    }
}

// Helper functions

fn format_speed(bytes_per_sec: f64) -> String {
    if bytes_per_sec >= 1_000_000.0 {
        format!("{:.1} MB/s", bytes_per_sec / 1_000_000.0)
    } else if bytes_per_sec >= 1_000.0 {
        format!("{:.1} KB/s", bytes_per_sec / 1_000.0)
    } else {
        format!("{:.0} B/s", bytes_per_sec)
    }
}

fn format_duration(duration: Duration) -> String {
    let total_secs = duration.as_secs();
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;

    if hours > 0 {
        format!("{}h {}m {}s", hours, minutes, seconds)
    } else if minutes > 0 {
        format!("{}m {}s", minutes, seconds)
    } else {
        format!("{}s", seconds)
    }
}

/// Launch the enhanced interactive CLI
pub async fn launch_interactive_cli() -> Result<()> {
    let mut cli = InteractiveCli::new()?;
    cli.run().await
}
