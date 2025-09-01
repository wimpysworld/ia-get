//! File Filtering and Processing Testing Module
//!
//! Tests for file filtering, size parsing, and formatting functionality including:
//! - Size string parsing and validation
//! - File filtering by extension, size, and pattern
//! - Size formatting for display
//! - Filter combinations and edge cases

use ia_get::{
    filters::{filter_files, format_size, parse_size_string},
    metadata_storage::{ArchiveFile, ArchiveMetadata},
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
}

/// Test size string parsing error cases
#[test]
fn test_size_string_parsing_errors() {
    // Test invalid formats
    assert!(parse_size_string("").is_err());
    assert!(parse_size_string("abc").is_err());
    assert!(parse_size_string("1XB").is_err());
    assert!(parse_size_string("1.2.3MB").is_err());
    assert!(parse_size_string("-1MB").is_err());
    assert!(parse_size_string("1 MB").is_err()); // Space not allowed
    assert!(parse_size_string("1MBB").is_err());
    assert!(parse_size_string("MB1").is_err());
}

/// Test size formatting for display
#[test]
fn test_size_formatting() {
    // Test bytes
    assert_eq!(format_size(0), "0 B");
    assert_eq!(format_size(1), "1 B");
    assert_eq!(format_size(512), "512 B");
    assert_eq!(format_size(1023), "1023 B");

    // Test kilobytes
    assert_eq!(format_size(1024), "1.0 KB");
    assert_eq!(format_size(1536), "1.5 KB");
    assert_eq!(format_size(2048), "2.0 KB");
    assert_eq!(format_size(1048575), "1024.0 KB");

    // Test megabytes
    assert_eq!(format_size(1048576), "1.0 MB");
    assert_eq!(format_size(1572864), "1.5 MB");
    assert_eq!(format_size(2097152), "2.0 MB");

    // Test gigabytes
    assert_eq!(format_size(1073741824), "1.0 GB");
    assert_eq!(format_size(1610612736), "1.5 GB");

    // Test terabytes
    assert_eq!(format_size(1099511627776), "1.0 TB");
    assert_eq!(format_size(1649267441664), "1.5 TB");
}

/// Create test archive file helper
fn create_test_file(name: &str, size: Option<u64>, format: Option<&str>) -> ArchiveFile {
    ArchiveFile {
        name: name.to_string(),
        source: Some("original".to_string()),
        mtime: Some("1234567890".to_string()),
        size: size.map(|s| s.to_string()),
        md5: Some("test-hash".to_string()),
        crc32: None,
        sha1: None,
        format: format.map(|f| f.to_string()),
        width: None,
        height: None,
        length: None,
    }
}

/// Test file filtering by extension inclusion
#[test]
fn test_file_filtering_include_extensions() {
    let files = vec![
        create_test_file("document.pdf", Some(1024), Some("PDF")),
        create_test_file("image.jpg", Some(2048), Some("JPEG")),
        create_test_file("text.txt", Some(512), Some("Text")),
        create_test_file("archive.zip", Some(4096), Some("ZIP")),
        create_test_file("no-extension", Some(256), None),
    ];

    // Test including PDF files only
    let include_ext = vec!["pdf".to_string()];
    let filtered = filter_files(&files, &include_ext, &[], None);
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].name, "document.pdf");

    // Test including multiple extensions
    let include_ext = vec!["pdf".to_string(), "txt".to_string()];
    let filtered = filter_files(&files, &include_ext, &[], None);
    assert_eq!(filtered.len(), 2);
    assert!(filtered.iter().any(|f| f.name == "document.pdf"));
    assert!(filtered.iter().any(|f| f.name == "text.txt"));

    // Test case insensitive matching
    let include_ext = vec!["PDF".to_string(), "TXT".to_string()];
    let filtered = filter_files(&files, &include_ext, &[], None);
    assert_eq!(filtered.len(), 2);
}

/// Test file filtering by extension exclusion
#[test]
fn test_file_filtering_exclude_extensions() {
    let files = vec![
        create_test_file("document.pdf", Some(1024), Some("PDF")),
        create_test_file("image.jpg", Some(2048), Some("JPEG")),
        create_test_file("text.txt", Some(512), Some("Text")),
        create_test_file("archive.zip", Some(4096), Some("ZIP")),
        create_test_file("no-extension", Some(256), None),
    ];

    // Test excluding ZIP files
    let exclude_ext = vec!["zip".to_string()];
    let filtered = filter_files(&files, &[], &exclude_ext, None);
    assert_eq!(filtered.len(), 4);
    assert!(!filtered.iter().any(|f| f.name == "archive.zip"));

    // Test excluding multiple extensions
    let exclude_ext = vec!["zip".to_string(), "jpg".to_string()];
    let filtered = filter_files(&files, &[], &exclude_ext, None);
    assert_eq!(filtered.len(), 3);
    assert!(!filtered.iter().any(|f| f.name == "archive.zip"));
    assert!(!filtered.iter().any(|f| f.name == "image.jpg"));
}

/// Test file filtering by maximum size
#[test]
fn test_file_filtering_max_size() {
    let files = vec![
        create_test_file("small.txt", Some(100), Some("Text")),
        create_test_file("medium.pdf", Some(1500), Some("PDF")),
        create_test_file("large.zip", Some(5000), Some("ZIP")),
        create_test_file("no-size", None, Some("Unknown")),
    ];

    // Test filtering by max size
    let filtered = filter_files(&files, &[], &[], Some(2000));
    assert_eq!(filtered.len(), 3); // small, medium, and no-size
    assert!(filtered.iter().any(|f| f.name == "small.txt"));
    assert!(filtered.iter().any(|f| f.name == "medium.pdf"));
    assert!(filtered.iter().any(|f| f.name == "no-size")); // files without size pass through
    assert!(!filtered.iter().any(|f| f.name == "large.zip"));

    // Test very small max size
    let filtered = filter_files(&files, &[], &[], Some(50));
    assert_eq!(filtered.len(), 1); // only no-size should pass
    assert_eq!(filtered[0].name, "no-size");

    // Test very large max size
    let filtered = filter_files(&files, &[], &[], Some(10000));
    assert_eq!(filtered.len(), 4); // all files should pass
}

/// Test combined filtering (inclusion + exclusion + size)
#[test]
fn test_file_filtering_combined() {
    let files = vec![
        create_test_file("small.pdf", Some(100), Some("PDF")),
        create_test_file("large.pdf", Some(5000), Some("PDF")),
        create_test_file("small.txt", Some(200), Some("Text")),
        create_test_file("large.txt", Some(6000), Some("Text")),
        create_test_file("medium.jpg", Some(1500), Some("JPEG")),
        create_test_file("medium.zip", Some(2000), Some("ZIP")),
    ];

    // Include PDF and TXT, exclude TXT, max size 3000
    // Should result in only small PDF files
    let include_ext = vec!["pdf".to_string(), "txt".to_string()];
    let exclude_ext = vec!["txt".to_string()];
    let filtered = filter_files(&files, &include_ext, &exclude_ext, Some(3000));

    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].name, "small.pdf");
}

/// Test filtering with no filters applied
#[test]
fn test_file_filtering_no_filters() {
    let files = vec![
        create_test_file("file1.pdf", Some(1000), Some("PDF")),
        create_test_file("file2.txt", Some(2000), Some("Text")),
        create_test_file("file3.jpg", Some(3000), Some("JPEG")),
    ];

    // No filters should return all files
    let filtered = filter_files(&files, &[], &[], None);
    assert_eq!(filtered.len(), 3);
}

/// Test filtering with empty file list
#[test]
fn test_file_filtering_empty_list() {
    let files = vec![];

    let filtered = filter_files(&files, &["pdf".to_string()], &[], None);
    assert_eq!(filtered.len(), 0);
}

/// Test extension extraction edge cases
#[test]
fn test_extension_extraction_edge_cases() {
    let files = vec![
        create_test_file("file.tar.gz", Some(1000), Some("Gzip")),
        create_test_file("file.backup.txt", Some(1000), Some("Text")),
        create_test_file("file.", Some(1000), None),
        create_test_file(".hidden", Some(1000), None),
        create_test_file("no-extension", Some(1000), None),
        create_test_file("FILE.PDF", Some(1000), Some("PDF")), // uppercase extension
    ];

    // Test filtering tar.gz files (should match on 'gz')
    let include_ext = vec!["gz".to_string()];
    let filtered = filter_files(&files, &include_ext, &[], None);
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].name, "file.tar.gz");

    // Test filtering txt files (should match backup.txt)
    let include_ext = vec!["txt".to_string()];
    let filtered = filter_files(&files, &include_ext, &[], None);
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].name, "file.backup.txt");

    // Test case insensitive matching
    let include_ext = vec!["pdf".to_string()];
    let filtered = filter_files(&files, &include_ext, &[], None);
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].name, "FILE.PDF");
}

/// Test size parsing with decimal precision
#[test]
fn test_size_parsing_decimal_precision() {
    assert_eq!(parse_size_string("1.1KB").unwrap(), 1126); // 1.1 * 1024
    assert_eq!(parse_size_string("2.5MB").unwrap(), 2621440); // 2.5 * 1048576
    assert_eq!(parse_size_string("0.25GB").unwrap(), 268435456); // 0.25 * 1073741824

    // Test very small decimals
    assert_eq!(parse_size_string("0.001MB").unwrap(), 1048); // rounds down

    // Test large decimals
    assert_eq!(parse_size_string("999.999KB").unwrap(), 1023998); // 999.999 * 1024
}

/// Test filter performance with large file lists
#[test]
fn test_filter_performance() {
    // Create a large list of files
    let mut files = Vec::new();
    for i in 0..1000 {
        let name = format!("file{}.{}", i, if i % 2 == 0 { "pdf" } else { "txt" });
        files.push(create_test_file(&name, Some(i * 1000), Some("Test")));
    }

    // Filter should complete quickly
    let start = std::time::Instant::now();
    let include_ext = vec!["pdf".to_string()];
    let filtered = filter_files(&files, &include_ext, &[], Some(500000));
    let duration = start.elapsed();

    // Should filter correctly
    let expected_count = files
        .iter()
        .filter(|f| f.name.ends_with(".pdf") && f.size.unwrap_or(0) <= 500_000)
        .count();
    assert_eq!(filtered.len(), expected_count); // Files with .pdf extension and size <= 500_000

    // Should complete quickly (under 100ms for 1000 files)
    assert!(duration.as_millis() < 100);
}

/// Test size formatting edge cases
#[test]
fn test_size_formatting_edge_cases() {
    // Test very large numbers
    assert_eq!(format_size(u64::MAX), "16777216.0 TB");

    // Test boundary values
    assert_eq!(format_size(1023), "1023 B");
    assert_eq!(format_size(1024), "1.0 KB");
    assert_eq!(format_size(1048575), "1024.0 KB");
    assert_eq!(format_size(1048576), "1.0 MB");
}

/// Test filtering with files without size information
#[test]
fn test_filtering_files_without_size() {
    let files = vec![
        create_test_file("with-size.pdf", Some(1000), Some("PDF")),
        create_test_file("no-size.pdf", None, Some("PDF")),
        create_test_file("zero-size.pdf", Some(0), Some("PDF")),
    ];

    // Files without size should pass through size filters
    let filtered = filter_files(&files, &[], &[], Some(500));
    assert_eq!(filtered.len(), 2); // no-size and zero-size should pass
    assert!(filtered.iter().any(|f| f.name == "no-size.pdf"));
    assert!(filtered.iter().any(|f| f.name == "zero-size.pdf"));
    assert!(!filtered.iter().any(|f| f.name == "with-size.pdf"));
}
