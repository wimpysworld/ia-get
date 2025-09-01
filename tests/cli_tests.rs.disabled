//! CLI Testing Module
//!
//! Comprehensive tests for command-line interface functionality including:
//! - Argument parsing validation
//! - CLI option combinations
//! - Interactive mode detection
//! - Help text and error messages
//! - Configuration validation

use clap::Parser;
use ia_get::cli::{Cli, Commands};

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
    let mut cli = Cli::default();

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
    let mut cli = Cli::default();

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

/// Test interactive mode detection
#[test]
fn test_interactive_mode_detection() {
    // No URL and no command = interactive mode
    let cli = Cli::default();
    assert!(cli.is_interactive_mode());

    // URL provided = not interactive
    let cli = Cli {
        url: Some("https://archive.org/details/test".to_string()),
        ..Default::default()
    };
    assert!(!cli.is_interactive_mode());

    // Command provided = not interactive
    let cli = Cli {
        command: Some(Commands::Download {
            url: "test".to_string(),
            output: None,
            include_ext: None,
            exclude_ext: None,
            max_file_size: None,
            compress: false,
        }),
        ..Default::default()
    };
    assert!(!cli.is_interactive_mode());
}

/// Test URL extraction from different sources
#[test]
fn test_url_extraction() {
    // From direct URL argument
    let cli = Cli {
        url: Some("https://archive.org/details/test".to_string()),
        ..Default::default()
    };
    assert_eq!(cli.get_url(), Some("https://archive.org/details/test"));

    // From download subcommand
    let cli = Cli {
        command: Some(Commands::Download {
            url: "test-identifier".to_string(),
            output: None,
            include_ext: None,
            exclude_ext: None,
            max_file_size: None,
            compress: false,
        }),
        ..Default::default()
    };
    assert_eq!(cli.get_url(), Some("test-identifier"));

    // URL in direct argument takes precedence
    let cli = Cli {
        url: Some("direct-url".to_string()),
        command: Some(Commands::Download {
            url: "subcommand-url".to_string(),
            output: None,
            include_ext: None,
            exclude_ext: None,
            max_file_size: None,
            compress: false,
        }),
        ..Default::default()
    };
    assert_eq!(cli.get_url(), Some("direct-url"));

    // No URL provided
    let cli = Cli::default();
    assert_eq!(cli.get_url(), None);
}

/// Test output directory extraction
#[test]
fn test_output_directory_extraction() {
    // From direct output argument
    let cli = Cli {
        output_path: Some("output-dir".to_string()),
        ..Default::default()
    };
    assert_eq!(cli.get_output_dir(), Some("output-dir"));

    // From download subcommand
    let cli = Cli {
        command: Some(Commands::Download {
            url: "test".to_string(),
            output: Some("subcommand-output".to_string()),
            include_ext: None,
            exclude_ext: None,
            max_file_size: None,
            compress: false,
        }),
        ..Default::default()
    };
    assert_eq!(cli.get_output_dir(), Some("subcommand-output"));

    // Direct argument takes precedence
    let cli = Cli {
        output_path: Some("direct-output".to_string()),
        command: Some(Commands::Download {
            url: "test".to_string(),
            output: Some("subcommand-output".to_string()),
            include_ext: None,
            exclude_ext: None,
            max_file_size: None,
            compress: false,
        }),
        ..Default::default()
    };
    assert_eq!(cli.get_output_dir(), Some("direct-output"));

    // No output provided
    let cli = Cli::default();
    assert_eq!(cli.get_output_dir(), None);
}

/// Test file size parsing functionality
#[test]
fn test_max_file_size_parsing() {
    use ia_get::filters::parse_size_string;

    let cli = Cli {
        max_file_size: Some("100MB".to_string()),
        ..Default::default()
    };

    let size = cli.max_file_size_bytes();
    assert_eq!(size, parse_size_string("100MB").ok());

    // Test with invalid size
    let cli = Cli {
        max_file_size: Some("invalid".to_string()),
        ..Default::default()
    };
    assert_eq!(cli.max_file_size_bytes(), None);

    // Test with no size
    let cli = Cli::default();
    assert_eq!(cli.max_file_size_bytes(), None);
}

/// Test CLI parsing edge cases and error handling
#[test]
fn test_cli_parsing_edge_cases() {
    // Test with empty strings
    let cli = Cli::parse_from(["ia-get", ""]);
    assert_eq!(cli.url, Some("".to_string()));

    // Test with special characters in URLs
    let url_with_special = "https://archive.org/details/test?special=chars&more=params";
    let cli = Cli::parse_from(["ia-get", url_with_special]);
    assert_eq!(cli.url, Some(url_with_special.to_string()));
}

/// Test default CLI configuration
#[test]
fn test_cli_defaults() {
    let cli = Cli::default();

    assert_eq!(cli.url, None);
    assert_eq!(cli.output_path, None);
    assert!(!cli.log_hash_errors);
    assert!(!cli.verbose);
    assert!(!cli.dry_run);
    assert_eq!(cli.concurrent_downloads, 3);
    assert_eq!(cli.max_retries, 3);
    assert_eq!(cli.include_ext, None);
    assert_eq!(cli.exclude_ext, None);
    assert_eq!(cli.max_file_size, None);
    assert!(!cli.resume);
    assert!(!cli.compress);
    assert!(!cli.decompress);
    assert!(cli.decompress_formats.is_empty());
    assert!(cli.command.is_none());
}

/// Test CLI help text accessibility (ensure help can be generated)
#[test]
fn test_cli_help_generation() {
    let result = Cli::try_parse_from(["ia-get", "--help"]);
    assert!(result.is_err()); // --help causes clap to exit early with help text

    // Verify that the error is due to help display, not parsing failure
    if let Err(err) = result {
        assert_eq!(err.kind(), clap::error::ErrorKind::DisplayHelp);
    }
}

/// Test CLI version information accessibility
#[test]
fn test_cli_version_generation() {
    let result = Cli::try_parse_from(["ia-get", "--version"]);
    assert!(result.is_err()); // --version causes clap to exit early

    if let Err(err) = result {
        assert_eq!(err.kind(), clap::error::ErrorKind::DisplayVersion);
    }
}
