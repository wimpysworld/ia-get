//! Utility functions for ia-get.

use indicatif::{ProgressBar, ProgressStyle};
use regex::Regex;
use std::sync::LazyLock;
use crate::constants::URL_PATTERN;
use crate::{Result, IaGetError};

/// Spinner tick interval in milliseconds
pub const SPINNER_TICK_INTERVAL: u64 = 100;

/// Compiled regex for URL validation (initialized once)
static URL_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(URL_PATTERN).expect("Invalid URL regex pattern")
});

/// Validates an archive.org details URL format
/// 
/// # Arguments
/// * `url` - The URL to validate
/// 
/// # Returns
/// * `Ok(())` if the URL is valid
/// * `Err(IaGetError::UrlFormat)` if the URL format is invalid
/// 
/// # Examples
/// ```
/// use ia_get::utils::validate_archive_url;
/// 
/// assert!(validate_archive_url("https://archive.org/details/valid-item").is_ok());
/// assert!(validate_archive_url("https://example.com/invalid").is_err());
/// ```
pub fn validate_archive_url(url: &str) -> Result<()> {
    if URL_REGEX.is_match(url) {
        Ok(())
    } else {
        Err(IaGetError::UrlFormat(format!(
            "URL '{}' does not match expected format. Expected: https://archive.org/details/<identifier>", 
            url
        )))
    }
}

/// Create a progress bar with consistent styling
/// 
/// # Arguments
/// * `total` - Total value for the progress bar
/// * `action` - Action text to show at the beginning (e.g., "╰╼ Downloading  ")
/// * `color` - Optional color style (defaults to "green/green")
/// * `with_eta` - Whether to include ETA in the template
/// 
/// # Returns
/// A configured progress bar
pub fn create_progress_bar(total: u64, action: &str, color: Option<&str>, with_eta: bool) -> ProgressBar {
    let pb = ProgressBar::new(total);
    let color_str = color.unwrap_or("green/green");
    
    let template = if with_eta {
        format!("{action}{{elapsed_precise}}     {{bar:40.{color_str}}} {{bytes}}/{{total_bytes}} (ETA: {{eta}})")
    } else {
        format!("{action}{{elapsed_precise}}     {{bar:40.{color_str}}} {{bytes}}/{{total_bytes}}")
    };
    
    pb.set_style(
        ProgressStyle::default_bar()
            .template(&template)
            .expect("Failed to set progress bar style")
            .progress_chars("▓▒░"),
    );
    
    pb
}

/// Create a spinner with braille animation
/// 
/// # Arguments
/// * `message` - Message to display next to the spinner
/// 
/// # Returns
/// A configured spinner
pub fn create_spinner(message: &str) -> ProgressBar {
    let spinner = ProgressBar::new_spinner();
    spinner.set_style(
        ProgressStyle::default_spinner()
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏")
            .template(&format!("{{spinner}} {message}"))
            .expect("Failed to set spinner style")
    );
    spinner.enable_steady_tick(std::time::Duration::from_millis(SPINNER_TICK_INTERVAL));
    spinner
}

/// Format a duration into a human-readable string
pub fn format_duration(duration: std::time::Duration) -> String {
    let total_secs = duration.as_secs();
    if total_secs < 60 {
        return format!("{}.{:02}s", total_secs, duration.subsec_millis() / 10);
    }
    
    let hours = total_secs / 3600;
    let mins = (total_secs % 3600) / 60;
    let secs = total_secs % 60;
    
    if hours > 0 {
        format!("{}h {}m {}s", hours, mins, secs)
    } else {
        format!("{}m {}s", mins, secs)
    }
}

/// Format a size in bytes to a human-readable string
pub fn format_size(size: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    
    if size < KB {
        format!("{}B", size)
    } else if size < MB {
        format!("{:.2}KB", size as f64 / KB as f64)
    } else if size < GB {
        format!("{:.2}MB", size as f64 / MB as f64)
    } else {
        format!("{:.2}GB", size as f64 / GB as f64)
    }
}

/// Format transfer rate to appropriate units
pub fn format_transfer_rate(bytes_per_sec: f64) -> (f64, &'static str) {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    
    if bytes_per_sec < KB {
        (bytes_per_sec, "B")
    } else if bytes_per_sec < MB {
        (bytes_per_sec / KB, "KB")
    } else if bytes_per_sec < GB {
        (bytes_per_sec / MB, "MB")
    } else {
        (bytes_per_sec / GB, "GB")
    }
}
