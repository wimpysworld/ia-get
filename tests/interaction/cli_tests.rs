//! CLI Interaction Layer Tests
//!
//! Comprehensive tests for command-line interface functionality including
//! argument parsing validation, CLI option combinations, interactive mode
//! detection, help text and error messages, and configuration validation.

use clap::Parser;
use ia_get::cli::{Cli, Commands, SourceType};

/// Test basic CLI parsing with URL arguments
#[test]
fn test_cli_basic_url_parsing() {
    let cli = Cli::parse_from(["ia-get", "https://archive.org/details/test"]);
    assert_eq!(
        cli.url,
        Some("https://archive.org/details/test".to_string())
    );
    assert!(cli.command.is_none());
}

/// Test CLI parsing with download subcommand
#[test]
fn test_cli_download_subcommand() {
    let cli = Cli::parse_from(["ia-get", "download", "https://archive.org/details/test"]);
    match cli.command {
        Some(Commands::Download { url, .. }) => {
            assert_eq!(url, "https://archive.org/details/test");
        }
        _ => panic!("Expected Download command"),
    }
}

/// Test CLI parsing with various flags and options
#[test]
fn test_cli_flags_parsing() {
    let cli = Cli::parse_from([
        "ia-get",
        "--verbose",
        "--dry-run",
        "--concurrent-downloads",
        "5",
        "--include-ext",
        "pdf,txt",
        "https://archive.org/details/test",
    ]);

    assert!(cli.verbose);
    assert!(cli.dry_run);
    assert_eq!(cli.concurrent_downloads, 5);
    assert_eq!(cli.include_ext, Some("pdf,txt".to_string()));
    assert_eq!(
        cli.url,
        Some("https://archive.org/details/test".to_string())
    );
}

/// Test CLI validation logic for concurrent downloads
#[test]
fn test_cli_validation_concurrent_downloads() {
    let mut cli = Cli::parse_from(["ia-get"]);

    // Valid configuration
    cli.concurrent_downloads = 3;
    assert!(cli.validate().is_ok());

    // Invalid: too low
    cli.concurrent_downloads = 0;
    assert!(cli.validate().is_err());
    assert!(cli.validate().unwrap_err().contains("between 1 and 10"));

    // Invalid: too high
    cli.concurrent_downloads = 15;
    assert!(cli.validate().is_err());
    assert!(cli.validate().unwrap_err().contains("between 1 and 10"));

    // Valid: boundary values
    cli.concurrent_downloads = 1;
    assert!(cli.validate().is_ok());

    cli.concurrent_downloads = 10;
    assert!(cli.validate().is_ok());
}

/// Test CLI validation logic for max retries
#[test]
fn test_cli_validation_max_retries() {
    let mut cli = Cli::parse_from(["ia-get"]);

    // Valid configuration
    cli.max_retries = 3;
    assert!(cli.validate().is_ok());

    // Valid: boundary case
    cli.max_retries = 20;
    assert!(cli.validate().is_ok());

    // Invalid: too high
    cli.max_retries = 25;
    assert!(cli.validate().is_err());
    assert!(cli.validate().unwrap_err().contains("cannot exceed 20"));
}

/// Test extension parsing functionality
#[test]
fn test_extension_parsing() {
    let cli = Cli::parse_from([
        "ia-get",
        "--include-ext",
        "pdf,txt, mp3 ",
        "--exclude-ext",
        "XML,Log",
    ]);

    let include = cli.include_extensions();
    assert_eq!(include, vec!["pdf", "txt", "mp3"]);

    let exclude = cli.exclude_extensions();
    assert_eq!(exclude, vec!["xml", "log"]);
}

/// Test source type parsing and resolution
#[test]
fn test_source_type_parsing() {
    // Test default behavior
    let cli = Cli::parse_from(["ia-get"]);
    let source_types = cli.get_source_types();
    assert_eq!(source_types, vec![SourceType::Original]);

    // Test explicit source types
    let cli = Cli::parse_from(["ia-get", "--source-types", "original,derivative"]);
    let source_types = cli.get_source_types();
    assert_eq!(
        source_types,
        vec![SourceType::Original, SourceType::Derivative]
    );

    // Test convenience flags
    let cli = Cli::parse_from(["ia-get", "--include-derivatives"]);
    let source_types = cli.get_source_types();
    assert!(source_types.contains(&SourceType::Original));
    assert!(source_types.contains(&SourceType::Derivative));

    let cli = Cli::parse_from(["ia-get", "--include-metadata"]);
    let source_types = cli.get_source_types();
    assert!(source_types.contains(&SourceType::Original));
    assert!(source_types.contains(&SourceType::Metadata));

    // Test original-only flag
    let cli = Cli::parse_from(["ia-get", "--original-only"]);
    let source_types = cli.get_source_types();
    assert_eq!(source_types, vec![SourceType::Original]);
}

/// Test source filtering logic
#[test]
fn test_source_filtering_logic() {
    // Test with default (original only)
    let cli = Cli::parse_from(["ia-get"]);
    assert!(cli.should_download_source("original"));
    assert!(!cli.should_download_source("derivative"));
    assert!(!cli.should_download_source("metadata"));

    // Test with all types
    let cli = Cli::parse_from(["ia-get", "--source-types", "original,derivative,metadata"]);
    assert!(cli.should_download_source("original"));
    assert!(cli.should_download_source("derivative"));
    assert!(cli.should_download_source("metadata"));

    // Test with derivatives included
    let cli = Cli::parse_from(["ia-get", "--include-derivatives"]);
    assert!(cli.should_download_source("original"));
    assert!(cli.should_download_source("derivative"));
    assert!(!cli.should_download_source("metadata"));
}

/// Test SourceType enum functionality
#[test]
fn test_source_type_enum() {
    assert_eq!(SourceType::Original.as_str(), "original");
    assert_eq!(SourceType::Derivative.as_str(), "derivative");
    assert_eq!(SourceType::Metadata.as_str(), "metadata");

    assert!(SourceType::Original.matches("original"));
    assert!(!SourceType::Original.matches("derivative"));
    assert!(SourceType::Derivative.matches("derivative"));
    assert!(SourceType::Metadata.matches("metadata"));
}

/// Test CLI with compression options
#[test]
fn test_compression_options() {
    let cli = Cli::parse_from([
        "ia-get",
        "--compress",
        "--decompress",
        "--decompress-formats",
        "gzip,bzip2",
    ]);

    assert!(cli.compress);
    assert!(cli.decompress);
    assert_eq!(cli.decompress_formats, vec!["gzip", "bzip2"]);
}

/// Test CLI with output and resume options
#[test]
fn test_output_and_resume_options() {
    let cli = Cli::parse_from([
        "ia-get",
        "--output-path",
        "/tmp/downloads",
        "--resume",
        "--Log",
    ]);

    assert_eq!(cli.output_path, Some("/tmp/downloads".to_string()));
    assert!(cli.resume);
    assert!(cli.log_hash_errors);
}

/// Test CLI size filtering options
#[test]
fn test_size_filtering_options() {
    let cli = Cli::parse_from(["ia-get", "--max-file-size", "100MB"]);

    assert_eq!(cli.max_file_size, Some("100MB".to_string()));
}

/// Test CLI retry configuration
#[test]
fn test_retry_configuration() {
    let cli = Cli::parse_from(["ia-get", "--max-retries", "5"]);

    assert_eq!(cli.max_retries, 5);
}
