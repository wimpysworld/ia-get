//! Interactive CLI menu system for ia-get configuration
//!
//! Provides a user-friendly terminal interface for configuring the application
//! with navigation, editing, and preview capabilities.

use crate::{
    Result,
    infrastructure::config::{Config, ConfigManager, FilterPreset},
    utilities::filters::parse_size_string,
};
use colored::*;
use std::io::{self, Write};

/// Interactive menu system for configuration management
pub struct InteractiveMenu {
    config_manager: ConfigManager,
    config: Config,
}

impl InteractiveMenu {
    /// Create a new interactive menu
    pub fn new() -> Result<Self> {
        let config_manager = ConfigManager::new()?;
        let config = config_manager.load_config()?;

        Ok(Self {
            config_manager,
            config,
        })
    }

    /// Run the interactive configuration menu
    pub async fn run(&mut self) -> Result<()> {
        self.print_welcome();

        loop {
            self.print_main_menu();

            match self.get_user_choice("Enter your choice")? {
                1 => self.configure_downloads().await?,
                2 => self.configure_filters().await?,
                3 => self.manage_presets().await?,
                4 => self.view_recent_urls().await?,
                5 => self.advanced_settings().await?,
                6 => self.backup_restore().await?,
                7 => self.preview_config().await?,
                8 => {
                    self.save_and_exit().await?;
                    break;
                }
                9 => {
                    println!("{}", "Configuration cancelled. No changes saved.".yellow());
                    break;
                }
                _ => println!("{}", "Invalid choice. Please try again.".red()),
            }
        }

        Ok(())
    }

    /// Print welcome message
    fn print_welcome(&self) {
        println!("\n{}", "=".repeat(60).cyan());
        println!("{}", "🚀 IA-GET CONFIGURATION MENU".cyan().bold());
        println!("{}", "=".repeat(60).cyan());
        println!("Welcome to the ia-get configuration interface!");
        println!("Configure download settings, filters, and preferences.\n");
    }

    /// Print main menu options
    fn print_main_menu(&self) {
        println!("{}", "📋 MAIN MENU".blue().bold());
        println!("{}", "-".repeat(30).blue());
        println!("1. {} Download Settings", "⬇️".cyan());
        println!("2. {} File Filters", "🔍".cyan());
        println!("3. {} Filter Presets", "📁".cyan());
        println!("4. {} Recent URLs", "🕒".cyan());
        println!("5. {} Advanced Settings", "⚙️".cyan());
        println!("6. {} Backup & Restore", "💾".cyan());
        println!("7. {} Preview Configuration", "👁️".cyan());
        println!("8. {} Save & Exit", "✅".green());
        println!("9. {} Cancel", "❌".red());
        println!();
    }

    /// Configure download settings
    async fn configure_downloads(&mut self) -> Result<()> {
        self.print_section_header("Download Settings");

        loop {
            println!("Current Settings:");
            println!(
                "  Output Path: {}",
                self.config
                    .default_output_path
                    .as_deref()
                    .unwrap_or("(current directory)")
                    .cyan()
            );
            println!(
                "  Concurrent Downloads: {}",
                self.config.concurrent_downloads.to_string().cyan()
            );
            println!(
                "  Max Retries: {}",
                self.config.max_retries.to_string().cyan()
            );
            println!(
                "  Resume Downloads: {}",
                if self.config.default_resume {
                    "Yes".green()
                } else {
                    "No".red()
                }
            );
            println!(
                "  Verbose Output: {}",
                if self.config.default_verbose {
                    "Yes".green()
                } else {
                    "No".red()
                }
            );
            println!(
                "  Log Hash Errors: {}",
                if self.config.default_log_hash_errors {
                    "Yes".green()
                } else {
                    "No".red()
                }
            );
            println!();

            println!("1. Change Output Path");
            println!("2. Set Concurrent Downloads (1-20)");
            println!("3. Set Max Retries (1-10)");
            println!("4. Toggle Resume Downloads");
            println!("5. Toggle Verbose Output");
            println!("6. Toggle Hash Error Logging");
            println!("7. Back to Main Menu");

            match self.get_user_choice("Select option")? {
                1 => self.set_output_path()?,
                2 => self.set_concurrent_downloads()?,
                3 => self.set_max_retries()?,
                4 => self.config.default_resume = !self.config.default_resume,
                5 => self.config.default_verbose = !self.config.default_verbose,
                6 => self.config.default_log_hash_errors = !self.config.default_log_hash_errors,
                7 => break,
                _ => println!("{}", "Invalid choice.".red()),
            }
            println!();
        }

        Ok(())
    }

    /// Configure file filters
    async fn configure_filters(&mut self) -> Result<()> {
        self.print_section_header("File Filters");

        loop {
            println!("Current Filter Settings:");
            println!(
                "  Include Extensions: {}",
                self.config
                    .default_include_ext
                    .as_deref()
                    .unwrap_or("(none)")
                    .cyan()
            );
            println!(
                "  Exclude Extensions: {}",
                self.config
                    .default_exclude_ext
                    .as_deref()
                    .unwrap_or("(none)")
                    .cyan()
            );
            println!(
                "  Max File Size: {}",
                self.config
                    .default_max_file_size
                    .as_deref()
                    .unwrap_or("(unlimited)")
                    .cyan()
            );
            println!();

            println!("1. Set Include Extensions (e.g., pdf,txt,mp3)");
            println!("2. Set Exclude Extensions (e.g., xml,log,tmp)");
            println!("3. Set Maximum File Size (e.g., 100MB, 1GB)");
            println!("4. Clear Include Extensions");
            println!("5. Clear Exclude Extensions");
            println!("6. Clear File Size Limit");
            println!("7. Back to Main Menu");

            match self.get_user_choice("Select option")? {
                1 => self.set_include_extensions()?,
                2 => self.set_exclude_extensions()?,
                3 => self.set_max_file_size()?,
                4 => self.config.default_include_ext = None,
                5 => self.config.default_exclude_ext = None,
                6 => self.config.default_max_file_size = None,
                7 => break,
                _ => println!("{}", "Invalid choice.".red()),
            }
            println!();
        }

        Ok(())
    }

    /// Manage filter presets
    async fn manage_presets(&mut self) -> Result<()> {
        self.print_section_header("Filter Presets");

        loop {
            println!("Available Presets:");
            for (i, preset) in self.config.filter_presets.iter().enumerate() {
                println!(
                    "  {}. {} - {}",
                    (i + 1).to_string().cyan(),
                    preset.name.bold(),
                    preset.description.dimmed()
                );
            }
            println!();

            println!("1. Apply Preset to Current Filters");
            println!("2. Create New Preset");
            println!("3. Edit Preset");
            println!("4. Delete Preset");
            println!("5. View Preset Details");
            println!("6. Back to Main Menu");

            match self.get_user_choice("Select option")? {
                1 => self.apply_preset()?,
                2 => self.create_preset()?,
                3 => self.edit_preset()?,
                4 => self.delete_preset()?,
                5 => self.view_preset_details()?,
                6 => break,
                _ => println!("{}", "Invalid choice.".red()),
            }
            println!();
        }

        Ok(())
    }

    /// View recent URLs
    async fn view_recent_urls(&mut self) -> Result<()> {
        self.print_section_header("Recent URLs");

        if self.config.recent_urls.is_empty() {
            println!("{}", "No recent URLs found.".yellow());
            println!(
                "URLs will appear here after you use ia-get to download from Internet Archive."
            );
        } else {
            println!("Recent Archive URLs:");
            for (i, url) in self.config.recent_urls.iter().enumerate() {
                println!("  {}. {}", (i + 1).to_string().cyan(), url.dimmed());
            }
            println!();

            println!("1. Clear Recent URLs");
            println!(
                "2. Set Max Recent URLs (current: {})",
                self.config.max_recent_urls
            );
            println!("3. Back to Main Menu");

            match self.get_user_choice("Select option")? {
                1 => {
                    self.config.recent_urls.clear();
                    println!("{}", "Recent URLs cleared.".green());
                }
                2 => self.set_max_recent_urls()?,
                3 => {}
                _ => println!("{}", "Invalid choice.".red()),
            }
        }

        self.pause();
        Ok(())
    }

    /// Advanced settings menu
    async fn advanced_settings(&mut self) -> Result<()> {
        self.print_section_header("Advanced Settings");

        loop {
            println!("Current Advanced Settings:");
            println!(
                "  HTTP Timeout: {} seconds",
                self.config.http_timeout.to_string().cyan()
            );
            println!(
                "  User Agent: {}",
                self.config
                    .user_agent_override
                    .as_deref()
                    .unwrap_or("(default)")
                    .cyan()
            );
            println!(
                "  Dry Run Mode: {}",
                if self.config.default_dry_run {
                    "Yes".green()
                } else {
                    "No".red()
                }
            );
            println!();

            println!("1. Set HTTP Timeout (1-300 seconds)");
            println!("2. Set Custom User Agent");
            println!("3. Clear Custom User Agent");
            println!("4. Toggle Default Dry Run Mode");
            println!("5. Reset All Settings to Defaults");
            println!("6. Back to Main Menu");

            match self.get_user_choice("Select option")? {
                1 => self.set_http_timeout()?,
                2 => self.set_user_agent()?,
                3 => self.config.user_agent_override = None,
                4 => self.config.default_dry_run = !self.config.default_dry_run,
                5 => self.reset_to_defaults()?,
                6 => break,
                _ => println!("{}", "Invalid choice.".red()),
            }
            println!();
        }

        Ok(())
    }

    /// Backup and restore functionality
    async fn backup_restore(&mut self) -> Result<()> {
        self.print_section_header("Backup & Restore");

        println!("1. Create Configuration Backup");
        println!("2. List Available Backups");
        println!("3. Restore from Backup");
        println!("4. Show Config File Location");
        println!("5. Back to Main Menu");

        match self.get_user_choice("Select option")? {
            1 => self.create_backup()?,
            2 => self.list_backups()?,
            3 => self.restore_backup()?,
            4 => self.show_config_location()?,
            5 => {}
            _ => println!("{}", "Invalid choice.".red()),
        }

        self.pause();
        Ok(())
    }

    /// Preview current configuration
    async fn preview_config(&mut self) -> Result<()> {
        self.print_section_header("Configuration Preview");

        println!("{}", "📋 Current Configuration:".bold());
        println!();

        // Download settings
        println!("{}", "⬇️ Download Settings:".blue().bold());
        println!(
            "  Output Path: {}",
            self.config
                .default_output_path
                .as_deref()
                .unwrap_or("(current directory)")
        );
        println!(
            "  Concurrent Downloads: {}",
            self.config.concurrent_downloads
        );
        println!("  Max Retries: {}", self.config.max_retries);
        println!("  Resume Downloads: {}", self.config.default_resume);
        println!("  Verbose Output: {}", self.config.default_verbose);
        println!("  Log Hash Errors: {}", self.config.default_log_hash_errors);
        println!();

        // Filter settings
        println!("{}", "🔍 Filter Settings:".blue().bold());
        println!(
            "  Include Extensions: {}",
            self.config
                .default_include_ext
                .as_deref()
                .unwrap_or("(none)")
        );
        println!(
            "  Exclude Extensions: {}",
            self.config
                .default_exclude_ext
                .as_deref()
                .unwrap_or("(none)")
        );
        println!(
            "  Max File Size: {}",
            self.config
                .default_max_file_size
                .as_deref()
                .unwrap_or("(unlimited)")
        );
        println!();

        // Advanced settings
        println!("{}", "⚙️ Advanced Settings:".blue().bold());
        println!("  HTTP Timeout: {} seconds", self.config.http_timeout);
        println!(
            "  User Agent: {}",
            self.config
                .user_agent_override
                .as_deref()
                .unwrap_or("(default)")
        );
        println!("  Dry Run Mode: {}", self.config.default_dry_run);
        println!();

        // Filter presets
        println!("{}", "📁 Filter Presets:".blue().bold());
        for preset in &self.config.filter_presets {
            println!("  • {} - {}", preset.name.bold(), preset.description);
        }
        println!();

        // Recent URLs
        println!("{}", "🕒 Recent URLs:".blue().bold());
        if self.config.recent_urls.is_empty() {
            println!("  (none)");
        } else {
            for (i, url) in self.config.recent_urls.iter().take(5).enumerate() {
                println!("  {}. {}", i + 1, url);
            }
            if self.config.recent_urls.len() > 5 {
                println!("  ... and {} more", self.config.recent_urls.len() - 5);
            }
        }

        self.pause();
        Ok(())
    }

    /// Save configuration and exit
    async fn save_and_exit(&mut self) -> Result<()> {
        println!("{}", "💾 Saving configuration...".cyan());

        match ConfigManager::validate_config(&self.config) {
            Ok(()) => {
                self.config_manager.save_config(&self.config)?;
                println!("{}", "✅ Configuration saved successfully!".green().bold());
                println!(
                    "Config file: {}",
                    self.config_manager
                        .config_file_path()
                        .display()
                        .to_string()
                        .dimmed()
                );
            }
            Err(e) => {
                println!("{} Configuration validation failed: {}", "❌".red(), e);
                println!("Please fix the issues before saving.");
                self.pause();
                return Ok(());
            }
        }

        Ok(())
    }

    // Helper methods for menu interactions

    fn print_section_header(&self, title: &str) {
        println!("\n{}", "=".repeat(50).cyan());
        println!("{} {}", "🔧".cyan(), title.bold());
        println!("{}", "=".repeat(50).cyan());
    }

    fn get_user_choice(&self, prompt: &str) -> Result<usize> {
        loop {
            print!("{} (1-9): ", prompt.bold());
            io::stdout().flush().unwrap();

            let mut input = String::new();
            io::stdin().read_line(&mut input).unwrap();

            match input.trim().parse::<usize>() {
                Ok(choice) => return Ok(choice),
                Err(_) => println!("{}", "Please enter a valid number.".red()),
            }
        }
    }

    fn get_string_input(&self, prompt: &str) -> Result<String> {
        print!("{}: ", prompt.bold());
        io::stdout().flush().unwrap();

        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap();

        Ok(input.trim().to_string())
    }

    fn pause(&self) {
        print!("\n{}", "Press Enter to continue...".dimmed());
        io::stdout().flush().unwrap();
        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap();
    }

    // Configuration setters

    fn set_output_path(&mut self) -> Result<()> {
        let path = self.get_string_input("Enter default output path (or press Enter to clear)")?;
        if path.is_empty() {
            self.config.default_output_path = None;
            println!("{}", "Output path cleared.".green());
        } else {
            self.config.default_output_path = Some(path);
            println!("{}", "Output path updated.".green());
        }
        Ok(())
    }

    fn set_concurrent_downloads(&mut self) -> Result<()> {
        loop {
            let input = self.get_string_input("Enter concurrent downloads (1-20)")?;
            match input.parse::<usize>() {
                Ok(num) if (1..=20).contains(&num) => {
                    self.config.concurrent_downloads = num;
                    println!("{}", "Concurrent downloads updated.".green());
                    break;
                }
                _ => println!("{}", "Please enter a number between 1 and 20.".red()),
            }
        }
        Ok(())
    }

    fn set_max_retries(&mut self) -> Result<()> {
        loop {
            let input = self.get_string_input("Enter max retries (1-10)")?;
            match input.parse::<usize>() {
                Ok(num) if (1..=10).contains(&num) => {
                    self.config.max_retries = num;
                    println!("{}", "Max retries updated.".green());
                    break;
                }
                _ => println!("{}", "Please enter a number between 1 and 10.".red()),
            }
        }
        Ok(())
    }

    fn set_include_extensions(&mut self) -> Result<()> {
        let exts = self.get_string_input(
            "Enter file extensions to include (comma-separated, e.g., pdf,txt,mp3)",
        )?;
        if exts.is_empty() {
            self.config.default_include_ext = None;
        } else {
            self.config.default_include_ext = Some(exts);
        }
        println!("{}", "Include extensions updated.".green());
        Ok(())
    }

    fn set_exclude_extensions(&mut self) -> Result<()> {
        let exts = self.get_string_input(
            "Enter file extensions to exclude (comma-separated, e.g., xml,log,tmp)",
        )?;
        if exts.is_empty() {
            self.config.default_exclude_ext = None;
        } else {
            self.config.default_exclude_ext = Some(exts);
        }
        println!("{}", "Exclude extensions updated.".green());
        Ok(())
    }

    fn set_max_file_size(&mut self) -> Result<()> {
        loop {
            let size = self.get_string_input(
                "Enter maximum file size (e.g., 100MB, 1GB, or press Enter to clear)",
            )?;
            if size.is_empty() {
                self.config.default_max_file_size = None;
                println!("{}", "File size limit cleared.".green());
                break;
            } else {
                match parse_size_string(&size) {
                    Ok(_) => {
                        self.config.default_max_file_size = Some(size);
                        println!("{}", "File size limit updated.".green());
                        break;
                    }
                    Err(e) => println!("{} Invalid size format: {}", "❌".red(), e),
                }
            }
        }
        Ok(())
    }

    fn set_http_timeout(&mut self) -> Result<()> {
        loop {
            let input = self.get_string_input("Enter HTTP timeout in seconds (1-300)")?;
            match input.parse::<u64>() {
                Ok(num) if (1..=300).contains(&num) => {
                    self.config.http_timeout = num;
                    println!("{}", "HTTP timeout updated.".green());
                    break;
                }
                _ => println!("{}", "Please enter a number between 1 and 300.".red()),
            }
        }
        Ok(())
    }

    fn set_user_agent(&mut self) -> Result<()> {
        let agent = self.get_string_input("Enter custom User-Agent string")?;
        if agent.is_empty() {
            self.config.user_agent_override = None;
        } else {
            self.config.user_agent_override = Some(agent);
        }
        println!("{}", "User agent updated.".green());
        Ok(())
    }

    fn set_max_recent_urls(&mut self) -> Result<()> {
        loop {
            let input = self.get_string_input("Enter max recent URLs to keep (1-100)")?;
            match input.parse::<usize>() {
                Ok(num) if (1..=100).contains(&num) => {
                    self.config.max_recent_urls = num;
                    // Trim current list if needed
                    self.config.recent_urls.truncate(num);
                    println!("{}", "Max recent URLs updated.".green());
                    break;
                }
                _ => println!("{}", "Please enter a number between 1 and 100.".red()),
            }
        }
        Ok(())
    }

    // Preset management methods

    fn apply_preset(&mut self) -> Result<()> {
        if self.config.filter_presets.is_empty() {
            println!("{}", "No presets available.".yellow());
            return Ok(());
        }

        loop {
            let choice = self.get_user_choice("Select preset to apply")?;
            if choice >= 1 && choice <= self.config.filter_presets.len() {
                let preset = &self.config.filter_presets[choice - 1];
                self.config.default_include_ext = preset.include_ext.clone();
                self.config.default_exclude_ext = preset.exclude_ext.clone();
                self.config.default_max_file_size = preset.max_file_size.clone();
                println!("{} Applied preset: {}", "✅".green(), preset.name.bold());
                break;
            } else {
                println!("{}", "Invalid preset number.".red());
            }
        }
        Ok(())
    }

    fn create_preset(&mut self) -> Result<()> {
        let name = self.get_string_input("Enter preset name")?;
        if name.is_empty() {
            println!("{}", "Preset name cannot be empty.".red());
            return Ok(());
        }

        let description = self.get_string_input("Enter preset description")?;
        let include_ext = self.get_string_input("Enter include extensions (optional)")?;
        let exclude_ext = self.get_string_input("Enter exclude extensions (optional)")?;
        let max_file_size = self.get_string_input("Enter max file size (optional, e.g., 100MB)")?;

        let preset = FilterPreset {
            name,
            description,
            include_ext: if include_ext.is_empty() {
                None
            } else {
                Some(include_ext)
            },
            exclude_ext: if exclude_ext.is_empty() {
                None
            } else {
                Some(exclude_ext)
            },
            max_file_size: if max_file_size.is_empty() {
                None
            } else {
                Some(max_file_size)
            },
        };

        self.config.filter_presets.push(preset);
        println!("{}", "Preset created successfully.".green());
        Ok(())
    }

    fn edit_preset(&mut self) -> Result<()> {
        if self.config.filter_presets.is_empty() {
            println!("{}", "No presets to edit.".yellow());
            return Ok(());
        }

        loop {
            let choice = self.get_user_choice("Select preset to edit")?;
            if choice >= 1 && choice <= self.config.filter_presets.len() {
                // Implementation for editing preset
                println!(
                    "{}",
                    "Preset editing functionality would be implemented here.".yellow()
                );
                break;
            } else {
                println!("{}", "Invalid preset number.".red());
            }
        }
        Ok(())
    }

    fn delete_preset(&mut self) -> Result<()> {
        if self.config.filter_presets.is_empty() {
            println!("{}", "No presets to delete.".yellow());
            return Ok(());
        }

        loop {
            let choice = self.get_user_choice("Select preset to delete")?;
            if choice >= 1 && choice <= self.config.filter_presets.len() {
                let preset = self.config.filter_presets.remove(choice - 1);
                println!("{} Deleted preset: {}", "🗑️".red(), preset.name.bold());
                break;
            } else {
                println!("{}", "Invalid preset number.".red());
            }
        }
        Ok(())
    }

    fn view_preset_details(&mut self) -> Result<()> {
        if self.config.filter_presets.is_empty() {
            println!("{}", "No presets available.".yellow());
            return Ok(());
        }

        loop {
            let choice = self.get_user_choice("Select preset to view")?;
            if choice >= 1 && choice <= self.config.filter_presets.len() {
                let preset = &self.config.filter_presets[choice - 1];
                println!("\n{} {}", "📁".cyan(), preset.name.bold());
                println!("Description: {}", preset.description);
                println!(
                    "Include Extensions: {}",
                    preset.include_ext.as_deref().unwrap_or("(none)")
                );
                println!(
                    "Exclude Extensions: {}",
                    preset.exclude_ext.as_deref().unwrap_or("(none)")
                );
                println!(
                    "Max File Size: {}",
                    preset.max_file_size.as_deref().unwrap_or("(unlimited)")
                );
                break;
            } else {
                println!("{}", "Invalid preset number.".red());
            }
        }
        Ok(())
    }

    fn reset_to_defaults(&mut self) -> Result<()> {
        println!(
            "{}",
            "⚠️ This will reset ALL settings to defaults. Are you sure? (y/N)"
                .yellow()
                .bold()
        );
        let confirm = self.get_string_input("")?;
        if confirm.to_lowercase() == "y" || confirm.to_lowercase() == "yes" {
            self.config = Config::default();
            println!("{}", "✅ All settings reset to defaults.".green());
        } else {
            println!("{}", "Reset cancelled.".yellow());
        }
        Ok(())
    }

    fn create_backup(&mut self) -> Result<()> {
        match self.config_manager.backup_config() {
            Ok(backup_path) => {
                println!(
                    "{} Backup created: {}",
                    "💾".green(),
                    backup_path.display().to_string().cyan()
                );
            }
            Err(e) => println!("{} Failed to create backup: {}", "❌".red(), e),
        }
        Ok(())
    }

    fn list_backups(&mut self) -> Result<()> {
        match self.config_manager.list_backups() {
            Ok(backups) => {
                if backups.is_empty() {
                    println!("{}", "No backups found.".yellow());
                } else {
                    println!("Available backups:");
                    for (i, backup) in backups.iter().enumerate() {
                        if let Some(name) = backup.file_name().and_then(|n| n.to_str()) {
                            println!("  {}. {}", (i + 1).to_string().cyan(), name);
                        }
                    }
                }
            }
            Err(e) => println!("{} Failed to list backups: {}", "❌".red(), e),
        }
        Ok(())
    }

    fn restore_backup(&mut self) -> Result<()> {
        // Implementation for restoring from backup
        println!(
            "{}",
            "Backup restoration functionality would be implemented here.".yellow()
        );
        Ok(())
    }

    fn show_config_location(&mut self) -> Result<()> {
        println!("Configuration file location:");
        println!(
            "  {}",
            self.config_manager
                .config_file_path()
                .display()
                .to_string()
                .cyan()
        );
        println!("Configuration directory:");
        println!(
            "  {}",
            self.config_manager
                .config_directory()
                .display()
                .to_string()
                .cyan()
        );
        Ok(())
    }
}

/// Launch the interactive configuration menu
pub async fn launch_config_menu() -> Result<()> {
    let mut menu = InteractiveMenu::new()?;
    menu.run().await
}
