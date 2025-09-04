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

            #[cfg(feature = "gui")]
            let max_choice = if crate::can_use_gui() { 7 } else { 6 };
            #[cfg(not(feature = "gui"))]
            let max_choice = 6;

            match self.get_user_choice("Select an option", max_choice)? {
                1 => self.download_archive().await?,
                2 => self.quick_download().await?,
                3 => self.browse_and_download().await?,
                4 => self.configure_settings().await?,
                5 => self.view_history().await?,
                6 => {
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
                7 => {
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

        // Only show GUI option if GUI features are compiled and available
        #[cfg(feature = "gui")]
        {
            if crate::can_use_gui() {
                println!(
                    "│  {} {} {}{}│",
                    "6.".bright_green().bold(),
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
                    "7.".bright_green().bold(),
                    "🚪".cyan(),
                    "Exit                                                ".normal(),
                    " ".blue()
                );
            } else {
                println!(
                    "│  {} {} {}{}│",
                    "6.".bright_green().bold(),
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
                "6.".bright_green().bold(),
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

        use crate::cli::SourceType;

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
        use crate::cli::SourceType;
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
