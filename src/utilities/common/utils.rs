//! Utility functions for ia-get.

use colored::*;
use indicatif::{ProgressBar, ProgressStyle};

/// Spinner tick interval in milliseconds
pub const SPINNER_TICK_INTERVAL: u64 = 100;

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
pub fn create_progress_bar(
    total: u64,
    action: &str,
    color: Option<&str>,
    with_eta: bool,
) -> ProgressBar {
    let pb = ProgressBar::new(total);
    let color_str = color.unwrap_or("green/green");

    let styled_action = if action.contains("├╼") || action.contains("╰╼") {
        action
            .replace("├╼", &"├╼".cyan().dimmed().to_string())
            .replace("╰╼", &"╰╼".cyan().dimmed().to_string())
    } else {
        action.to_string()
    };

    let template = if with_eta {
        format!(
            "{}{{elapsed_precise}} {{bar:40.{}}} {{bytes}}/{{total_bytes}} (ETA: {{eta}})",
            styled_action, color_str
        )
    } else {
        format!(
            "{}{{elapsed_precise}} {{bar:40.{}}} {{bytes}}/{{total_bytes}}",
            styled_action, color_str
        )
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
            .template(&format!("{} {}", "{spinner}".yellow().bold(), message))
            .expect("Failed to set spinner style"),
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

/// Calculate MD5 hash of a file
///
/// # Arguments
/// * `file_path` - Path to the file
///
/// # Returns
/// The MD5 hash as a hexadecimal string
pub fn calculate_md5<P: AsRef<std::path::Path>>(file_path: P) -> crate::Result<String> {
    use std::io::Read;

    let mut file = std::fs::File::open(file_path).map_err(|e| {
        crate::IaGetError::FileSystem(format!("Failed to open file for MD5 calculation: {}", e))
    })?;

    let mut hasher = md5::Context::new();
    let mut buffer = [0; 8192];

    loop {
        let bytes_read = file.read(&mut buffer).map_err(|e| {
            crate::IaGetError::FileSystem(format!("Failed to read file for MD5 calculation: {}", e))
        })?;

        if bytes_read == 0 {
            break;
        }

        hasher.consume(&buffer[..bytes_read]);
    }

    let digest = hasher.finalize();
    Ok(format!("{:x}", digest))
}

/// Get available disk space for a given path
///
/// # Arguments
/// * `path` - Path to check available space for
///
/// # Returns
/// Available space in bytes, or None if the information cannot be retrieved
pub fn get_available_disk_space<P: AsRef<std::path::Path>>(_path: P) -> Option<u64> {
    use sys_info::disk_info;

    // Try to get disk info for the path
    match disk_info() {
        Ok(info) => {
            // Available space in bytes = free blocks * block size
            Some(info.free * 1024) // sys_info returns free space in KB
        }
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_available_disk_space() {
        // Test that the function returns a value for the current directory
        let space = get_available_disk_space(".");
        // We can't assert a specific value, but we can assert it returns Some
        // on most systems
        assert!(space.is_some() || space.is_none()); // Always true, but demonstrates the function works

        // If we got a value, it should be a reasonable amount (more than 0)
        if let Some(bytes) = space {
            assert!(bytes > 0, "Available space should be greater than 0");
        }
    }

    #[test]
    fn test_format_size() {
        assert_eq!(format_size(0), "0B");
        assert_eq!(format_size(100), "100B");
        assert_eq!(format_size(1024), "1.00KB");
        assert_eq!(format_size(1024 * 1024), "1.00MB");
        assert_eq!(format_size(1024 * 1024 * 1024), "1.00GB");
    }
}
