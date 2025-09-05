//! CLI command handlers for configuration and history management

use crate::{
    error::IaGetError,
    infrastructure::{
        config::Config,
        persistence::{
            config_persistence::ConfigPersistence,
            download_history::{get_default_history_db_path, DownloadHistory, TaskStatus},
        },
    },
    utilities::filters::format_size,
    Result,
};
use colored::Colorize;
use std::io::{self, Write};

use super::{ConfigAction, HistoryAction};

/// Handle configuration commands
pub async fn handle_config_command(action: ConfigAction) -> Result<()> {
    let persistence = ConfigPersistence::new()?;

    match action {
        ConfigAction::Show => show_config(&persistence).await,
        ConfigAction::Set { key, value } => set_config(&persistence, &key, &value).await,
        ConfigAction::Unset { key } => unset_config(&persistence, &key).await,
        ConfigAction::Location => show_config_location(&persistence).await,
        ConfigAction::Reset => reset_config(&persistence).await,
        ConfigAction::Validate => validate_config(&persistence).await,
    }
}

/// Handle history commands
pub async fn handle_history_command(action: HistoryAction) -> Result<()> {
    let history_path = get_default_history_db_path()?;

    match action {
        HistoryAction::Show {
            limit,
            status,
            detailed,
        } => show_history(&history_path, limit, status.as_deref(), detailed).await,
        HistoryAction::Clear { force } => clear_history(&history_path, force).await,
        HistoryAction::Remove { id } => remove_history_entry(&history_path, &id).await,
        HistoryAction::Stats => show_history_stats(&history_path).await,
    }
}

/// Show current configuration
async fn show_config(persistence: &ConfigPersistence) -> Result<()> {
    println!("{} Current Configuration", "📋".blue().bold());
    println!();

    let config = persistence.load_config()?;

    // Show file location
    println!("{} Configuration File:", "📁".cyan());
    if persistence.config_exists() {
        println!(
            "  Location: {}",
            persistence
                .get_config_file_path()
                .display()
                .to_string()
                .green()
        );
    } else {
        println!(
            "  Location: {} {}",
            persistence
                .get_config_file_path()
                .display()
                .to_string()
                .dimmed(),
            "(file does not exist)".yellow()
        );
    }
    println!();

    // Show download settings
    println!("{} Download Settings:", "⬇️".blue());
    println!(
        "  Default output path: {}",
        format_option(&config.default_output_path)
    );
    println!(
        "  Concurrent downloads: {}",
        config.concurrent_downloads.to_string().cyan()
    );
    println!("  Max retries: {}", config.max_retries.to_string().cyan());
    println!(
        "  HTTP timeout: {} seconds",
        config.http_timeout.to_string().cyan()
    );
    println!();

    // Show filter settings
    println!("{} Filter Settings:", "🔍".green());
    println!(
        "  Default include extensions: {}",
        format_option(&config.default_include_ext)
    );
    println!(
        "  Default exclude extensions: {}",
        format_option(&config.default_exclude_ext)
    );
    println!(
        "  Default min file size: {}",
        format_option(&config.default_min_file_size)
    );
    println!(
        "  Default max file size: {}",
        format_option(&config.default_max_file_size)
    );
    println!();

    // Show behavior settings
    println!("{} Default Behavior:", "⚙️".yellow());
    println!("  Resume downloads: {}", format_bool(config.default_resume));
    println!("  Verbose output: {}", format_bool(config.default_verbose));
    println!(
        "  Log hash errors: {}",
        format_bool(config.default_log_hash_errors)
    );
    println!("  Dry run mode: {}", format_bool(config.default_dry_run));
    println!(
        "  HTTP compression: {}",
        format_bool(config.default_compress)
    );
    println!(
        "  Auto decompress: {}",
        format_bool(config.default_decompress)
    );
    println!(
        "  Decompress formats: {}",
        format_option(&config.default_decompress_formats)
    );
    println!();

    // Show advanced settings
    println!("{} Advanced Settings:", "🔧".purple());
    println!(
        "  User agent override: {}",
        format_option(&config.user_agent_override)
    );
    println!(
        "  Max recent URLs: {}",
        config.max_recent_urls.to_string().cyan()
    );
    println!();

    // Show filter presets
    if !config.filter_presets.is_empty() {
        println!("{} Filter Presets:", "📝".magenta());
        for preset in &config.filter_presets {
            println!(
                "  • {} - {}",
                preset.name.bright_green(),
                preset.description.dimmed()
            );
            if let Some(ref include) = preset.include_ext {
                println!("    Include: {}", include.cyan());
            }
            if let Some(ref exclude) = preset.exclude_ext {
                println!("    Exclude: {}", exclude.red());
            }
            if let Some(ref max_size) = preset.max_file_size {
                println!("    Max size: {}", max_size.yellow());
            }
        }
        println!();
    }

    // Show recent URLs
    if !config.recent_urls.is_empty() {
        println!("{} Recent URLs:", "🕒".bright_blue());
        for (i, url) in config.recent_urls.iter().enumerate() {
            println!("  {}. {}", (i + 1).to_string().dimmed(), url.bright_blue());
        }
        println!();
    }

    println!(
        "{} Use 'ia-get config set <key> <value>' to modify settings",
        "💡".yellow()
    );
    println!(
        "{} Use 'ia-get config unset <key>' to reset to default",
        "💡".yellow()
    );

    Ok(())
}

/// Set a configuration value
async fn set_config(persistence: &ConfigPersistence, key: &str, value: &str) -> Result<()> {
    let mut config = persistence.load_config().unwrap_or_default();

    match key {
        "default_output_path" => {
            config.default_output_path = if value.is_empty() {
                None
            } else {
                Some(value.to_string())
            };
        }
        "concurrent_downloads" => {
            let val: usize = value.parse().map_err(|_| {
                IaGetError::Config("concurrent_downloads must be a number".to_string())
            })?;
            if val == 0 || val > 20 {
                return Err(IaGetError::Config(
                    "concurrent_downloads must be between 1 and 20".to_string(),
                ));
            }
            config.concurrent_downloads = val;
        }
        "max_retries" => {
            let val: usize = value
                .parse()
                .map_err(|_| IaGetError::Config("max_retries must be a number".to_string()))?;
            if val > 50 {
                return Err(IaGetError::Config(
                    "max_retries must be 50 or less".to_string(),
                ));
            }
            config.max_retries = val;
        }
        "default_include_ext" => {
            config.default_include_ext = if value.is_empty() {
                None
            } else {
                Some(value.to_string())
            };
        }
        "default_exclude_ext" => {
            config.default_exclude_ext = if value.is_empty() {
                None
            } else {
                Some(value.to_string())
            };
        }
        "default_max_file_size" => {
            config.default_max_file_size = if value.is_empty() {
                None
            } else {
                Some(value.to_string())
            };
        }
        "default_resume" => {
            config.default_resume = parse_bool(value)?;
        }
        "default_verbose" => {
            config.default_verbose = parse_bool(value)?;
        }
        "default_log_hash_errors" => {
            config.default_log_hash_errors = parse_bool(value)?;
        }
        "default_dry_run" => {
            config.default_dry_run = parse_bool(value)?;
        }
        "default_compress" => {
            config.default_compress = parse_bool(value)?;
        }
        "default_decompress" => {
            config.default_decompress = parse_bool(value)?;
        }
        "http_timeout" => {
            let val: u64 = value
                .parse()
                .map_err(|_| IaGetError::Config("http_timeout must be a number".to_string()))?;
            if !(5..=600).contains(&val) {
                return Err(IaGetError::Config(
                    "http_timeout must be between 5 and 600 seconds".to_string(),
                ));
            }
            config.http_timeout = val;
        }
        "user_agent_override" => {
            config.user_agent_override = if value.is_empty() {
                None
            } else {
                Some(value.to_string())
            };
        }
        _ => {
            return Err(IaGetError::Config(format!(
                "Unknown configuration key: '{}'.\n\n{} Valid keys:\n  {}\n\n{} Use 'ia-get config show' to see current values",
                key.bright_red(),
                "💡".bright_yellow(),
                [
                    "default_output_path", "concurrent_downloads", "max_retries",
                    "default_include_ext", "default_exclude_ext", "default_min_file_size",
                    "default_max_file_size", "default_resume", "default_verbose",
                    "default_log_hash_errors", "default_dry_run", "default_compress",
                    "default_decompress", "http_timeout", "user_agent_override"
                ].join(", ").bright_cyan(),
                "💡".bright_yellow()
            )));
        }
    }

    persistence.save_config(&config)?;

    println!(
        "{} Configuration updated: {} = {}",
        "✅".green(),
        key.bright_cyan(),
        value.bright_green()
    );
    println!(
        "{} Configuration saved to: {}",
        "💾".blue(),
        persistence.get_config_file_path().display()
    );

    Ok(())
}

/// Unset a configuration value (reset to default)
async fn unset_config(persistence: &ConfigPersistence, key: &str) -> Result<()> {
    let mut config = persistence.load_config().unwrap_or_default();
    let default_config = Config::default();

    match key {
        "default_output_path" => config.default_output_path = default_config.default_output_path,
        "concurrent_downloads" => config.concurrent_downloads = default_config.concurrent_downloads,
        "max_retries" => config.max_retries = default_config.max_retries,
        "default_include_ext" => config.default_include_ext = default_config.default_include_ext,
        "default_exclude_ext" => config.default_exclude_ext = default_config.default_exclude_ext,
        "default_max_file_size" => {
            config.default_max_file_size = default_config.default_max_file_size
        }
        "default_resume" => config.default_resume = default_config.default_resume,
        "default_verbose" => config.default_verbose = default_config.default_verbose,
        "default_log_hash_errors" => {
            config.default_log_hash_errors = default_config.default_log_hash_errors
        }
        "default_dry_run" => config.default_dry_run = default_config.default_dry_run,
        "default_compress" => config.default_compress = default_config.default_compress,
        "default_decompress" => config.default_decompress = default_config.default_decompress,
        "http_timeout" => config.http_timeout = default_config.http_timeout,
        "user_agent_override" => config.user_agent_override = default_config.user_agent_override,
        _ => {
            return Err(IaGetError::Config(format!(
                "Unknown configuration key: '{}'.\n\n{} Valid keys:\n  {}\n\n{} Use 'ia-get config show' to see current values",
                key.bright_red(),
                "💡".bright_yellow(),
                [
                    "default_output_path", "concurrent_downloads", "max_retries",
                    "default_include_ext", "default_exclude_ext", "default_min_file_size",
                    "default_max_file_size", "default_resume", "default_verbose",
                    "default_log_hash_errors", "default_dry_run", "default_compress",
                    "default_decompress", "http_timeout", "user_agent_override"
                ].join(", ").bright_cyan(),
                "💡".bright_yellow()
            )));
        }
    }

    persistence.save_config(&config)?;

    println!(
        "{} Configuration key '{}' reset to default",
        "✅".green(),
        key.bright_cyan()
    );

    Ok(())
}

/// Show configuration file location
async fn show_config_location(persistence: &ConfigPersistence) -> Result<()> {
    println!("{} Configuration File Location", "📁".blue().bold());
    println!();

    println!(
        "Primary config file: {}",
        persistence
            .get_config_file_path()
            .display()
            .to_string()
            .bright_green()
    );
    println!(
        "Config directory: {}",
        persistence
            .get_config_directory()
            .display()
            .to_string()
            .cyan()
    );

    if persistence.config_exists() {
        println!("Status: {}", "File exists".green());
    } else {
        println!(
            "Status: {} (will be created when settings are saved)",
            "File does not exist".yellow()
        );
    }

    Ok(())
}

/// Reset all configuration to defaults
async fn reset_config(persistence: &ConfigPersistence) -> Result<()> {
    print!(
        "{} This will reset ALL configuration to defaults. Continue? [y/N]: ",
        "⚠️".yellow()
    );
    io::stdout().flush().unwrap();

    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();

    if !matches!(input.trim().to_lowercase().as_str(), "y" | "yes") {
        println!("Configuration reset cancelled.");
        return Ok(());
    }

    let default_config = Config::default();
    persistence.save_config(&default_config)?;

    println!("{} All configuration reset to defaults", "✅".green());

    Ok(())
}

/// Validate configuration
async fn validate_config(persistence: &ConfigPersistence) -> Result<()> {
    println!("{} Validating Configuration", "🔍".blue().bold());
    println!();

    match persistence.load_config() {
        Ok(config) => {
            println!("{} Configuration file is valid", "✅".green());

            // Validate specific values
            let mut warnings = Vec::new();

            if config.concurrent_downloads > 10 {
                warnings.push("concurrent_downloads is quite high, consider reducing for Archive.org compatibility".to_string());
            }

            if config.http_timeout < 10 {
                warnings.push(
                    "http_timeout is quite low, may cause timeouts for large files".to_string(),
                );
            }

            if config.max_retries > 10 {
                warnings.push(
                    "max_retries is quite high, failed downloads may take a long time".to_string(),
                );
            }

            if warnings.is_empty() {
                println!("{} No validation warnings", "✅".green());
            } else {
                println!("{} Validation warnings:", "⚠️".yellow());
                for warning in warnings {
                    println!("  • {}", warning.yellow());
                }
            }
        }
        Err(e) => {
            println!("{} Configuration validation failed: {}", "❌".red(), e);
            return Err(e);
        }
    }

    Ok(())
}

/// Show download history
async fn show_history(
    history_path: &std::path::Path,
    limit: usize,
    status_filter: Option<&str>,
    detailed: bool,
) -> Result<()> {
    let history = DownloadHistory::load_or_create(history_path)?;

    println!("{} Download History", "📚".blue().bold());
    println!();

    if history.entries.is_empty() {
        println!("{} No download history found", "ℹ️".blue());
        return Ok(());
    }

    // Filter by status if requested
    let entries: Vec<_> = if let Some(status_str) = status_filter {
        let target_status = match status_str.to_lowercase().as_str() {
            "success" => TaskStatus::Success,
            "failed" => TaskStatus::Failed("".to_string()),
            "in_progress" | "inprogress" => TaskStatus::InProgress,
            "cancelled" => TaskStatus::Cancelled,
            "paused" => TaskStatus::Paused,
            _ => return Err(IaGetError::Config(format!("Invalid status filter: {}. Valid options: success, failed, in_progress, cancelled, paused", status_str))),
        };
        history.get_entries_by_status(&target_status)
    } else {
        history.get_recent_entries(limit)
    };

    if entries.is_empty() {
        println!("{} No entries match the specified criteria", "ℹ️".blue());
        return Ok(());
    }

    for (i, entry) in entries.iter().enumerate() {
        if i >= limit {
            break;
        }

        let status_display = match &entry.status {
            TaskStatus::Success => "✅ Success".green(),
            TaskStatus::Failed(msg) => format!("❌ Failed: {}", msg).red(),
            TaskStatus::InProgress => "🔄 In Progress".yellow(),
            TaskStatus::Cancelled => "⏹️ Cancelled".cyan(),
            TaskStatus::Paused => "⏸️ Paused".blue(),
        };

        println!(
            "{} {}",
            format!("{}.", i + 1).dimmed(),
            entry.archive_identifier.bright_green()
        );
        println!("    ID: {}", entry.id.cyan());
        println!("    Status: {}", status_display);
        println!(
            "    Started: {}",
            entry
                .started_at
                .format("%Y-%m-%d %H:%M:%S UTC")
                .to_string()
                .dimmed()
        );

        if let Some(completed) = entry.completed_at {
            println!(
                "    Completed: {}",
                completed
                    .format("%Y-%m-%d %H:%M:%S UTC")
                    .to_string()
                    .dimmed()
            );
        }

        if detailed {
            println!("    Original input: {}", entry.original_input.blue());
            println!("    Output directory: {}", entry.output_directory.cyan());
            println!(
                "    Progress: {}/{} files ({:.1}%)",
                entry.completed_files,
                entry.total_files,
                entry.completion_percentage()
            );
            println!(
                "    Data downloaded: {}",
                format_size(entry.bytes_downloaded)
            );

            if entry.failed_files > 0 {
                println!("    Failed files: {}", entry.failed_files.to_string().red());
            }
        }

        println!();
    }

    if entries.len() == limit && history.entries.len() > limit {
        println!(
            "{} Showing {} of {} entries. Use --limit to see more.",
            "ℹ️".blue(),
            limit,
            history.entries.len()
        );
    }

    Ok(())
}

/// Clear download history
async fn clear_history(history_path: &std::path::Path, force: bool) -> Result<()> {
    if !force {
        print!(
            "{} This will clear ALL download history. Continue? [y/N]: ",
            "⚠️".yellow()
        );
        io::stdout().flush().unwrap();

        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap();

        if !matches!(input.trim().to_lowercase().as_str(), "y" | "yes") {
            println!("History clear cancelled.");
            return Ok(());
        }
    }

    let mut history = DownloadHistory::load_or_create(history_path)?;
    let count = history.entries.len();
    history.clear();
    history.save_to_file(history_path)?;

    println!("{} Cleared {} history entries", "✅".green(), count);

    Ok(())
}

/// Remove specific history entry
async fn remove_history_entry(history_path: &std::path::Path, id: &str) -> Result<()> {
    let mut history = DownloadHistory::load_or_create(history_path)?;

    if history.remove_entry(id) {
        history.save_to_file(history_path)?;
        println!("{} Removed history entry: {}", "✅".green(), id.cyan());
    } else {
        return Err(IaGetError::Config(format!(
            "History entry with ID '{}' not found",
            id
        )));
    }

    Ok(())
}

/// Show download history statistics
async fn show_history_stats(history_path: &std::path::Path) -> Result<()> {
    let history = DownloadHistory::load_or_create(history_path)?;

    println!("{} Download History Statistics", "📊".blue().bold());
    println!();

    if history.entries.is_empty() {
        println!("{} No download history found", "ℹ️".blue());
        return Ok(());
    }

    let stats = history.get_statistics();

    println!("{} Overall Statistics:", "📈".green());
    println!(
        "  Total downloads: {}",
        stats.total_downloads.to_string().cyan()
    );
    println!(
        "  Successful: {} ({:.1}%)",
        stats.successful_downloads.to_string().green(),
        (stats.successful_downloads as f32 / stats.total_downloads as f32) * 100.0
    );
    println!(
        "  Failed: {} ({:.1}%)",
        stats.failed_downloads.to_string().red(),
        (stats.failed_downloads as f32 / stats.total_downloads as f32) * 100.0
    );

    if stats.in_progress_downloads > 0 {
        println!(
            "  In progress: {}",
            stats.in_progress_downloads.to_string().yellow()
        );
    }

    if stats.cancelled_downloads > 0 {
        println!(
            "  Cancelled: {}",
            stats.cancelled_downloads.to_string().cyan()
        );
    }

    println!();
    println!("{} Data Transfer:", "💾".blue());
    println!(
        "  Total data downloaded: {}",
        format_size(stats.total_bytes_downloaded).bright_green()
    );
    println!(
        "  Total files downloaded: {}",
        stats.total_files_downloaded.to_string().cyan()
    );

    if stats.total_files_downloaded > 0 {
        let avg_file_size = stats.total_bytes_downloaded / stats.total_files_downloaded as u64;
        println!(
            "  Average file size: {}",
            format_size(avg_file_size).yellow()
        );
    }

    println!();
    println!("{} Database Information:", "🗃️".purple());
    println!("  Database version: {}", history.version.cyan());
    println!(
        "  Created: {}",
        history
            .created_at
            .format("%Y-%m-%d %H:%M:%S UTC")
            .to_string()
            .dimmed()
    );
    println!(
        "  Last updated: {}",
        history
            .last_updated
            .format("%Y-%m-%d %H:%M:%S UTC")
            .to_string()
            .dimmed()
    );
    println!("  Max entries: {}", history.max_entries.to_string().cyan());

    Ok(())
}

/// Helper function to format optional strings
fn format_option(opt: &Option<String>) -> colored::ColoredString {
    match opt {
        Some(value) => value.green(),
        None => "(not set)".dimmed(),
    }
}

/// Helper function to format boolean values
fn format_bool(value: bool) -> colored::ColoredString {
    if value {
        "enabled".green()
    } else {
        "disabled".red()
    }
}

/// Helper function to parse boolean values from strings
fn parse_bool(value: &str) -> Result<bool> {
    match value.to_lowercase().as_str() {
        "true" | "yes" | "on" | "1" | "enabled" => Ok(true),
        "false" | "no" | "off" | "0" | "disabled" => Ok(false),
        _ => Err(IaGetError::Config(format!(
            "Invalid boolean value: '{}'. Use true/false, yes/no, on/off, 1/0, or enabled/disabled",
            value
        ))),
    }
}
