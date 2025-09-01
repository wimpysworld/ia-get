//! Command-line interface module for ia-get
//!
//! Contains the CLI structure and argument parsing logic.

use clap::{Parser, Subcommand};

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

    /// Exclude files by extension (e.g., --exclude-ext xml,log)
    #[arg(long, value_name = "EXTENSIONS")]
    pub exclude_ext: Option<String>,

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
            exclude_ext: None,
            max_file_size: None,
            resume: false,
            compress: false,
            decompress: false,
            decompress_formats: vec![],
            command: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn test_cli_validation() {
        let mut cli = Cli::default();

        // Valid configuration
        assert!(cli.validate().is_ok());

        // Invalid concurrent downloads
        cli.concurrent_downloads = 0;
        assert!(cli.validate().is_err());

        cli.concurrent_downloads = 15;
        assert!(cli.validate().is_err());

        // Invalid max retries
        cli.concurrent_downloads = 3;
        cli.max_retries = 25;
        assert!(cli.validate().is_err());
    }

    #[test]
    fn test_extension_parsing() {
        let cli = Cli {
            include_ext: Some("pdf,txt, mp3 ".to_string()),
            exclude_ext: Some("XML,Log".to_string()),
            ..Default::default()
        };

        let include = cli.include_extensions();
        assert_eq!(include, vec!["pdf", "txt", "mp3"]);

        let exclude = cli.exclude_extensions();
        assert_eq!(exclude, vec!["xml", "log"]);
    }

    #[test]
    fn test_interactive_mode_detection() {
        let cli = Cli::default();
        assert!(cli.is_interactive_mode());

        let cli = Cli {
            url: Some("https://archive.org/details/test".to_string()),
            ..Default::default()
        };
        assert!(!cli.is_interactive_mode());
    }

    #[test]
    fn test_cli_parsing() {
        let args = vec![
            "ia-get",
            "https://archive.org/details/test",
            "--verbose",
            "--concurrent-downloads",
            "5",
            "--include-ext",
            "pdf,txt",
            "--compress",
        ];

        let cli = Cli::try_parse_from(args).unwrap();
        assert_eq!(
            cli.url,
            Some("https://archive.org/details/test".to_string())
        );
        assert!(cli.verbose);
        assert_eq!(cli.concurrent_downloads, 5);
        assert_eq!(cli.include_ext, Some("pdf,txt".to_string()));
        assert!(cli.compress);
    }
}
