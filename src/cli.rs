//! Command-line interface module for ia-get
//!
//! Contains the CLI structure and argument parsing logic.

use clap::Parser;

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
}
