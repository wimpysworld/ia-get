//! Basic Metadata Tests
//!
//! Simple tests for metadata functionality without complex validation

use ia_get::{
    metadata::{get_json_url, parse_archive_metadata},
    url_processing::{construct_metadata_url, extract_identifier_from_url},
    DownloadRequest, DownloadResult, DownloadService,
};
use std::path::PathBuf;

/// Test JSON URL generation from different input formats
#[test]
fn test_json_url_generation_basic() {
    // Test details URL conversion
    let details_url = "https://archive.org/details/example";
    let json_url = get_json_url(details_url);
    assert_eq!(json_url, "https://archive.org/metadata/example");

    // Test with already metadata URL
    let metadata_url = "https://archive.org/metadata/example";
    let json_url = get_json_url(metadata_url);
    assert_eq!(json_url, metadata_url);

    // Test with identifier only
    let identifier = "example-identifier";
    let json_url = get_json_url(identifier);
    assert_eq!(json_url, "https://archive.org/metadata/example-identifier");
}

/// Test metadata URL construction
#[test]
fn test_metadata_url_construction_basic() {
    let identifier = "test-archive";
    let metadata_url = construct_metadata_url(identifier);
    assert!(metadata_url.contains("metadata"));
    assert!(metadata_url.contains(identifier));
    assert!(metadata_url.starts_with("https://"));
}

/// Test identifier extraction from URLs
#[test]
fn test_identifier_extraction_basic() {
    // Test details URL
    let details_url = "https://archive.org/details/example-archive";
    let identifier = extract_identifier_from_url(details_url).unwrap();
    assert_eq!(identifier, "example-archive");

    // Test metadata URL
    let metadata_url = "https://archive.org/metadata/example-archive";
    let identifier = extract_identifier_from_url(metadata_url).unwrap();
    assert_eq!(identifier, "example-archive");

    // Test invalid URL
    let invalid_url = "https://example.com/not-archive";
    let result = extract_identifier_from_url(invalid_url);
    assert!(result.is_err());
}

/// Test JSON metadata parsing with minimal valid data
#[test]
fn test_json_metadata_parsing_minimal() {
    let minimal_json = r#"{
        "created": 1234567890,
        "d1": "ia801234.us.archive.org",
        "d2": "ia801235.us.archive.org",
        "dir": "/test",
        "files": [
            {
                "name": "file.txt",
                "source": "original"
            }
        ],
        "files_count": 1,
        "item_last_updated": 1234567890,
        "item_size": 1024,
        "metadata": {},
        "server": "test-server",
        "uniq": 1234567890,
        "workable_servers": ["test-server"],
        "reviews": []
    }"#;

    let result = parse_archive_metadata(minimal_json);
    assert!(result.is_ok());

    let metadata = result.unwrap();
    assert_eq!(metadata.files.len(), 1);
    assert_eq!(metadata.files[0].name, "file.txt");
    assert_eq!(metadata.files[0].source, "original");
    assert_eq!(metadata.server, "test-server");
    assert_eq!(metadata.dir, "/test");
}

/// Test JSON metadata parsing error handling
#[test]
fn test_json_metadata_parsing_errors() {
    // Test with invalid JSON
    let invalid_json = r#"{ "files": [ incomplete"#;
    let result = parse_archive_metadata(invalid_json);
    assert!(result.is_err());

    // Test with missing required fields - this should fail since we need all required fields
    let missing_files = r#"{ "server": "test" }"#;
    let result = parse_archive_metadata(missing_files);
    assert!(result.is_err());
}

/// Test metadata parsing function wrapper
#[test]
fn test_parse_archive_metadata_function() {
    let json_data = r#"{
        "created": 1234567890,
        "d1": "ia801234.us.archive.org",
        "d2": "ia801235.us.archive.org",
        "dir": "/test-dir",
        "files": [
            { "name": "test.txt", "source": "original", "size": "1024" }
        ],
        "files_count": 1,
        "item_last_updated": 1234567890,
        "item_size": 1024,
        "metadata": {},
        "server": "test-server.archive.org",
        "uniq": 1234567890,
        "workable_servers": ["test-server.archive.org"],
        "reviews": []
    }"#;

    let result = parse_archive_metadata(json_data);
    assert!(result.is_ok());

    let metadata = result.unwrap();
    assert_eq!(metadata.files.len(), 1);
    assert_eq!(metadata.files[0].name, "test.txt");
    assert_eq!(metadata.server, "test-server.archive.org");

    // Test with invalid JSON
    let invalid_data = "not valid json";
    let result = parse_archive_metadata(invalid_data);
    assert!(result.is_err());
}

/// Test URL edge cases
#[test]
fn test_url_edge_cases() {
    // Test with query parameters
    let url_with_params = "https://archive.org/details/example?param=value";
    let json_url = get_json_url(url_with_params);
    assert!(json_url.contains("metadata"));
    assert!(json_url.contains("example"));

    // Test with fragments
    let url_with_fragment = "https://archive.org/details/example#section";
    let json_url = get_json_url(url_with_fragment);
    assert!(json_url.contains("metadata"));
    assert!(json_url.contains("example"));

    // Test with trailing slash
    let url_with_slash = "https://archive.org/details/example/";
    let json_url = get_json_url(url_with_slash);
    assert_eq!(json_url, "https://archive.org/metadata/example/");
}

/// Test identifier extraction edge cases
#[test]
fn test_identifier_extraction_edge_cases() {
    // Test with complex identifiers
    let complex_url = "https://archive.org/details/complex_identifier-with-many_parts.123";
    let identifier = extract_identifier_from_url(complex_url).unwrap();
    assert_eq!(identifier, "complex_identifier-with-many_parts.123");

    // Test with numeric identifier
    let numeric_url = "https://archive.org/details/123456789";
    let identifier = extract_identifier_from_url(numeric_url).unwrap();
    assert_eq!(identifier, "123456789");

    // Test with special characters in path (but valid identifier)
    let special_url = "https://archive.org/details/test-archive_v1.0";
    let identifier = extract_identifier_from_url(special_url).unwrap();
    assert_eq!(identifier, "test-archive_v1.0");
}

/// Test core functionality with real Archive.org data using dry-run on Mario archive
/// This test ensures the unified download service can properly handle real Archive.org data
#[tokio::test]
async fn test_mario_archive_dry_run() {
    // Create the download service
    let service = DownloadService::new().expect("Failed to create DownloadService");

    // Create a dry-run request for the Mario archive
    let request = DownloadRequest {
        identifier: "mario".to_string(),
        output_dir: PathBuf::from("/tmp/test-mario"),
        include_formats: vec![], // Include all formats
        exclude_formats: vec![],
        min_file_size: String::new(),
        max_file_size: None,
        concurrent_downloads: 3,
        enable_compression: true,
        auto_decompress: false,
        decompress_formats: vec![],
        dry_run: true, // Key: this is a dry run - no actual downloads
        verify_md5: true,
        preserve_mtime: true,
        verbose: true,
        resume: true,
    };

    // Execute the dry-run request
    let result = service.download(request, None).await;

    // Verify the request was successful
    assert!(
        result.is_ok(),
        "Download service failed: {:?}",
        result.err()
    );

    // Extract the result and verify it's a success
    match result.unwrap() {
        DownloadResult::Success(session, api_stats) => {
            // Verify basic session data
            assert_eq!(session.identifier, "mario");
            assert!(session.original_url.contains("mario"));
            assert!(
                !session.archive_metadata.files.is_empty(),
                "No files found in mario archive"
            );

            // Verify at least some common file characteristics
            let file_names: Vec<&str> = session
                .archive_metadata
                .files
                .iter()
                .map(|f| f.name.as_str())
                .collect();

            // Mario archive should have some files
            assert!(!file_names.is_empty(), "Mario archive should contain files");

            // Verify metadata structure is complete
            assert!(
                !session.archive_metadata.server.is_empty(),
                "Server should be specified"
            );
            assert!(
                !session.archive_metadata.dir.is_empty(),
                "Directory should be specified"
            );

            // Verify API stats are present (Archive.org compliance monitoring)
            assert!(
                api_stats.is_some(),
                "API stats should be available for monitoring"
            );

            // Print some info for manual verification if verbose testing
            if std::env::var("RUST_TEST_VERBOSE").is_ok() {
                println!("✅ Mario archive dry-run successful:");
                println!("   - Files found: {}", session.archive_metadata.files.len());
                println!("   - Server: {}", session.archive_metadata.server);
                println!("   - Directory: {}", session.archive_metadata.dir);
                if let Some(stats) = api_stats {
                    println!("   - API requests made: {}", stats.request_count);
                }
            }
        }
        DownloadResult::Error(error) => {
            panic!("Expected successful dry-run, got error: {}", error);
        }
    }
}

/// Test core functionality with real Archive.org data using dry-run on Luigi archive
/// This test ensures the unified download service can properly handle Archive.org data with string numbers
#[tokio::test]
async fn test_luigi_archive_dry_run() {
    // Initialize the download service
    let service = DownloadService::new().expect("Failed to create download service");

    // Create test request for Luigi archive (known to have string number fields)
    let request = DownloadRequest {
        identifier: "luigi".to_string(),
        output_dir: PathBuf::from("/tmp/luigi_test"),
        dry_run: true, // Don't actually download
        ..Default::default()
    };

    // Execute the dry-run request
    let result = service.download(request, None).await;

    // Verify the request was successful
    assert!(
        result.is_ok(),
        "Luigi download service failed: {:?}",
        result.err()
    );

    // Extract the result and verify it's a success
    match result.unwrap() {
        DownloadResult::Success(session, api_stats) => {
            // Verify basic session data
            assert_eq!(session.identifier, "luigi");
            assert!(session.original_url.contains("luigi"));
            assert!(
                !session.archive_metadata.files.is_empty(),
                "No files found in luigi archive"
            );

            // Verify API stats are present (Archive.org compliance monitoring)
            assert!(
                api_stats.is_some(),
                "API stats should be available for monitoring"
            );

            // Print some info for manual verification if verbose testing
            if std::env::var("RUST_TEST_VERBOSE").is_ok() {
                println!("✅ Luigi archive dry-run successful:");
                println!("   - Files found: {}", session.archive_metadata.files.len());
                println!("   - Server: {}", session.archive_metadata.server);
                println!("   - Directory: {}", session.archive_metadata.dir);
                if let Some(stats) = api_stats {
                    println!("   - API requests made: {}", stats.request_count);
                }
            }
        }
        DownloadResult::Error(error) => {
            panic!("Expected successful Luigi dry-run, got error: {}", error);
        }
    }
}
