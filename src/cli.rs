//! Command-line interface module for ia-get
//!
//! Contains the CLI structure and argument parsing logic.

use clap::{Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};

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

/// Command options for the new CLI
#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Download files from archive.org
    Download {
        /// URL to archive.org details page
        url: String,

        /// Output directory
        #[arg(short, long)]
        output: Option<String>,

        /// Include only files with these extensions (comma-separated)
        #[arg(long)]
        include_ext: Option<String>,

        /// Exclude files with these extensions (comma-separated)
        #[arg(long)]
        exclude_ext: Option<String>,

        /// Maximum file size to download
        #[arg(long)]
        max_file_size: Option<String>,

        /// Enable compression for downloads
        #[arg(long)]
        compress: bool,
    },

    /// List available file format categories and examples
    ListFormats {
        /// Show detailed information including all file extensions in each category
        #[arg(short, long)]
        detailed: bool,

        /// Show only specific categories (comma-separated)
        #[arg(short, long, value_delimiter = ',')]
        categories: Vec<String>,
    },
}

/// Command-line interface for ia-get
#[derive(Parser)]
#[command(name = "ia-get")]
#[command(about = "A command-line tool for downloading files from the Internet Archive")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(author = env!("CARGO_PKG_AUTHORS"))]
pub struct Cli {
    /// URL to an archive.org details page or identifier (optional for interactive mode)
    #[arg(value_name = "URL")]
    pub url: Option<String>,

    /// Output directory for downloaded files (default: identifier from URL)
    #[arg(short, long, value_name = "DIR")]
    pub output_path: Option<String>,

    /// Enable logging of files with failed hash verification to a file named 'Hash errors'
    #[arg(short = 'L', long = "Log")]
    pub log_hash_errors: bool,

    /// Enable verbose output for debugging
    #[arg(short, long)]
    pub verbose: bool,

    /// Show what would be downloaded without actually downloading
    #[arg(long)]
    pub dry_run: bool,

    /// Set maximum concurrent downloads (default: 3, max: 10)
    #[arg(short = 'j', long, value_name = "NUM", default_value = "3")]
    pub concurrent_downloads: usize,

    /// Set download retry attempts (default: 3)
    #[arg(long, value_name = "NUM", default_value = "3")]
    pub max_retries: usize,

    /// Filter files by extension (e.g., --include-ext pdf,txt,mp3)
    #[arg(long, value_name = "EXTENSIONS")]
    pub include_ext: Option<String>,

    /// Include files by format category (e.g., --include-formats documents,images)
    #[arg(long, value_delimiter = ',')]
    pub include_formats: Vec<String>,

    /// Exclude files by extension (e.g., --exclude-ext xml,log)
    #[arg(long, value_name = "EXTENSIONS")]
    pub exclude_ext: Option<String>,

    /// Exclude files by format category (e.g., --exclude-formats metadata,archives)
    #[arg(long, value_delimiter = ',')]
    pub exclude_formats: Vec<String>,

    /// Skip files larger than specified size (e.g., 100MB, 1GB)
    #[arg(long, value_name = "SIZE")]
    pub max_file_size: Option<String>,

    /// Resume downloads from previous session
    #[arg(long)]
    pub resume: bool,

    /// Enable HTTP compression during downloads
    #[arg(long)]
    pub compress: bool,

    /// Automatically decompress downloaded files
    #[arg(long)]
    pub decompress: bool,

    /// Comma-separated list of formats to auto-decompress (e.g., gzip,bzip2,xz)
    #[arg(long, value_delimiter = ',')]
    pub decompress_formats: Vec<String>,

    /// File source types to download (default: original only)
    #[arg(long, value_enum, value_delimiter = ',', default_values_t = vec![SourceType::Original])]
    pub source_types: Vec<SourceType>,

    /// Download only original files (shorthand for --source-types original)
    #[arg(long, conflicts_with = "source_types")]
    pub original_only: bool,

    /// Include derivative files in addition to originals
    #[arg(long, conflicts_with = "source_types")]
    pub include_derivatives: bool,

    /// Include metadata files in addition to originals  
    #[arg(long, conflicts_with = "source_types")]
    pub include_metadata: bool,

    /// Subcommands
    #[command(subcommand)]
    pub command: Option<Commands>,
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
        // Handle convenience flags
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

        // If explicit source_types were provided, use those instead
        if !self.source_types.is_empty() && self.source_types != vec![SourceType::Original] {
            return self.source_types.clone();
        }

        types
    }

    /// Check if a file source should be downloaded based on CLI arguments
    pub fn should_download_source(&self, source: &str) -> bool {
        let allowed_types = self.get_source_types();
        allowed_types.iter().any(|t| t.matches(source))
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
            .and_then(|size| crate::filters::parse_size_string(size).ok())
    }

    /// Get all extensions to include based on both --include-ext and --include-formats  
    pub fn get_include_extensions(&self) -> Vec<String> {
        let mut extensions = self.include_extensions();

        // Add extensions from format categories
        if !self.include_formats.is_empty() {
            use crate::file_formats::{FileFormats, FormatCategory};
            let file_formats = FileFormats::new();

            for format_name in &self.include_formats {
                let format_name_lower = format_name.to_lowercase();

                // Try to match category name
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

        // Add extensions from format categories
        if !self.exclude_formats.is_empty() {
            use crate::file_formats::{FileFormats, FormatCategory};
            let file_formats = FileFormats::new();

            for format_name in &self.exclude_formats {
                let format_name_lower = format_name.to_lowercase();

                // Try to match category name
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

    /// Check if interactive mode should be enabled
    pub fn is_interactive_mode(&self) -> bool {
        self.url.is_none() && self.command.is_none()
    }

    /// Get the URL from either direct argument or subcommand
    pub fn get_url(&self) -> Option<&str> {
        if let Some(ref url) = self.url {
            Some(url)
        } else if let Some(Commands::Download { url, .. }) = &self.command {
            Some(url)
        } else {
            None
        }
    }

    /// Get output directory with fallback logic
    pub fn get_output_dir(&self) -> Option<&str> {
        if let Some(ref output) = self.output_path {
            Some(output)
        } else if let Some(Commands::Download {
            output: Some(ref output),
            ..
        }) = &self.command
        {
            Some(output)
        } else {
            None
        }
    }
}

impl Default for Cli {
    fn default() -> Self {
        Self {
            url: None,
            output_path: None,
            log_hash_errors: false,
            verbose: false,
            dry_run: false,
            concurrent_downloads: 3,
            max_retries: 3,
            include_ext: None,
            include_formats: vec![],
            exclude_ext: None,
            exclude_formats: vec![],
            max_file_size: None,
            resume: false,
            compress: false,
            decompress: false,
            decompress_formats: vec![],
            source_types: vec![SourceType::Original],
            original_only: false,
            include_derivatives: false,
            include_metadata: false,
            command: None,
        }
    }
}
