//! Integration tests for compression functionality

use ia_get::compression::{decompress_file, should_decompress, CompressionFormat};
use ia_get::metadata_storage::ArchiveFile;
use std::fs::File;
use std::io::Write;
use tempfile::TempDir;

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
    assert_eq!(CompressionFormat::from_filename("plain.txt"), None);
}

#[test]
fn test_archive_file_compression_detection() {
    // Test file with explicit format
    let compressed_file = ArchiveFile {
        name: "data.tar.gz".to_string(),
        source: "original".to_string(),
        format: Some("gzip".to_string()),
        mtime: None,
        size: Some(1024),
        md5: None,
        crc32: None,
        sha1: None,
        btih: None,
        summation: None,
        original: None,
        rotation: None,
    };

    assert!(compressed_file.is_compressed());
    assert_eq!(
        compressed_file.get_compression_format(),
        Some("gzip".to_string())
    );
    assert_eq!(compressed_file.get_decompressed_name(), "data.tar");

    // Test file without explicit format (extension-based detection)
    let extension_based = ArchiveFile {
        name: "archive.tar.bz2".to_string(),
        source: "original".to_string(),
        format: None,
        mtime: None,
        size: Some(2048),
        md5: None,
        crc32: None,
        sha1: None,
        btih: None,
        summation: None,
        original: None,
        rotation: None,
    };

    assert!(extension_based.is_compressed());
    assert_eq!(
        extension_based.get_compression_format(),
        Some("bzip2".to_string())
    );
    assert_eq!(extension_based.get_decompressed_name(), "archive.tar");

    // Test non-compressed file
    let plain_file = ArchiveFile {
        name: "document.txt".to_string(),
        source: "original".to_string(),
        format: Some("text".to_string()),
        mtime: None,
        size: Some(512),
        md5: None,
        crc32: None,
        sha1: None,
        btih: None,
        summation: None,
        original: None,
        rotation: None,
    };

    assert!(!plain_file.is_compressed());
    assert_eq!(plain_file.get_compression_format(), None);
    assert_eq!(plain_file.get_decompressed_name(), "document.txt");
}

#[test]
fn test_should_decompress_configuration() {
    let gzip_format = CompressionFormat::Gzip;
    let zip_format = CompressionFormat::Zip;
    let tar_gz_format = CompressionFormat::TarGz;

    // Test default behavior (empty config enables common formats)
    assert!(should_decompress(&gzip_format, &[]));
    assert!(should_decompress(&tar_gz_format, &[])); // tar.gz is now included in default list
    assert!(!should_decompress(&zip_format, &[]));

    // Test explicit configuration
    let config = vec!["zip".to_string(), "gzip".to_string()];
    assert!(should_decompress(&gzip_format, &config));
    assert!(should_decompress(&zip_format, &config));
    assert!(!should_decompress(&tar_gz_format, &config));
}

#[test]
fn test_gzip_decompression() {
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Read;

    let temp_dir = TempDir::new().unwrap();

    // Create test data
    let test_data = b"Hello, World! This is a test of gzip compression.";

    // Create compressed file
    let compressed_path = temp_dir.path().join("test.txt.gz");
    let compressed_file = File::create(&compressed_path).unwrap();
    let mut encoder = GzEncoder::new(compressed_file, Compression::default());
    encoder.write_all(test_data).unwrap();
    encoder.finish().unwrap();

    // Decompress using our function
    let decompressed_path = temp_dir.path().join("test.txt");
    decompress_file(
        &compressed_path,
        &decompressed_path,
        CompressionFormat::Gzip,
        None,
    )
    .unwrap();

    // Verify decompressed content
    let mut decompressed_file = File::open(&decompressed_path).unwrap();
    let mut decompressed_content = Vec::new();
    decompressed_file
        .read_to_end(&mut decompressed_content)
        .unwrap();

    assert_eq!(decompressed_content, test_data);
}

#[test]
fn test_decompressed_name_generation() {
    let formats_and_names = vec![
        (CompressionFormat::Gzip, "file.txt.gz", "file.txt"),
        (CompressionFormat::Bzip2, "data.bz2", "data"),
        (CompressionFormat::Xz, "archive.xz", "archive"),
        (CompressionFormat::TarGz, "backup.tar.gz", "backup.tar"),
        (CompressionFormat::TarBz2, "backup.tar.bz2", "backup.tar"),
        (CompressionFormat::TarXz, "backup.tar.xz", "backup.tar"),
        (CompressionFormat::Zip, "folder.zip", "folder"),
        (CompressionFormat::Tar, "archive.tar", "archive"),
    ];

    for (format, input, expected) in formats_and_names {
        assert_eq!(
            format.get_decompressed_name(input),
            expected,
            "Failed for format {:?} with input '{}'",
            format,
            input
        );
    }
}
