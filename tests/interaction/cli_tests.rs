//! CLI Interaction Layer Tests
//!
//! Tests for command-line interface functionality including
//! CLI structure validation and SourceType functionality.

use ia_get::interface::cli::{Cli, SourceType};

/// Test basic CLI structure creation and validation
#[test]
fn test_cli_creation_and_validation() {
    let mut cli = Cli::default();

    // Test default values
    assert!(!cli.verbose);
    assert!(!cli.dry_run);

    // Test validation with valid values
    cli.concurrent_downloads = 3;
    assert!(cli.validate().is_ok());

    // Test validation with invalid values
    cli.concurrent_downloads = 0;
    assert!(cli.validate().is_err());
    assert!(cli.validate().unwrap_err().contains("between 1 and 10"));

    cli.concurrent_downloads = 15;
    assert!(cli.validate().is_err());
    assert!(cli.validate().unwrap_err().contains("between 1 and 10"));

    // Test valid boundary values
    cli.concurrent_downloads = 1;
    assert!(cli.validate().is_ok());

    cli.concurrent_downloads = 10;
    assert!(cli.validate().is_ok());
}

/// Test SourceType enum functionality
#[test]
fn test_source_type_functionality() {
    let original = SourceType::Original;
    let derivative = SourceType::Derivative;
    let metadata = SourceType::Metadata;

    // Test as_str conversion
    assert_eq!(original.as_str(), "original");
    assert_eq!(derivative.as_str(), "derivative");
    assert_eq!(metadata.as_str(), "metadata");

    // Test matches function
    assert!(original.matches("original"));
    assert!(!original.matches("derivative"));
    assert!(derivative.matches("derivative"));
    assert!(!derivative.matches("metadata"));
    assert!(metadata.matches("metadata"));
    assert!(!metadata.matches("original"));
}

/// Test CLI field setting and getting
#[test]
fn test_cli_field_operations() {
    let cli = Cli {
        url: Some("https://archive.org/details/test".to_string()),
        verbose: true,
        dry_run: true,
        concurrent_downloads: 5,
        include_ext: Some("pdf,txt".to_string()),
        ..Default::default()
    };

    // Verify fields are set correctly
    assert_eq!(
        cli.url,
        Some("https://archive.org/details/test".to_string())
    );
    assert!(cli.verbose);
    assert!(cli.dry_run);
    assert_eq!(cli.concurrent_downloads, 5);
    assert_eq!(cli.include_ext, Some("pdf,txt".to_string()));
}

/// Test CLI max retries validation
#[test]
fn test_cli_validation_max_retries() {
    let mut cli = Cli {
        concurrent_downloads: 3,
        ..Default::default()
    };

    // Valid max retries
    cli.max_retries = 3;
    assert!(cli.validate().is_ok());

    cli.max_retries = 20;
    assert!(cli.validate().is_ok());

    // Invalid: too high
    cli.max_retries = 25;
    assert!(cli.validate().is_err());
    assert!(cli.validate().unwrap_err().contains("cannot exceed 20"));
}

/// Test CLI additional field functionality
#[test]
fn test_cli_additional_fields() {
    let mut cli = Cli::default();

    // Test default values for additional fields
    assert!(!cli.log_hash_errors);
    assert!(!cli.resume);
    assert!(!cli.compress);
    assert!(!cli.decompress);
    assert!(!cli.original_only);
    assert!(!cli.include_derivatives);
    assert!(!cli.include_metadata);

    // Test setting fields
    cli.log_hash_errors = true;
    cli.resume = true;
    cli.compress = true;
    cli.decompress = true;
    cli.original_only = true;
    cli.include_derivatives = true;
    cli.include_metadata = true;

    assert!(cli.log_hash_errors);
    assert!(cli.resume);
    assert!(cli.compress);
    assert!(cli.decompress);
    assert!(cli.original_only);
    assert!(cli.include_derivatives);
    assert!(cli.include_metadata);
}

/// Test SourceType vector operations
#[test]
fn test_source_type_collections() {
    let source_types = [
        SourceType::Original,
        SourceType::Derivative,
        SourceType::Metadata,
    ];

    // Test that we can create collections of source types
    assert_eq!(source_types.len(), 3);
    assert!(source_types.contains(&SourceType::Original));
    assert!(source_types.contains(&SourceType::Derivative));
    assert!(source_types.contains(&SourceType::Metadata));
}
