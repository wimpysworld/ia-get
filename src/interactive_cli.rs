//! Enhanced interactive CLI interface for ia-get
//!
//! Provides a comprehensive command-line interface that mirrors GUI functionality
//! with live updating progress, non-scrolling interface, and unified API usage.

use crate::{
    config::{Config, ConfigManager},
    download_service::{DownloadRequest, DownloadResult, DownloadService, ProgressUpdate},
    filters::format_size,
    Result,
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

            match self.get_user_choice("Select an option", 6)? {
                1 => self.download_archive().await?,
                2 => self.quick_download().await?,
                3 => self.browse_and_download().await?,
                4 => self.configure_settings().await?,
                5 => self.view_history().await?,
                6 => {
                    println!(
                        "{}",
                        "\n✨ Thanks for using ia-get! Goodbye! 👋".bright_cyan()
                    );
                    break;
                }
                _ => {
                    self.show_error("Invalid choice. Please try again.");
                    self.wait_for_keypress();
                }
            }
        }

        Ok(())
    }

    fn clear_screen(&self) {
        // Clear screen and move cursor to top-left
        print!("\x1B[2J\x1B[H");
        io::stdout().flush().unwrap();
    }

    fn print_header(&self) {
        println!(
            "{}",
            "╔══════════════════════════════════════════════════════════════════════════════╗"
                .cyan()
        );
        println!(
            "{}",
            "║                    🚀 ia-get - Internet Archive Downloader                   ║"
                .cyan()
        );
        println!(
            "{}",
            "╚══════════════════════════════════════════════════════════════════════════════╝"
                .cyan()
        );
        println!();
    }

    fn print_main_menu(&self) {
        println!("{}", "📋 MAIN MENU".bold().bright_blue());
        println!("{}", "━".repeat(50).blue());
        println!();
        println!(
            "  {} {} Full Archive Download",
            "1.".bright_green(),
            "📦".cyan()
        );
        println!("     Download complete archives with filtering options");
        println!();
        println!(
            "  {} {} Quick URL Download",
            "2.".bright_green(),
            "⚡".cyan()
        );
        println!("     Fast download from Archive URL or identifier");
        println!();
        println!("  {} {} Browse & Select", "3.".bright_green(), "🔍".cyan());
        println!("     Browse archive contents and select files");
        println!();
        println!(
            "  {} {} Settings & Configuration",
            "4.".bright_green(),
            "⚙️".cyan()
        );
        println!("     Configure download preferences and filters");
        println!();
        println!("  {} {} Download History", "5.".bright_green(), "📚".cyan());
        println!("     View and manage download history");
        println!();
        println!("  {} {} Exit", "6.".bright_green(), "🚪".cyan());
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

    async fn browse_and_download(&mut self) -> Result<()> {
        self.clear_screen();
        self.print_section_header("Browse & Select Files");

        // This would be implemented to show archive contents and let users select
        // For now, show a placeholder
        println!("{}", "🚧 Browse & Select feature coming soon!".yellow());
        println!("This will allow you to:");
        println!("  • View archive file listings");
        println!("  • Select specific files to download");
        println!("  • Preview file information");
        println!();
        self.wait_for_keypress();
        Ok(())
    }

    async fn configure_settings(&mut self) -> Result<()> {
        // Save current config before launching config menu
        self.config_manager.save_config(&self.config)?;

        use crate::interactive_menu::launch_config_menu;
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
            DownloadResult::Success(session, _stats) => {
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

        println!(
            "{}",
            "╔══════════════════════════════════════════════════════════════════════════════╗"
                .cyan()
        );
        println!(
            "{}",
            "║                           📥 Download Progress                               ║"
                .cyan()
        );
        println!(
            "{}",
            "╚══════════════════════════════════════════════════════════════════════════════╝"
                .cyan()
        );
        println!();

        // Status
        println!("{} {}", "Status:".bold(), state.status.bright_blue());
        println!();

        // Current file
        if !state.current_file.is_empty() {
            println!(
                "{} {}",
                "Current:".bold(),
                state.current_file.truncate_with_ellipsis(60).cyan()
            );
            println!();
        }

        // Progress bar
        if state.total_files > 0 {
            let progress = state.completed_files as f32 / state.total_files as f32;
            let bar_width = 60;
            let filled = (progress * bar_width as f32) as usize;
            let empty = bar_width - filled;

            let bar = format!(
                "[{}{}] {:.1}%",
                "█".repeat(filled).green(),
                "░".repeat(empty).dimmed(),
                progress * 100.0
            );

            println!("Progress: {}", bar);
            println!();
        }

        // Statistics
        println!("{}", "Statistics:".bold());
        println!(
            "  Files: {} / {} completed",
            state.completed_files.to_string().green(),
            state.total_files.to_string().cyan()
        );

        if state.failed_files > 0 {
            println!("  Failed: {}", state.failed_files.to_string().red());
        }

        if state.current_speed > 0.0 {
            println!(
                "  Speed: {}",
                format_speed(state.current_speed).bright_yellow()
            );
        }

        if !state.eta.is_empty() {
            println!("  ETA: {}", state.eta.bright_blue());
        }

        if let Some(start_time) = state.start_time {
            let elapsed = start_time.elapsed();
            println!("  Elapsed: {}", format_duration(elapsed).bright_magenta());
        }

        println!();
        println!("{}", "Press Ctrl+C to cancel".dimmed());

        io::stdout().flush().unwrap();
    }

    // Helper methods...

    fn print_section_header(&self, title: &str) {
        println!("{}", format!("🔧 {}", title).bold().bright_cyan());
        println!("{}", "━".repeat(60).cyan());
        println!();
    }

    fn print_subsection(&self, title: &str) {
        println!();
        println!("{}", format!("📋 {}", title).bold().blue());
        println!("{}", "─".repeat(40).blue());
    }

    fn get_user_choice(&self, prompt: &str, max: usize) -> Result<usize> {
        loop {
            print!("{} (1-{}): ", prompt.bold(), max);
            io::stdout().flush()?;

            let mut input = String::new();
            io::stdin().read_line(&mut input)?;

            match input.trim().parse::<usize>() {
                Ok(choice) if choice >= 1 && choice <= max => return Ok(choice),
                _ => {
                    self.show_error(&format!("Please enter a number between 1 and {}", max));
                }
            }
        }
    }

    fn get_string_input(&self, prompt: &str, hint: &str) -> Result<String> {
        println!("{}", prompt.bold());
        if !hint.is_empty() {
            println!("{}", format!("  {}", hint).dimmed());
        }
        print!("> ");
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
        println!("{}", prompt.bold());
        if !hint.is_empty() {
            println!("{}", format!("  {}", hint).dimmed());
        }
        print!("> ");
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
        let default_str = if default { "Y/n" } else { "y/N" };
        loop {
            print!("{} ({}): ", prompt.bold(), default_str);
            io::stdout().flush()?;

            let mut input = String::new();
            io::stdin().read_line(&mut input)?;

            match input.trim().to_lowercase().as_str() {
                "y" | "yes" => return Ok(true),
                "n" | "no" => return Ok(false),
                "" => return Ok(default),
                _ => self.show_error("Please enter 'y' for yes or 'n' for no"),
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

        let include = self.get_string_input(
            "Include formats (comma-separated, e.g., pdf,txt,mp3)",
            "Leave empty to include all formats",
        )?;

        if !include.is_empty() {
            request.include_formats = include.split(',').map(|s| s.trim().to_string()).collect();
        }

        let exclude = self.get_string_input(
            "Exclude formats (comma-separated, e.g., xml,log,tmp)",
            "Leave empty to exclude no formats",
        )?;

        if !exclude.is_empty() {
            request.exclude_formats = exclude.split(',').map(|s| s.trim().to_string()).collect();
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

        if request.dry_run {
            println!(
                "{}",
                "🔍 DRY RUN MODE - No files will be downloaded".bright_yellow()
            );
        }

        println!();
    }

    fn show_success_summary(&self, session: &crate::metadata_storage::DownloadSession) {
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
        println!("{} {}", "❌".red(), message.red());
    }

    fn wait_for_keypress(&self) {
        print!("\n{}", "Press Enter to continue...".dimmed());
        io::stdout().flush().unwrap();
        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap();
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

// Extension trait for string truncation
trait StringExt {
    fn truncate_with_ellipsis(&self, max_len: usize) -> String;
}

impl StringExt for str {
    fn truncate_with_ellipsis(&self, max_len: usize) -> String {
        if self.len() <= max_len {
            self.to_string()
        } else {
            format!("{}...", &self[..max_len.saturating_sub(3)])
        }
    }
}

/// Launch the enhanced interactive CLI
pub async fn launch_interactive_cli() -> Result<()> {
    let mut cli = InteractiveCli::new()?;
    cli.run().await
}
