//! Compression Support Layer Tests
//!
//! Tests for compression detection, format identification, and decompression
//! functionality in the support layer.

use ia_get::compression::CompressionFormat;

#[test]
fn test_compression_format_detection() {
    // Test various file extensions
    assert_eq!(
        CompressionFormat::from_filename("archive.tar.gz"),
        Some(CompressionFormat::TarGz)
    );
    assert_eq!(
        CompressionFormat::from_filename("data.bz2"),
        Some(CompressionFormat::Bzip2)
    );
    assert_eq!(
        CompressionFormat::from_filename("file.xz"),
        Some(CompressionFormat::Xz)
    );
    assert_eq!(
        CompressionFormat::from_filename("document.zip"),
        Some(CompressionFormat::Zip)
    );
    assert_eq!(
        CompressionFormat::from_filename("archive.tar"),
        Some(CompressionFormat::Tar)
    );
    assert_eq!(
        CompressionFormat::from_filename("data.gz"),
        Some(CompressionFormat::Gzip)
    );
    assert_eq!(
        CompressionFormat::from_filename("archive.tar.bz2"),
        Some(CompressionFormat::TarBz2)
    );
    assert_eq!(
        CompressionFormat::from_filename("archive.tar.xz"),
        Some(CompressionFormat::TarXz)
    );
    assert_eq!(CompressionFormat::from_filename("plain.txt"), None);
}

#[test]
fn test_compression_format_case_insensitive() {
    // Test case insensitive detection
    assert_eq!(
        CompressionFormat::from_filename("ARCHIVE.TAR.GZ"),
        Some(CompressionFormat::TarGz)
    );
    assert_eq!(
        CompressionFormat::from_filename("Data.BZ2"),
        Some(CompressionFormat::Bzip2)
    );
    assert_eq!(
        CompressionFormat::from_filename("File.XZ"),
        Some(CompressionFormat::Xz)
    );
    assert_eq!(
        CompressionFormat::from_filename("DOCUMENT.ZIP"),
        Some(CompressionFormat::Zip)
    );
}

#[test]
fn test_compression_format_complex_names() {
    // Test with complex filenames
    assert_eq!(
        CompressionFormat::from_filename("very-long-filename-with-dashes.tar.gz"),
        Some(CompressionFormat::TarGz)
    );
    assert_eq!(
        CompressionFormat::from_filename("file.name.with.dots.bz2"),
        Some(CompressionFormat::Bzip2)
    );
    assert_eq!(
        CompressionFormat::from_filename("123-numbers-456.tar.xz"),
        Some(CompressionFormat::TarXz)
    );
}

#[test]
fn test_compression_format_edge_cases() {
    // Test edge cases
    assert_eq!(CompressionFormat::from_filename(""), None);
    assert_eq!(
        CompressionFormat::from_filename(".gz"),
        Some(CompressionFormat::Gzip)
    );
    assert_eq!(CompressionFormat::from_filename("file."), None);
    assert_eq!(CompressionFormat::from_filename("file.unknown"), None);
    assert_eq!(CompressionFormat::from_filename("tar.gz.txt"), None);
}

#[test]
fn test_compression_format_string_methods() {
    // Test the format enum values
    let formats = [
        CompressionFormat::Gzip,
        CompressionFormat::Bzip2,
        CompressionFormat::Xz,
        CompressionFormat::Zip,
        CompressionFormat::Tar,
        CompressionFormat::TarGz,
        CompressionFormat::TarBz2,
        CompressionFormat::TarXz,
    ];

    for format in formats {
        // Test that debug formatting works
        let debug_str = format!("{:?}", format);
        assert!(!debug_str.is_empty());

        // Test that all enum variants can be formatted as strings (most should be short)
        assert!(debug_str.len() >= 2); // Should at least contain the enum name
    }
}

// Note: Actual decompression tests would require file I/O and are better suited
// for integration tests. These tests focus on the compression format detection
// and logic that can be tested without file system operations.
