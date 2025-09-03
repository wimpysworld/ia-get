//! File Filtering and Size Formatting Support Layer Tests
//!
//! Tests for file filtering, size parsing, and formatting functionality
//! in the support layer including size string parsing, file filtering,
//! and display formatting.

use ia_get::{
    archive_metadata::JsonFile,
    filters::{filter_files, format_size, parse_size_string, FilterOptions},
};

/// Test size string parsing with various units
#[test]
fn test_size_string_parsing() {
    // Test bytes
    assert_eq!(parse_size_string("1024").unwrap(), 1024);
    assert_eq!(parse_size_string("0").unwrap(), 0);
    assert_eq!(parse_size_string("1").unwrap(), 1);

    // Test with B suffix
    assert_eq!(parse_size_string("1024B").unwrap(), 1024);
    assert_eq!(parse_size_string("512b").unwrap(), 512);

    // Test kilobytes
    assert_eq!(parse_size_string("1KB").unwrap(), 1024);
    assert_eq!(parse_size_string("2kb").unwrap(), 2048);
    assert_eq!(parse_size_string("0.5KB").unwrap(), 512);
    assert_eq!(parse_size_string("1.5KB").unwrap(), 1536);

    // Test megabytes
    assert_eq!(parse_size_string("1MB").unwrap(), 1048576);
    assert_eq!(parse_size_string("2mb").unwrap(), 2097152);
    assert_eq!(parse_size_string("0.5MB").unwrap(), 524288);
    assert_eq!(parse_size_string("1.25MB").unwrap(), 1310720);

    // Test gigabytes
    assert_eq!(parse_size_string("1GB").unwrap(), 1073741824);
    assert_eq!(parse_size_string("2gb").unwrap(), 2147483648);
    assert_eq!(parse_size_string("0.5GB").unwrap(), 536870912);

    // Test terabytes
    assert_eq!(parse_size_string("1TB").unwrap(), 1099511627776);
    assert_eq!(parse_size_string("0.5tb").unwrap(), 549755813888);

    // Basic parsing tests from src
    assert_eq!(parse_size_string("100").unwrap(), 100);
    assert_eq!(parse_size_string("100B").unwrap(), 100);
    assert_eq!(parse_size_string("1KB").unwrap(), 1024);
    assert_eq!(parse_size_string("1MB").unwrap(), 1024 * 1024);
    assert_eq!(parse_size_string("1GB").unwrap(), 1024 * 1024 * 1024);
    assert_eq!(
        parse_size_string("1.5MB").unwrap(),
        (1.5 * 1024.0 * 1024.0) as u64
    );

    assert!(parse_size_string("invalid").is_err());
}

/// Test size string parsing error cases
#[test]
fn test_size_string_parsing_errors() {
    // Invalid number
    assert!(parse_size_string("abc").is_err());
    assert!(parse_size_string("").is_err());
    assert!(parse_size_string("XYZ").is_err());

    // Invalid unit
    assert!(parse_size_string("100XY").is_err());
    assert!(parse_size_string("100 MB").is_err()); // space not allowed

    // Negative numbers
    assert!(parse_size_string("-10MB").is_err());
    assert!(parse_size_string("-5").is_err());
}

/// Test size formatting for display
#[test]
fn test_format_size() {
    assert_eq!(format_size(0), "0 B");
    assert_eq!(format_size(512), "512 B");
    assert_eq!(format_size(1024), "1.0 KB");
    assert_eq!(format_size(1536), "1.5 KB");
    assert_eq!(format_size(1024 * 1024), "1.0 MB");
    assert_eq!(format_size(1024 * 1024 * 1024), "1.0 GB");

    // Additional size formatting tests
    assert_eq!(format_size(1023), "1023 B");
    assert_eq!(format_size(2048), "2.0 KB");
    assert_eq!(format_size(1536), "1.5 KB");
    assert_eq!(format_size(1572864), "1.5 MB");
    assert_eq!(format_size(1073741824 + 536870912), "1.5 GB");
}

/// Create test files for filtering tests
fn create_test_files() -> Vec<JsonFile> {
    vec![
        JsonFile {
            name: "document.pdf".to_string(),
            source: "original".to_string(),
            mtime: None,
            size: Some(1048576), // 1MB
            format: Some("PDF".to_string()),
            rotation: None,
            md5: None,
            crc32: None,
            sha1: None,
            btih: None,
            summation: None,
            original: None,
        },
        JsonFile {
            name: "image.jpg".to_string(),
            source: "original".to_string(),
            mtime: None,
            size: Some(524288), // 512KB
            format: Some("JPEG".to_string()),
            rotation: None,
            md5: None,
            crc32: None,
            sha1: None,
            btih: None,
            summation: None,
            original: None,
        },
        JsonFile {
            name: "video.mp4".to_string(),
            source: "derivative".to_string(),
            mtime: None,
            size: Some(104857600), // 100MB
            format: Some("MP4".to_string()),
            rotation: None,
            md5: None,
            crc32: None,
            sha1: None,
            btih: None,
            summation: None,
            original: None,
        },
        JsonFile {
            name: "data.xml".to_string(),
            source: "metadata".to_string(),
            mtime: None,
            size: Some(2048),
            format: Some("XML".to_string()),
            rotation: None,
            md5: None,
            crc32: None,
            sha1: None,
            btih: None,
            summation: None,
            original: None,
        },
        JsonFile {
            name: "large_file.bin".to_string(),
            source: "original".to_string(),
            mtime: None,
            size: Some(1073741824), // 1GB
            format: None,
            rotation: None,
            md5: None,
            crc32: None,
            sha1: None,
            btih: None,
            summation: None,
            original: None,
        },
    ]
}

/// Create a simple filter options implementation for testing
struct TestFilterOptions {
    include_ext: Option<String>,
    exclude_ext: Option<String>,
    max_file_size: Option<String>,
    source_types: Vec<ia_get::cli::SourceType>,
}

impl FilterOptions for TestFilterOptions {
    fn include_ext(&self) -> &Option<String> {
        &self.include_ext
    }

    fn exclude_ext(&self) -> &Option<String> {
        &self.exclude_ext
    }

    fn max_file_size(&self) -> &Option<String> {
        &self.max_file_size
    }

    fn source_types(&self) -> Vec<ia_get::cli::SourceType> {
        self.source_types.clone()
    }
}

/// Test file filtering with include extensions
#[test]
fn test_filter_files_include_extensions() {
    let files = create_test_files();
    let options = TestFilterOptions {
        include_ext: Some("pdf,jpg".to_string()),
        exclude_ext: None,
        max_file_size: None,
        source_types: vec![ia_get::cli::SourceType::Original],
    };

    let filtered = filter_files(files, &options);
    assert_eq!(filtered.len(), 2);
    assert!(filtered.iter().any(|f| f.name == "document.pdf"));
    assert!(filtered.iter().any(|f| f.name == "image.jpg"));
}

/// Test file filtering with exclude extensions
#[test]
fn test_filter_files_exclude_extensions() {
    let files = create_test_files();
    let options = TestFilterOptions {
        include_ext: None,
        exclude_ext: Some("xml,bin".to_string()),
        max_file_size: None,
        source_types: vec![
            ia_get::cli::SourceType::Original,
            ia_get::cli::SourceType::Derivative,
            ia_get::cli::SourceType::Metadata,
        ],
    };

    let filtered = filter_files(files, &options);
    assert_eq!(filtered.len(), 3);
    assert!(!filtered.iter().any(|f| f.name == "data.xml"));
    assert!(!filtered.iter().any(|f| f.name == "large_file.bin"));
}

/// Test file filtering with size limits
#[test]
fn test_filter_files_size_limits() {
    let files = create_test_files();
    let options = TestFilterOptions {
        include_ext: None,
        exclude_ext: None,
        max_file_size: Some("10MB".to_string()),
        source_types: vec![
            ia_get::cli::SourceType::Original,
            ia_get::cli::SourceType::Derivative,
            ia_get::cli::SourceType::Metadata,
        ],
    };

    let filtered = filter_files(files, &options);
    assert_eq!(filtered.len(), 3);
    assert!(!filtered.iter().any(|f| f.name == "video.mp4"));
    assert!(!filtered.iter().any(|f| f.name == "large_file.bin"));
}

/// Test file filtering with source types
#[test]
fn test_filter_files_source_types() {
    let files = create_test_files();
    let options = TestFilterOptions {
        include_ext: None,
        exclude_ext: None,
        max_file_size: None,
        source_types: vec![ia_get::cli::SourceType::Original],
    };

    let filtered = filter_files(files, &options);
    assert_eq!(filtered.len(), 3);
    assert!(filtered.iter().all(|f| f.source == "original"));
}

/// Test complex filtering combinations
#[test]
fn test_filter_files_complex_combinations() {
    let files = create_test_files();
    let options = TestFilterOptions {
        include_ext: Some("pdf,jpg,mp4".to_string()),
        exclude_ext: None,
        max_file_size: Some("2MB".to_string()),
        source_types: vec![ia_get::cli::SourceType::Original],
    };

    let filtered = filter_files(files, &options);
    assert_eq!(filtered.len(), 2);
    assert!(filtered.iter().any(|f| f.name == "document.pdf"));
    assert!(filtered.iter().any(|f| f.name == "image.jpg"));
    assert!(!filtered.iter().any(|f| f.name == "video.mp4")); // derivative source
}
