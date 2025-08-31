//! Metadata Processing Testing Module
//!
//! Tests for Internet Archive metadata functionality including:
//! - JSON metadata URL generation
//! - Metadata parsing and validation
//! - Archive file structure handling
//! - Error handling in metadata processing

use ia_get::{
    metadata::{get_json_url, parse_archive_metadata},
    metadata_storage::{ArchiveMetadata, ArchiveFile},
    url_processing::{extract_identifier_from_url, construct_metadata_url}
};
use serde_json;

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
    assert_eq!(json_url, "https://archive.org/metadata/complex_identifier-with-dashes_123");
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
        "d1": "archive.org",
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
        "metadata": {
            "identifier": "example-archive",
            "title": "Example Archive",
            "description": "A test archive for validation"
        },
        "misc": {},
        "reviews": [],
        "server": "ia801234.us.archive.org",
        "uniq": 1234567890,
        "workable_servers": ["ia801234.us.archive.org"]
    }"#;
    
    let metadata: ArchiveMetadata = serde_json::from_str(json_data).unwrap();
    
    // Verify basic structure
    assert_eq!(metadata.files.len(), 2);
    assert_eq!(metadata.server, "ia801234.us.archive.org");
    assert_eq!(metadata.dir, "/example");
    
    // Verify file details
    let pdf_file = &metadata.files[0];
    assert_eq!(pdf_file.name, "example.pdf");
    assert_eq!(pdf_file.size, Some("1024000".to_string()));
    assert_eq!(pdf_file.format, Some("PDF".to_string()));
    assert_eq!(pdf_file.md5, Some("d41d8cd98f00b204e9800998ecf8427e".to_string()));
    
    let txt_file = &metadata.files[1];
    assert_eq!(txt_file.name, "example.txt");
    assert_eq!(txt_file.size, Some("2048".to_string()));
    assert_eq!(txt_file.format, Some("Text".to_string()));
}

/// Test JSON metadata parsing with minimal required fields
#[test]
fn test_json_metadata_parsing_minimal() {
    let minimal_json = r#"{
        "files": [
            {
                "name": "file.txt"
            }
        ],
        "server": "test-server",
        "dir": "/test"
    }"#;
    
    let metadata: ArchiveMetadata = serde_json::from_str(minimal_json).unwrap();
    
    assert_eq!(metadata.files.len(), 1);
    assert_eq!(metadata.files[0].name, "file.txt");
    assert_eq!(metadata.server, "test-server");
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
    let missing_files = r#"{ "server": "test" }"#;
    let result: Result<ArchiveMetadata, _> = serde_json::from_str(missing_files);
    assert!(result.is_err());
    
    // Test with wrong data types
    let wrong_types = r#"{ "files": "not-an-array", "server": "test", "dir": "/test" }"#;
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
        "format": "PDF",
        "width": "612",
        "height": "792"
    }"#;
    
    let file: ArchiveFile = serde_json::from_str(file_json).unwrap();
    
    assert_eq!(file.name, "test.pdf");
    assert_eq!(file.source, Some("original".to_string()));
    assert_eq!(file.mtime, Some("1234567890".to_string()));
    assert_eq!(file.size, Some("1048576".to_string()));
    assert_eq!(file.md5, Some("d41d8cd98f00b204e9800998ecf8427e".to_string()));
    assert_eq!(file.format, Some("PDF".to_string()));
    assert_eq!(file.width, Some("612".to_string()));
    assert_eq!(file.height, Some("792".to_string()));
}

/// Test archive file with minimal data
#[test]
fn test_archive_file_minimal() {
    let minimal_file_json = r#"{ "name": "minimal.txt" }"#;
    let file: ArchiveFile = serde_json::from_str(minimal_file_json).unwrap();
    
    assert_eq!(file.name, "minimal.txt");
    assert_eq!(file.source, None);
    assert_eq!(file.size, None);
    assert_eq!(file.md5, None);
    assert_eq!(file.format, None);
}

/// Test metadata parsing function wrapper
#[test]
fn test_parse_archive_metadata_function() {
    let json_data = r#"{
        "files": [
            { "name": "test.txt", "size": "1024" }
        ],
        "server": "test-server.archive.org",
        "dir": "/test-dir"
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

/// Test complex archive metadata with various file types
#[test]
fn test_complex_archive_metadata() {
    let complex_json = r#"{
        "files": [
            {
                "name": "document.pdf",
                "source": "original",
                "size": "1048576",
                "format": "PDF",
                "md5": "abc123"
            },
            {
                "name": "image.jpg", 
                "source": "derivative",
                "size": "256000",
                "format": "JPEG",
                "width": "1920",
                "height": "1080"
            },
            {
                "name": "metadata.xml",
                "source": "metadata",
                "size": "4096",
                "format": "Metadata"
            },
            {
                "name": "archive.zip",
                "source": "original",
                "size": "10485760",
                "format": "ZIP"
            }
        ],
        "server": "ia801234.us.archive.org",
        "dir": "/complex-example",
        "metadata": {
            "identifier": "complex-example",
            "title": "Complex Example Archive",
            "creator": "Test Creator",
            "date": "2023-01-01"
        }
    }"#;
    
    let metadata: ArchiveMetadata = serde_json::from_str(complex_json).unwrap();
    
    assert_eq!(metadata.files.len(), 4);
    
    // Verify different file types
    let pdf_file = metadata.files.iter().find(|f| f.name == "document.pdf").unwrap();
    assert_eq!(pdf_file.format, Some("PDF".to_string()));
    assert_eq!(pdf_file.source, Some("original".to_string()));
    
    let image_file = metadata.files.iter().find(|f| f.name == "image.jpg").unwrap();
    assert_eq!(image_file.format, Some("JPEG".to_string()));
    assert_eq!(image_file.width, Some("1920".to_string()));
    assert_eq!(image_file.height, Some("1080".to_string()));
    
    let metadata_file = metadata.files.iter().find(|f| f.name == "metadata.xml").unwrap();
    assert_eq!(metadata_file.source, Some("metadata".to_string()));
    
    let zip_file = metadata.files.iter().find(|f| f.name == "archive.zip").unwrap();
    assert_eq!(zip_file.format, Some("ZIP".to_string()));
    assert_eq!(zip_file.size, Some("10485760".to_string()));
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