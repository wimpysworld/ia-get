//! Command-line interface module for ia-get
//!
//! Contains the CLI structure and argument parsing logic.

use clap::{Parser, Subcommand};

/// Legacy CLI structure for backward compatibility
#[derive(Debug)]
pub struct LegacyCli {
    pub url: Option<String>,
    pub output_path: Option<String>,
    pub log_hash_errors: bool,
    pub verbose: bool,
    pub dry_run: bool,
    pub concurrent_downloads: usize,
    pub max_retries: usize,
    pub include_ext: Option<String>,
    pub exclude_ext: Option<String>,
    pub max_file_size: Option<String>,
    pub resume: bool,
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
}

/// Command-line interface for ia-get
#[derive(Parser)]
#[command(name = "ia-get")]
#[command(about = "A command-line tool for downloading files from the Internet Archive")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(author = env!("CARGO_PKG_AUTHORS"))]
pub struct Cli {
    /// URL to an archive.org details page (optional for interactive mode)
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
