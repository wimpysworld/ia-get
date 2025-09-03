//! Metadata Processing API Layer Tests
//!
//! Tests for Internet Archive metadata functionality including JSON metadata
//! URL generation, metadata parsing and validation, archive file structure
//! handling, and error handling in metadata processing.

use ia_get::{
    metadata::get_json_url,
    metadata_storage::{ArchiveFile, ArchiveMetadata},
    url_processing::{construct_metadata_url, extract_identifier_from_url},
};

/// Test JSON URL generation from different input formats
#[test]
fn test_json_url_generation() {
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

    // Test with complex identifier
    let complex_id = "complex_identifier-with-dashes_123";
    let json_url = get_json_url(complex_id);
    assert_eq!(
        json_url,
        "https://archive.org/metadata/complex_identifier-with-dashes_123"
    );
}

/// Test metadata URL construction
#[test]
fn test_metadata_url_construction() {
    let identifier = "test-archive";
    let metadata_url = construct_metadata_url(identifier);
    assert!(metadata_url.contains("metadata"));
    assert!(metadata_url.contains(identifier));
    assert!(metadata_url.starts_with("https://"));
}

/// Test identifier extraction from URLs
#[test]
fn test_identifier_extraction() {
    // Test details URL
    let details_url = "https://archive.org/details/example-archive";
    let identifier = extract_identifier_from_url(details_url).unwrap();
    assert_eq!(identifier, "example-archive");

    // Test metadata URL
    let metadata_url = "https://archive.org/metadata/example-archive";
    let identifier = extract_identifier_from_url(metadata_url).unwrap();
    assert_eq!(identifier, "example-archive");

    // Test download URL
    let download_url = "https://archive.org/download/example-archive/file.pdf";
    let identifier = extract_identifier_from_url(download_url).unwrap();
    assert_eq!(identifier, "example-archive");

    // Test invalid URL
    let invalid_url = "https://example.com/not-archive";
    let result = extract_identifier_from_url(invalid_url);
    assert!(result.is_err());
}

/// Test JSON metadata parsing with valid data
#[test]
fn test_json_metadata_parsing_valid() {
    let json_data = r#"{
        "created": 1234567890,
        "d1": "ia801234.us.archive.org",
        "d2": "ia801235.us.archive.org",
        "dir": "/example",
        "files": [
            {
                "name": "example.pdf",
                "source": "original",
                "mtime": "1234567890",
                "size": "1024000",
                "md5": "d41d8cd98f00b204e9800998ecf8427e",
                "crc32": "00000000",
                "sha1": "da39a3ee5e6b4b0d3255bfef95601890afd80709",
                "format": "PDF"
            },
            {
                "name": "example.txt",
                "source": "original", 
                "mtime": "1234567890",
                "size": "2048",
                "md5": "098f6bcd4621d373cade4e832627b4f6",
                "format": "Text"
            }
        ],
        "files_count": 2,
        "item_last_updated": 1234567890,
        "item_size": 1026048,
        "metadata": {},
        "server": "ia801234.us.archive.org",
        "uniq": 987654321,
        "workable_servers": ["ia801234.us.archive.org", "ia801235.us.archive.org"]
    }"#;

    let metadata: ArchiveMetadata = serde_json::from_str(json_data).unwrap();

    // Verify basic structure
    assert_eq!(metadata.files.len(), 2);
    assert_eq!(metadata.d1, "ia801234.us.archive.org");
    assert_eq!(metadata.dir, "/example");

    // Verify file details
    let pdf_file = &metadata.files[0];
    assert_eq!(pdf_file.name, "example.pdf");
    assert_eq!(pdf_file.size, Some(1024000));
    assert_eq!(pdf_file.format, Some("PDF".to_string()));
    assert_eq!(
        pdf_file.md5,
        Some("d41d8cd98f00b204e9800998ecf8427e".to_string())
    );

    let txt_file = &metadata.files[1];
    assert_eq!(txt_file.name, "example.txt");
    assert_eq!(txt_file.size, Some(2048));
    assert_eq!(txt_file.format, Some("Text".to_string()));
}

/// Test JSON metadata parsing with minimal required fields
#[test]
fn test_json_metadata_parsing_minimal() {
    let minimal_json = r#"{
        "created": 1234567890,
        "d1": "test-server",
        "d2": "test-server2",
        "dir": "/test",
        "files": [
            {
                "name": "file.txt",
                "source": "original"
            }
        ],
        "files_count": 1,
        "item_last_updated": 1234567890,
        "item_size": 0,
        "metadata": {},
        "server": "test-server",
        "uniq": 123456,
        "workable_servers": ["test-server"]
    }"#;

    let metadata: ArchiveMetadata = serde_json::from_str(minimal_json).unwrap();

    assert_eq!(metadata.files.len(), 1);
    assert_eq!(metadata.files[0].name, "file.txt");
    assert_eq!(metadata.d1, "test-server");
    assert_eq!(metadata.dir, "/test");

    // Verify optional fields are None/default
    assert_eq!(metadata.files[0].size, None);
    assert_eq!(metadata.files[0].md5, None);
    assert_eq!(metadata.files[0].format, None);
}

/// Test JSON metadata parsing error handling
#[test]
fn test_json_metadata_parsing_errors() {
    // Test with invalid JSON
    let invalid_json = r#"{ "files": [ incomplete"#;
    let result: Result<ArchiveMetadata, _> = serde_json::from_str(invalid_json);
    assert!(result.is_err());

    // Test with missing required fields
    let missing_files = r#"{ "d1": "test", "d2": "test2", "dir": "/test", "created": 123 }"#;
    let result: Result<ArchiveMetadata, _> = serde_json::from_str(missing_files);
    assert!(result.is_err());

    // Test with wrong data types
    let wrong_types = r#"{ "files": "not-an-array", "d1": "test", "d2": "test2", "dir": "/test", "created": 123 }"#;
    let result: Result<ArchiveMetadata, _> = serde_json::from_str(wrong_types);
    assert!(result.is_err());
}

/// Test archive file structure
#[test]
fn test_archive_file_structure() {
    let file_json = r#"{
        "name": "test.pdf",
        "source": "original",
        "mtime": "1234567890",
        "size": "1048576",
        "md5": "d41d8cd98f00b204e9800998ecf8427e",
        "crc32": "00000000",
        "sha1": "da39a3ee5e6b4b0d3255bfef95601890afd80709",
        "format": "PDF"
    }"#;

    let file: ArchiveFile = serde_json::from_str(file_json).unwrap();

    assert_eq!(file.name, "test.pdf");
    assert_eq!(file.source, "original");
    assert_eq!(file.size, Some(1048576));
    assert_eq!(file.format, Some("PDF".to_string()));
    assert_eq!(
        file.md5,
        Some("d41d8cd98f00b204e9800998ecf8427e".to_string())
    );
    assert_eq!(file.crc32, Some("00000000".to_string()));
    assert_eq!(
        file.sha1,
        Some("da39a3ee5e6b4b0d3255bfef95601890afd80709".to_string())
    );
}

/// Test archive file with minimal fields
#[test]
fn test_archive_file_minimal() {
    let minimal_file_json = r#"{
        "name": "simple.txt",
        "source": "original"
    }"#;

    let file: ArchiveFile = serde_json::from_str(minimal_file_json).unwrap();

    assert_eq!(file.name, "simple.txt");
    assert_eq!(file.source, "original"); // Set in JSON
    assert_eq!(file.size, None);
    assert_eq!(file.format, None);
    assert_eq!(file.md5, None);
}

/// Test metadata parsing with various file types
#[test]
fn test_metadata_parsing_various_file_types() {
    let json_data = r#"{
        "created": 1234567890,
        "d1": "ia801234.us.archive.org",
        "d2": "ia801235.us.archive.org", 
        "dir": "/example",
        "files": [
            {
                "name": "document.pdf",
                "source": "original",
                "format": "PDF",
                "size": "1048576"
            },
            {
                "name": "video.mp4",
                "source": "derivative",
                "format": "h.264",
                "size": "104857600"
            },
            {
                "name": "metadata.xml",
                "source": "metadata",
                "format": "Metadata",
                "size": "2048"
            }
        ],
        "files_count": 3,
        "item_last_updated": 1234567890,
        "item_size": 105908224,
        "metadata": {},
        "server": "ia801234.us.archive.org",
        "uniq": 987654321,
        "workable_servers": ["ia801234.us.archive.org"]
    }"#;

    let metadata: ArchiveMetadata = serde_json::from_str(json_data).unwrap();

    assert_eq!(metadata.files.len(), 3);

    // Check different source types
    assert_eq!(metadata.files[0].source, "original");
    assert_eq!(metadata.files[1].source, "derivative");
    assert_eq!(metadata.files[2].source, "metadata");

    // Check different formats
    assert_eq!(metadata.files[0].format, Some("PDF".to_string()));
    assert_eq!(metadata.files[1].format, Some("h.264".to_string()));
    assert_eq!(metadata.files[2].format, Some("Metadata".to_string()));
}
