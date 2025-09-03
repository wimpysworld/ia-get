//! Edge Case Testing Module
//!
//! Comprehensive tests for edge cases and error conditions including:
//! - Boundary value testing
//! - Invalid input handling
//! - Resource exhaustion scenarios
//! - Malformed data processing
//! - Recovery mechanisms

use clap::Parser;
use ia_get::{
    cli::{Cli, Commands},
    error::IaGetError,
    filters::{filter_files, format_size, parse_size_string},
    metadata::{get_json_url, parse_archive_metadata},
    metadata_storage::{ArchiveFile, ArchiveMetadata, DownloadConfig, DownloadSession},
    url_processing::{extract_identifier_from_url, validate_and_process_url},
};

/// Test URL edge cases and malformed inputs
#[test]
fn test_url_edge_cases() {
    // Empty string
    let result = validate_and_process_url("");
    assert!(result.is_err());

    // Just whitespace
    let result = validate_and_process_url("   ");
    assert!(result.is_err());

    // Invalid characters
    let result = validate_and_process_url("https://archive.org/details/test item");
    assert!(result.is_err());

    // Missing protocol
    let result = validate_and_process_url("archive.org/details/test");
    assert!(result.is_err());

    // Wrong domain
    let result = validate_and_process_url("https://example.com/details/test");
    assert!(result.is_err());

    // Very long URL
    let long_identifier = "a".repeat(1000);
    let long_url = format!("https://archive.org/details/{}", long_identifier);
    let result = validate_and_process_url(&long_url);
    // Should still work if it's a valid archive.org URL
    assert!(result.is_ok());

    // URL with special characters in identifier
    let special_url = "https://archive.org/details/test-item_with.special-chars123";
    let result = validate_and_process_url(special_url);
    assert!(result.is_ok());

    // URL with query parameters
    let url_with_query = "https://archive.org/details/test?param=value&other=123";
    let result = validate_and_process_url(url_with_query);
    assert!(result.is_ok());

    // URL with fragment
    let url_with_fragment = "https://archive.org/details/test#section";
    let result = validate_and_process_url(url_with_fragment);
    assert!(result.is_ok());
}

/// Test identifier extraction edge cases
#[test]
fn test_identifier_extraction_edge_cases() {
    // Identifier with maximum length
    let max_length_id = "a".repeat(100);
    let url = format!("https://archive.org/details/{}", max_length_id);
    let result = extract_identifier_from_url(&url);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), max_length_id);

    // Identifier with minimum length (single character)
    let result = extract_identifier_from_url("https://archive.org/details/a");
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "a");

    // Identifier with only numbers
    let result = extract_identifier_from_url("https://archive.org/details/123456789");
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "123456789");

    // Identifier with mixed case
    let result = extract_identifier_from_url("https://archive.org/details/TeSt-CaSe_123");
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "TeSt-CaSe_123");

    // URLs with trailing slashes
    let result = extract_identifier_from_url("https://archive.org/details/test/");
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "test");

    // URLs with multiple trailing slashes
    let result = extract_identifier_from_url("https://archive.org/details/test///");
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "test");
}

/// Test size parsing boundary values
#[test]
fn test_size_parsing_boundaries() {
    // Zero size
    assert_eq!(parse_size_string("0").unwrap(), 0);
    assert_eq!(parse_size_string("0B").unwrap(), 0);
    assert_eq!(parse_size_string("0KB").unwrap(), 0);

    // Maximum u64 value
    let max_bytes = u64::MAX.to_string();
    let result = parse_size_string(&max_bytes);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), u64::MAX);

    // Very small decimal values
    assert_eq!(parse_size_string("0.000001KB").unwrap(), 0); // Rounds to 0
    assert_eq!(parse_size_string("0.001KB").unwrap(), 1); // Rounds to 1

    // Very large values that might overflow
    let result = parse_size_string("99999999999999999999TB");
    // Should handle overflow gracefully
    assert!(result.is_err() || result.unwrap() == u64::MAX);

    // Edge case: exactly 1KB boundary
    assert_eq!(parse_size_string("1024").unwrap(), 1024);
    assert_eq!(parse_size_string("1KB").unwrap(), 1024);
    assert_eq!(parse_size_string("1.0KB").unwrap(), 1024);

    // Just under 1KB
    assert_eq!(parse_size_string("1023").unwrap(), 1023);

    // Just over 1KB
    assert_eq!(parse_size_string("1025").unwrap(), 1025);
}

/// Test malformed JSON metadata handling
#[test]
fn test_malformed_metadata_handling() {
    // Completely invalid JSON
    let invalid_json = "this is not json at all";
    let result = parse_archive_metadata(invalid_json);
    assert!(result.is_err());

    // Valid JSON but wrong structure
    let wrong_structure = r#"{"not": "metadata", "format": "wrong"}"#;
    let result = parse_archive_metadata(wrong_structure);
    assert!(result.is_err());

    // JSON with missing required fields
    let missing_files = r#"{"server": "test", "dir": "/test"}"#;
    let result = parse_archive_metadata(missing_files);
    assert!(result.is_err());

    // JSON with wrong data types
    let wrong_types = r#"{"files": "should-be-array", "server": 123, "dir": true}"#;
    let result = parse_archive_metadata(wrong_types);
    assert!(result.is_err());

    // JSON with null values in required fields
    let null_values = r#"{"files": null, "server": null, "dir": null}"#;
    let result = parse_archive_metadata(null_values);
    assert!(result.is_err());

    // Truncated JSON
    let truncated = r#"{"files": [{"name": "test.txt", "size": "100"#;
    let result = parse_archive_metadata(truncated);
    assert!(result.is_err());

    // JSON with unexpected additional fields (should still work)
    let extra_fields = r#"{
        "files": [{"name": "test.txt"}],
        "server": "test",
        "dir": "/test",
        "unexpected_field": "should be ignored",
        "another_field": 12345
    }"#;
    let result = parse_archive_metadata(extra_fields);
    assert!(result.is_ok());
}

/// Test file filtering with edge cases
#[test]
fn test_file_filtering_edge_cases() {
    // Empty file list
    let empty_files = vec![];
    let result = filter_files(&empty_files, &["pdf".to_string()], &[], None);
    assert_eq!(result.len(), 0);

    // Files with no extensions
    let no_ext_files = vec![ArchiveFile {
        name: "file_without_extension".to_string(),
        source: None,
        mtime: None,
        size: Some("1000".to_string()),
        md5: None,
        crc32: None,
        sha1: None,
        format: None,
        width: None,
        height: None,
        length: None,
    }];

    // Should not match extension filters
    let result = filter_files(&no_ext_files, &["pdf".to_string()], &[], None);
    assert_eq!(result.len(), 0);

    // But should pass through with no extension filters
    let result = filter_files(&no_ext_files, &[], &[], None);
    assert_eq!(result.len(), 1);

    // Files with multiple dots in name
    let multi_dot_files = vec![ArchiveFile {
        name: "file.with.many.dots.txt".to_string(),
        source: None,
        mtime: None,
        size: Some("1000".to_string()),
        md5: None,
        crc32: None,
        sha1: None,
        format: None,
        width: None,
        height: None,
        length: None,
    }];

    // Should match based on final extension
    let result = filter_files(&multi_dot_files, &["txt".to_string()], &[], None);
    assert_eq!(result.len(), 1);

    // Files with unusual sizes
    let unusual_size_files = vec![
        ArchiveFile {
            name: "zero_size.txt".to_string(),
            source: None,
            mtime: None,
            size: Some("0".to_string()),
            md5: None,
            crc32: None,
            sha1: None,
            format: None,
            width: None,
            height: None,
            length: None,
        },
        ArchiveFile {
            name: "no_size.txt".to_string(),
            source: None,
            mtime: None,
            size: None,
            md5: None,
            crc32: None,
            sha1: None,
            format: None,
            width: None,
            height: None,
            length: None,
        },
        ArchiveFile {
            name: "invalid_size.txt".to_string(),
            source: None,
            mtime: None,
            size: Some("not-a-number".to_string()),
            md5: None,
            crc32: None,
            sha1: None,
            format: None,
            width: None,
            height: None,
            length: None,
        },
    ];

    // Size filter should handle these gracefully
    let result = filter_files(&unusual_size_files, &[], &[], Some(100));
    // Files with no size or invalid size should pass through
    assert_eq!(result.len(), 3); // zero_size, no_size, and invalid_size all pass
}

/// Test CLI parsing edge cases
#[test]
fn test_cli_parsing_edge_cases() {
    // Very long arguments
    let long_url = format!("https://archive.org/details/{}", "a".repeat(500));
    let long_output = "b".repeat(200);

    let cli = Cli::parse_from(["ia-get", "--output", &long_output, &long_url]);

    assert_eq!(cli.url, Some(long_url));
    assert_eq!(cli.output_path, Some(long_output));

    // Empty string arguments
    let cli = Cli::parse_from(["ia-get", ""]);
    assert_eq!(cli.url, Some("".to_string()));

    // Maximum and minimum values for numeric arguments
    let cli = Cli::parse_from([
        "ia-get",
        "--concurrent-downloads",
        "1",
        "--max-retries",
        "0",
        "test",
    ]);
    assert_eq!(cli.concurrent_downloads, 1);
    assert_eq!(cli.max_retries, 0);

    let cli = Cli::parse_from([
        "ia-get",
        "--concurrent-downloads",
        "10",
        "--max-retries",
        "20",
        "test",
    ]);
    assert_eq!(cli.concurrent_downloads, 10);
    assert_eq!(cli.max_retries, 20);

    // Extension lists with various formats
    let cli = Cli::parse_from([
        "ia-get",
        "--include-ext",
        "",
        "--exclude-ext",
        "a,b,c,d,e,f,g,h,i,j,k,l,m,n,o,p",
        "test",
    ]);

    let include = cli.include_extensions();
    assert_eq!(include, vec![""]); // Empty string becomes single empty element

    let exclude = cli.exclude_extensions();
    assert_eq!(exclude.len(), 16); // Long list should be parsed correctly
}

/// Test session creation with unusual data
#[test]
fn test_session_edge_cases() {
    // Archive with no files
    let empty_metadata = ArchiveMetadata {
        created: None,
        d1: None,
        dir: "/empty".to_string(),
        files: vec![],
        metadata: None,
        misc: None,
        reviews: None,
        server: "test-server".to_string(),
        uniq: None,
        workable_servers: None,
    };

    let config = DownloadConfig {
        output_dir: "test".to_string(),
        concurrent_downloads: 1,
        max_retries: 1,
        include_extensions: vec![],
        exclude_extensions: vec![],
        max_file_size: None,
        resume: false,
        compress: false,
        decompress: false,
        decompress_formats: vec![],
        dry_run: false,
        verbose: false,
        log_hash_errors: false,
    };

    // Session with no requested files
    let session = DownloadSession::new(
        "https://archive.org/details/empty".to_string(),
        "empty".to_string(),
        empty_metadata,
        config.clone(),
        vec![],
    );

    assert_eq!(session.file_status.len(), 0);
    assert_eq!(session.get_pending_files().len(), 0);

    let progress = session.get_progress_summary();
    assert_eq!(progress.total_files, 0);
    assert_eq!(progress.total_bytes, 0);

    // Session with files that don't exist in metadata
    let metadata_with_files = ArchiveMetadata {
        created: None,
        d1: None,
        dir: "/test".to_string(),
        files: vec![ArchiveFile {
            name: "existing.txt".to_string(),
            source: None,
            mtime: None,
            size: None,
            md5: None,
            crc32: None,
            sha1: None,
            format: None,
            width: None,
            height: None,
            length: None,
        }],
        metadata: None,
        misc: None,
        reviews: None,
        server: "test-server".to_string(),
        uniq: None,
        workable_servers: None,
    };

    let session = DownloadSession::new(
        "https://archive.org/details/test".to_string(),
        "test".to_string(),
        metadata_with_files,
        config,
        vec!["existing.txt".to_string(), "nonexistent.txt".to_string()],
    );

    // Should only create status for files that exist in metadata
    assert_eq!(session.file_status.len(), 1);
    assert!(session.file_status.contains_key("existing.txt"));
    assert!(!session.file_status.contains_key("nonexistent.txt"));
}

/// Test format_size with edge cases
#[test]
fn test_format_size_edge_cases() {
    // Zero
    assert_eq!(format_size(0), "0 B");

    // Very small values
    assert_eq!(format_size(1), "1 B");
    assert_eq!(format_size(512), "512 B");

    // Boundary values
    assert_eq!(format_size(1023), "1023 B");
    assert_eq!(format_size(1024), "1.0 KB");
    assert_eq!(format_size(1025), "1.0 KB"); // Rounds down

    // Large values
    assert_eq!(format_size(u64::MAX), "16777216.0 TB");

    // Values that result in exact divisions
    assert_eq!(format_size(2048), "2.0 KB");
    assert_eq!(format_size(3072), "3.0 KB");
    assert_eq!(format_size(1048576), "1.0 MB");
    assert_eq!(format_size(1073741824), "1.0 GB");

    // Values that result in decimals
    assert_eq!(format_size(1536), "1.5 KB"); // 1.5 * 1024
    assert_eq!(format_size(1572864), "1.5 MB"); // 1.5 * 1048576
}

/// Test error propagation and handling
#[test]
fn test_error_propagation() {
    // Test that errors from different layers propagate correctly

    // URL validation error
    let url_error = validate_and_process_url("invalid-url");
    assert!(url_error.is_err());

    if let Err(error) = url_error {
        match error {
            IaGetError::Parse(_) => {
                // Expected error type for URL validation
            }
            _ => panic!("Unexpected error type for URL validation"),
        }
    }

    // Metadata parsing error
    let parse_error = parse_archive_metadata("invalid json");
    assert!(parse_error.is_err());

    if let Err(error) = parse_error {
        match error {
            IaGetError::Parse(_) => {
                // Expected error type for JSON parsing
            }
            _ => panic!("Unexpected error type for JSON parsing"),
        }
    }

    // Size parsing error
    let size_error = parse_size_string("invalid-size");
    assert!(size_error.is_err());

    if let Err(error) = size_error {
        match error {
            IaGetError::Parse(_) => {
                // Expected error type for size parsing
            }
            _ => panic!("Unexpected error type for size parsing"),
        }
    }
}

/// Test concurrent access patterns
#[test]
fn test_concurrent_edge_cases() {
    use std::sync::{Arc, Mutex};
    use std::thread;

    // Test multiple threads accessing the same data structures
    let cli = Arc::new(Cli::parse_from([
        "ia-get",
        "--concurrent-downloads",
        "5",
        "https://archive.org/details/test",
    ]));

    let handles: Vec<_> = (0..10)
        .map(|_| {
            let cli_clone = Arc::clone(&cli);
            thread::spawn(move || {
                // Each thread validates the same CLI data
                let url = cli_clone.get_url().unwrap();
                let result = validate_and_process_url(url);
                result.is_ok()
            })
        })
        .collect();

    // All threads should succeed
    for handle in handles {
        assert!(handle.join().unwrap());
    }

    // Test concurrent filter operations
    let files = Arc::new(vec![ArchiveFile {
        name: format!("file{}.txt", 1),
        source: None,
        mtime: None,
        size: Some("1000".to_string()),
        md5: None,
        crc32: None,
        sha1: None,
        format: None,
        width: None,
        height: None,
        length: None,
    }]);

    let filter_handles: Vec<_> = (0..5)
        .map(|i| {
            let files_clone = Arc::clone(&files);
            thread::spawn(move || {
                let ext = format!("ext{}", i);
                filter_files(&files_clone, &[ext], &[], None).len()
            })
        })
        .collect();

    // All filter operations should complete without panicking
    for handle in filter_handles {
        let _result = handle.join().unwrap();
    }
}

/// Test memory usage with large datasets
#[test]
fn test_large_dataset_handling() {
    // Create a large metadata structure
    let mut large_files = Vec::new();
    for i in 0..1000 {
        large_files.push(ArchiveFile {
            name: format!("file_{:04}.txt", i),
            source: Some("original".to_string()),
            mtime: Some("1234567890".to_string()),
            size: Some(format!("{}", i * 1000)),
            md5: Some(format!("hash_{}", i)),
            crc32: None,
            sha1: None,
            format: Some("Text".to_string()),
            width: None,
            height: None,
            length: None,
        });
    }

    let large_metadata = ArchiveMetadata {
        created: Some(1234567890),
        d1: Some("archive.org".to_string()),
        dir: "/large-test".to_string(),
        files: large_files,
        metadata: None,
        misc: None,
        reviews: None,
        server: "test-server".to_string(),
        uniq: Some(1234567890),
        workable_servers: Some(vec!["test-server".to_string()]),
    };

    // Test filtering large dataset
    let start = std::time::Instant::now();
    let filtered = filter_files(
        &large_metadata.files,
        &["txt".to_string()],
        &[],
        Some(500000),
    );
    let duration = start.elapsed();

    // Should filter correctly
    assert_eq!(filtered.len(), 500); // Files 0-499 under 500KB

    // Should complete in reasonable time (under 1 second)
    assert!(duration.as_millis() < 1000);

    // Test session creation with large dataset
    let config = DownloadConfig {
        output_dir: "large-test".to_string(),
        concurrent_downloads: 5,
        max_retries: 3,
        include_extensions: vec![],
        exclude_extensions: vec![],
        max_file_size: None,
        resume: false,
        compress: false,
        decompress: false,
        decompress_formats: vec![],
        dry_run: false,
        verbose: false,
        log_hash_errors: false,
    };

    let requested_files: Vec<String> = (0..100).map(|i| format!("file_{:04}.txt", i)).collect();

    let start = std::time::Instant::now();
    let session = DownloadSession::new(
        "https://archive.org/details/large-test".to_string(),
        "large-test".to_string(),
        large_metadata,
        config,
        requested_files,
    );
    let duration = start.elapsed();

    // Should create session correctly
    assert_eq!(session.file_status.len(), 100);

    // Should complete in reasonable time
    assert!(duration.as_millis() < 1000);
}
