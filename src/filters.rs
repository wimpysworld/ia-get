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

use crate::{archive_metadata::FileEntry, cli::Commands, error::IaGetError, Result};

/// Trait for extracting filter options from different CLI structures
pub trait FilterOptions {
    fn include_ext(&self) -> &Option<String>;
    fn exclude_ext(&self) -> &Option<String>;
    fn max_file_size(&self) -> &Option<String>;
}

impl FilterOptions for Commands {
    fn include_ext(&self) -> &Option<String> {
        match self {
            Commands::Download { include_ext, .. } => include_ext,
        }
    }

    fn exclude_ext(&self) -> &Option<String> {
        match self {
            Commands::Download { exclude_ext, .. } => exclude_ext,
        }
    }

    fn max_file_size(&self) -> &Option<String> {
        match self {
            Commands::Download { max_file_size, .. } => max_file_size,
        }
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
    let include_extensions: Option<Vec<String>> = options
        .include_ext()
        .as_ref()
        .map(|s| s.split(',').map(|ext| ext.trim().to_lowercase()).collect());

    let exclude_extensions: Option<Vec<String>> = options
        .exclude_ext()
        .as_ref()
        .map(|s| s.split(',').map(|ext| ext.trim().to_lowercase()).collect());

    let max_size = options
        .max_file_size()
        .as_ref()
        .and_then(|s| parse_size_string(s).ok());

    files
        .into_iter()
        .filter(|file| {
            // Get file extension
            let extension = std::path::Path::new(file.name())
                .extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or("")
                .to_lowercase();

            // Check include extensions
            if let Some(ref include_exts) = include_extensions {
                if !include_exts.contains(&extension) {
                    return false;
                }
            }

            // Check exclude extensions
            if let Some(ref exclude_exts) = exclude_extensions {
                if exclude_exts.contains(&extension) {
                    return false;
                }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_size_string() {
        assert_eq!(parse_size_string("100").unwrap(), 100);
        assert_eq!(parse_size_string("100B").unwrap(), 100);
        assert_eq!(parse_size_string("1KB").unwrap(), 1024);
        assert_eq!(parse_size_string("1MB").unwrap(), 1024 * 1024);
        assert_eq!(parse_size_string("1GB").unwrap(), 1024 * 1024 * 1024);
        assert_eq!(
            parse_size_string("1.5MB").unwrap(),
            (1.5 * 1024.0 * 1024.0) as u64
        );

        assert!(parse_size_string("invalid").is_err());
    }

    #[test]
    fn test_format_size() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(1024 * 1024), "1.0 MB");
        assert_eq!(format_size(1024 * 1024 * 1024), "1.0 GB");
    }

    #[test]
    fn test_filter_files() {
        use crate::archive_metadata::JsonFile;

        let files = vec![
            JsonFile {
                name: "test.txt".to_string(),
                source: "original".to_string(),
                mtime: None,
                size: Some(1024),
                format: None,
                rotation: None,
                md5: None,
                crc32: None,
                sha1: None,
                btih: None,
                summation: None,
                original: None,
            },
            JsonFile {
                name: "image.jpg".to_string(),
                source: "original".to_string(),
                mtime: None,
                size: Some(2048),
                format: None,
                rotation: None,
                md5: None,
                crc32: None,
                sha1: None,
                btih: None,
                summation: None,
                original: None,
            },
            JsonFile {
                name: "document.pdf".to_string(),
                source: "original".to_string(),
                mtime: None,
                size: Some(5000000), // ~5MB
                format: None,
                rotation: None,
                md5: None,
                crc32: None,
                sha1: None,
                btih: None,
                summation: None,
                original: None,
            },
        ];

        let cli = Commands::Download {
            url: "test".to_string(),
            output: None,
            include_ext: Some("txt,jpg".to_string()),
            exclude_ext: None,
            max_file_size: Some("1MB".to_string()),
            compress: false,
        };

        let filtered = filter_files(files, &cli);
        assert_eq!(filtered.len(), 2); // txt and jpg files under 1MB
        assert!(filtered.iter().any(|f| f.name == "test.txt"));
        assert!(filtered.iter().any(|f| f.name == "image.jpg"));
    }
}
