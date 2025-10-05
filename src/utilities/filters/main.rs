//! File filtering and formatting utilities
//!
//! This module provides comprehensive file filtering capabilities for selecting
//! specific files from Internet Archive collections based on various criteria.
//!
//! ## Features
//!
//! - **Extension Filtering**: Include/exclude files by file extension
//! - **Size Filtering**: Filter files by maximum size with human-readable formats
//! - **Size Formatting**: Convert bytes to human-readable format (KB, MB, GB, etc.)
//! - **Pattern Matching**: Flexible filtering with multiple extension support
//!
//! ## Usage Examples
//!
//! ```rust
//! use ia_get::filters::{parse_size_string, format_size};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Parse size strings
//! let size = parse_size_string("100MB")?; // Returns 104857600 bytes
//! let size = parse_size_string("2.5GB")?; // Returns 2684354560 bytes
//!
//! // Format bytes to human-readable
//! let formatted = format_size(1048576); // Returns "1.0 MB"
//! # Ok(())
//! # }
//! ```
//!
//! ## Supported Size Units
//!
//! - **B**: Bytes
//! - **KB**: Kilobytes (1000 bytes)
//! - **MB**: Megabytes (1000 KB)
//! - **GB**: Gigabytes (1000 MB)
//! - **TB**: Terabytes (1000 GB)

use crate::{
    Result,
    core::archive::FileEntry,
    error::IaGetError,
    interface::cli::{Cli, Commands, SourceType},
};

/// Trait for extracting filter options from different CLI structures
pub trait FilterOptions {
    fn include_ext(&self) -> &Option<String>;
    fn exclude_ext(&self) -> &Option<String>;
    fn max_file_size(&self) -> &Option<String>;
    fn source_types(&self) -> Vec<SourceType>;

    /// Get resolved extensions to include (combining manual extensions and format categories)
    fn get_resolved_include_extensions(&self) -> Vec<String> {
        self.include_ext()
            .as_ref()
            .map(|s| s.split(',').map(|ext| ext.trim().to_lowercase()).collect())
            .unwrap_or_default()
    }

    /// Get resolved extensions to exclude (combining manual extensions and format categories)
    fn get_resolved_exclude_extensions(&self) -> Vec<String> {
        self.exclude_ext()
            .as_ref()
            .map(|s| s.split(',').map(|ext| ext.trim().to_lowercase()).collect())
            .unwrap_or_default()
    }
}

impl FilterOptions for Commands {
    fn include_ext(&self) -> &Option<String> {
        match self {
            Commands::Download { include_ext, .. } => include_ext,
            Commands::ListFormats { .. } => &None,
        }
    }

    fn exclude_ext(&self) -> &Option<String> {
        match self {
            Commands::Download { exclude_ext, .. } => exclude_ext,
            Commands::ListFormats { .. } => &None,
        }
    }

    fn max_file_size(&self) -> &Option<String> {
        match self {
            Commands::Download { max_file_size, .. } => max_file_size,
            Commands::ListFormats { .. } => &None,
        }
    }

    fn source_types(&self) -> Vec<SourceType> {
        // Commands doesn't have source filtering - return default
        vec![SourceType::Original]
    }
}

impl FilterOptions for Cli {
    fn include_ext(&self) -> &Option<String> {
        &self.include_ext
    }

    fn exclude_ext(&self) -> &Option<String> {
        &self.exclude_ext
    }

    fn max_file_size(&self) -> &Option<String> {
        &self.max_file_size
    }

    fn source_types(&self) -> Vec<SourceType> {
        self.get_source_types()
    }

    fn get_resolved_include_extensions(&self) -> Vec<String> {
        self.get_include_extensions()
    }

    fn get_resolved_exclude_extensions(&self) -> Vec<String> {
        self.get_exclude_extensions()
    }
}

/// Parse a size string like "100MB", "2GB", "500KB" into bytes
pub fn parse_size_string(size_str: &str) -> Result<u64> {
    let size_str = size_str.trim().to_uppercase();

    // Extract the numeric part and unit
    let (number_str, unit) = if size_str.ends_with("GB")
        || size_str.ends_with("TB")
        || size_str.ends_with("MB")
        || size_str.ends_with("KB")
    {
        (
            &size_str[..size_str.len() - 2],
            &size_str[size_str.len() - 2..],
        )
    } else if size_str.ends_with("B") {
        (&size_str[..size_str.len() - 1], "B")
    } else {
        // No unit, assume bytes
        (size_str.as_str(), "B")
    };

    let number: f64 = number_str
        .parse()
        .map_err(|_| IaGetError::Parse(format!("Invalid number: {}", number_str)))?;

    if number < 0.0 {
        return Err(IaGetError::Parse("Size cannot be negative".to_string()));
    }

    let bytes = match unit {
        "B" => number as u64,
        "KB" => (number * 1024.0) as u64,
        "MB" => (number * 1024.0 * 1024.0) as u64,
        "GB" => (number * 1024.0 * 1024.0 * 1024.0) as u64,
        "TB" => (number * 1024.0 * 1024.0 * 1024.0 * 1024.0) as u64,
        _ => return Err(IaGetError::Parse(format!("Unknown unit: {}", unit))),
    };

    Ok(bytes)
}

/// Filter file objects based on CLI criteria  
pub fn filter_files<T: FilterOptions, F: FileEntry>(files: Vec<F>, options: &T) -> Vec<F> {
    let include_extensions = options.get_resolved_include_extensions();
    let exclude_extensions = options.get_resolved_exclude_extensions();

    let max_size = options
        .max_file_size()
        .as_ref()
        .and_then(|s| parse_size_string(s).ok());

    let allowed_sources = options.source_types();

    files
        .into_iter()
        .filter(|file| {
            // Check source type filtering
            let file_source = file.source();
            if !allowed_sources
                .iter()
                .any(|source_type| source_type.matches(file_source))
            {
                return false;
            }

            // Get file extension
            let extension = std::path::Path::new(file.name())
                .extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or("")
                .to_lowercase();

            // Check include extensions
            if !include_extensions.is_empty() && !include_extensions.contains(&extension) {
                return false;
            }

            // Check exclude extensions
            if !exclude_extensions.is_empty() && exclude_extensions.contains(&extension) {
                return false;
            }

            // Check file size
            if let (Some(max), Some(file_size)) = (max_size, file.size()) {
                if file_size > max {
                    return false;
                }
            }

            true
        })
        .collect()
}

/// Format a byte size for human-readable display
pub fn format_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    const THRESHOLD: f64 = 1024.0;

    if bytes == 0 {
        return "0 B".to_string();
    }

    let mut size = bytes as f64;
    let mut unit_index = 0;

    while size >= THRESHOLD && unit_index < UNITS.len() - 1 {
        size /= THRESHOLD;
        unit_index += 1;
    }

    if unit_index == 0 {
        format!("{} {}", bytes, UNITS[unit_index])
    } else {
        format!("{:.1} {}", size, UNITS[unit_index])
    }
}
