//! Command-line interface module for ia-get
//!
//! Contains the CLI structure and argument parsing logic.

pub mod commands;

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

// Re-export the command handlers for use in main.rs
pub use commands::{handle_config_command, handle_history_command};

// Export the action enums for main.rs to use
#[derive(Debug, Clone)]
pub enum ConfigAction {
    Show,
    Set { key: String, value: String },
    Unset { key: String },
    Location,
    Reset,
    Validate,
}

#[derive(Debug, Clone)]
pub enum HistoryAction {
    Show {
        limit: usize,
        status: Option<String>,
        detailed: bool,
    },
    Clear {
        force: bool,
    },
    Remove {
        id: String,
    },
    Stats,
}

/// File source types in Internet Archive
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
#[clap(rename_all = "lowercase")]
pub enum SourceType {
    /// Original uploaded files
    Original,
    /// Files derived from originals (e.g., different formats, thumbnails)
    Derivative,
    /// Metadata files (e.g., description files, database dumps)
    Metadata,
}

impl SourceType {
    /// Convert to string as used in IA metadata
    pub fn as_str(&self) -> &str {
        match self {
            SourceType::Original => "original",
            SourceType::Derivative => "derivative",
            SourceType::Metadata => "metadata",
        }
    }

    /// Check if a source string matches this type
    pub fn matches(&self, source: &str) -> bool {
        self.as_str() == source
    }
}

/// Simple CLI structure to maintain compatibility with existing code
/// The actual argument parsing is done in main.rs using clap Command builder
#[derive(Debug, Default)]
pub struct Cli {
    pub url: Option<String>,
    pub output_path: Option<String>,
    pub log_hash_errors: bool,
    pub verbose: bool,
    pub dry_run: bool,
    pub concurrent_downloads: usize,
    pub max_retries: usize,
    pub include_ext: Option<String>,
    pub include_formats: Vec<String>,
    pub exclude_ext: Option<String>,
    pub exclude_formats: Vec<String>,
    pub max_file_size: Option<String>,
    pub resume: bool,
    pub compress: bool,
    pub decompress: bool,
    pub decompress_formats: Vec<String>,
    pub source_types: Vec<SourceType>,
    pub original_only: bool,
    pub include_derivatives: bool,
    pub include_metadata: bool,
}

impl Cli {
    /// Validation and processing helper methods
    pub fn validate(&self) -> Result<(), String> {
        // Validate concurrent downloads range
        if self.concurrent_downloads == 0 || self.concurrent_downloads > 10 {
            return Err("Concurrent downloads must be between 1 and 10".to_string());
        }

        // Validate max retries
        if self.max_retries > 20 {
            return Err("Max retries cannot exceed 20".to_string());
        }

        Ok(())
    }

    /// Get the resolved source types based on CLI arguments
    pub fn get_source_types(&self) -> Vec<SourceType> {
        if self.original_only {
            return vec![SourceType::Original];
        }

        let mut types = vec![SourceType::Original]; // Always include originals by default

        if self.include_derivatives {
            types.push(SourceType::Derivative);
        }

        if self.include_metadata {
            types.push(SourceType::Metadata);
        }

        // If explicit source types are provided, use those instead
        if !self.source_types.is_empty() && self.source_types != vec![SourceType::Original] {
            return self.source_types.clone();
        }

        types
    }

    /// Check if a file source should be downloaded based on CLI arguments
    pub fn should_download_source(&self, source: &str) -> bool {
        let types = self.get_source_types();
        types.iter().any(|t| t.matches(source))
    }

    /// Get parsed extensions for inclusion filter
    pub fn include_extensions(&self) -> Vec<String> {
        self.include_ext
            .as_ref()
            .map(|ext| ext.split(',').map(|s| s.trim().to_lowercase()).collect())
            .unwrap_or_default()
    }

    /// Get parsed extensions for exclusion filter  
    pub fn exclude_extensions(&self) -> Vec<String> {
        self.exclude_ext
            .as_ref()
            .map(|ext| ext.split(',').map(|s| s.trim().to_lowercase()).collect())
            .unwrap_or_default()
    }

    /// Parse max file size into bytes
    pub fn max_file_size_bytes(&self) -> Option<u64> {
        self.max_file_size
            .as_ref()
            .and_then(|size| crate::utilities::filters::parse_size_string(size).ok())
    }

    /// Get all extensions to include based on both --include-ext and --include-formats  
    pub fn get_include_extensions(&self) -> Vec<String> {
        let mut extensions = self.include_extensions();

        // Add format category extensions
        if !self.include_formats.is_empty() {
            use crate::utilities::filters::{FileFormats, FormatCategory};
            let file_formats = FileFormats::new();

            for format_name in &self.include_formats {
                let format_name_lower = format_name.to_lowercase();
                for category in FormatCategory::all() {
                    if category.display_name().to_lowercase() == format_name_lower {
                        extensions.extend(file_formats.get_formats(&category));
                        break;
                    }
                }
            }
        }

        extensions.sort();
        extensions.dedup();
        extensions
    }

    /// Get all extensions to exclude based on both --exclude-ext and --exclude-formats
    pub fn get_exclude_extensions(&self) -> Vec<String> {
        let mut extensions = self.exclude_extensions();

        // Add format category extensions
        if !self.exclude_formats.is_empty() {
            use crate::utilities::filters::{FileFormats, FormatCategory};
            let file_formats = FileFormats::new();

            for format_name in &self.exclude_formats {
                let format_name_lower = format_name.to_lowercase();
                for category in FormatCategory::all() {
                    if category.display_name().to_lowercase() == format_name_lower {
                        extensions.extend(file_formats.get_formats(&category));
                        break;
                    }
                }
            }
        }

        extensions.sort();
        extensions.dedup();
        extensions
    }
}

/// Placeholder Commands enum for compatibility
#[derive(Debug)]
pub enum Commands {
    Download {
        include_ext: Option<String>,
        exclude_ext: Option<String>,
        max_file_size: Option<String>,
    },
    ListFormats {
        detailed: bool,
        categories: Vec<String>,
    },
}
