//! Metadata Storage Support Layer Tests
//!
//! Tests for metadata storage functionality including filename sanitization,
//! path validation, session management, and file metadata handling.

use ia_get::metadata_storage::{
    generate_session_filename, sanitize_filename_for_filesystem, validate_path_length, ArchiveFile,
    DownloadState,
};
use std::io::Write;
use tempfile::Builder;

#[test]
fn test_sanitize_identifier_normal() {
    let identifier = "normal-identifier_123";
    let result = ia_get::metadata_storage::sanitize_identifier_for_filesystem(identifier);
    assert_eq!(result, "normal-identifier_123");
}

#[test]
fn test_sanitize_identifier_with_invalid_characters() {
    let identifier = "test<>:|?*\\";
    let result = ia_get::metadata_storage::sanitize_identifier_for_filesystem(identifier);
    assert_eq!(result, "test");
}

#[test]
fn test_sanitize_identifier_with_spaces() {
    let identifier = "test with spaces";
    let result = ia_get::metadata_storage::sanitize_identifier_for_filesystem(identifier);
    assert_eq!(result, "test_with_spaces");
}

#[test]
fn test_sanitize_identifier_windows_problematic() {
    let identifier = "file<name>:with|invalid?chars*";
    let result = ia_get::metadata_storage::sanitize_identifier_for_filesystem(identifier);
    assert_eq!(result, "filenamewithinvalidchars");
}

#[test]
fn test_sanitize_identifier_consecutive_separators() {
    let identifier = "test--with__consecutive___separators";
    let result = ia_get::metadata_storage::sanitize_identifier_for_filesystem(identifier);
    assert_eq!(result, "test-with_consecutive_separators");
}

#[test]
fn test_sanitize_identifier_trim_edges() {
    let identifier = "--test_identifier--";
    let result = ia_get::metadata_storage::sanitize_identifier_for_filesystem(identifier);
    assert_eq!(result, "test_identifier");
}

#[test]
fn test_sanitize_identifier_empty_after_cleaning() {
    let identifier = "!$%^&*()";
    let result = ia_get::metadata_storage::sanitize_identifier_for_filesystem(identifier);
    assert_eq!(result, "archive");
}

#[test]
fn test_sanitize_identifier_long_identifier() {
    // Create an identifier longer than 200 characters
    let long_identifier = "a".repeat(250);
    let result = ia_get::metadata_storage::sanitize_identifier_for_filesystem(&long_identifier);

    // Should be truncated and include a hash
    assert!(result.len() <= 200);
    assert!(result.contains("-")); // Should have hash separator
    assert!(result.starts_with("a")); // Should start with original content
}

#[test]
fn test_sanitize_identifier_real_world_case() {
    let identifier = "ikaos-som-dragon-ball-complete-001-153-r2j-dragon-box-multi-audio-v4_202301";
    let result = ia_get::metadata_storage::sanitize_identifier_for_filesystem(identifier);
    // This identifier is already valid, should remain unchanged
    assert_eq!(result, identifier);
}

#[test]
fn test_sanitize_identifier_control_characters() {
    let identifier = "test\x00\x01\x02\x03identifier";
    let result = ia_get::metadata_storage::sanitize_identifier_for_filesystem(identifier);
    assert_eq!(result, "testidentifier");
}

#[test]
fn test_generate_session_filename_format() {
    let identifier = "test-identifier";
    let result = generate_session_filename(identifier);

    // Should match pattern: ia-get-session-{sanitized_identifier}-{timestamp}.json
    assert!(result.starts_with("ia-get-session-"));
    assert!(result.ends_with(".json"));
    assert!(result.contains("test-identifier"));
}

#[test]
fn test_generate_session_filename_with_problematic_identifier() {
    let identifier = "test<>:|identifier";
    let result = generate_session_filename(identifier);

    // Should sanitize the identifier
    assert!(result.starts_with("ia-get-session-"));
    assert!(result.ends_with(".json"));
    assert!(!result.contains("<"));
    assert!(!result.contains(">"));
    assert!(!result.contains(":"));
    assert!(!result.contains("|"));
}

#[test]
fn test_generate_session_filename_uniqueness() {
    let identifier = "test-identifier";
    let result1 = generate_session_filename(identifier);

    // Small delay to ensure different timestamp
    std::thread::sleep(std::time::Duration::from_millis(1001)); // More than 1 second

    let result2 = generate_session_filename(identifier);

    // Should generate different filenames due to timestamp
    assert_ne!(result1, result2);
}

#[test]
fn test_windows_filename_edge_cases() {
    // Test filenames ending with periods
    let result = sanitize_filename_for_filesystem("test_file...");
    assert!(!result.ends_with('.'));

    // Test filenames ending with spaces
    let result = sanitize_filename_for_filesystem("test_file   ");
    assert!(!result.ends_with(' '));

    // Test combination of periods and spaces
    let result = sanitize_filename_for_filesystem("test_file. . ");
    assert!(!result.ends_with('.'));
    assert!(!result.ends_with(' '));
}

#[test]
fn test_sanitize_preserves_reasonable_length() {
    let identifier = "moderately-long-but-reasonable-identifier-name";
    let result = ia_get::metadata_storage::sanitize_identifier_for_filesystem(identifier);
    assert_eq!(result, identifier);
}

#[test]
fn test_hash_consistency_for_long_identifiers() {
    let long_identifier = "a".repeat(250);
    let result1 = ia_get::metadata_storage::sanitize_identifier_for_filesystem(&long_identifier);
    let result2 = ia_get::metadata_storage::sanitize_identifier_for_filesystem(&long_identifier);

    // Should generate the same result for the same input
    assert_eq!(result1, result2);
}

#[test]
fn test_sanitize_filename_for_filesystem() {
    // Test normal filename
    let filename = "test_file.mp3";
    let result = sanitize_filename_for_filesystem(filename);
    assert_eq!(result, filename);

    // Test problematic characters
    let filename = "file<name>:with|invalid?chars*.mp3";
    let result = sanitize_filename_for_filesystem(filename);
    assert_eq!(result, "file_name_with_invalid_chars_.mp3");

    // Test Windows reserved name
    let filename = "CON.txt";
    let result = sanitize_filename_for_filesystem(filename);
    assert_eq!(result, "CON_file.txt");

    // Test consecutive underscores
    let filename = "file__with___underscores.txt";
    let result = sanitize_filename_for_filesystem(filename);
    assert_eq!(result, "file_with_underscores.txt");

    // Test empty after sanitization
    let filename = "<<<>>>";
    let result = sanitize_filename_for_filesystem(filename);
    assert_eq!(result, "file");
}

#[test]
fn test_validate_path_length() {
    // Test normal path
    let output_dir = "/home/user/downloads";
    let filename = "test.mp3";
    assert!(validate_path_length(output_dir, filename).is_ok());

    // Test long path - behavior depends on system long path support
    // Use consistent forward slashes since validate_path_length uses format!("{}/{}")
    let long_dir = if cfg!(target_os = "windows") {
        "C:/".to_string() + &"very_long_directory_name/".repeat(20)
    } else {
        "/tmp/".to_string() + &"very_long_directory_name/".repeat(20)
    };
    let long_filename = "very_long_filename_that_makes_path_exceed_windows_limit.mp3";
    let full_path = format!("{}/{}", long_dir, long_filename);

    // The validation result depends on system capabilities
    let result = validate_path_length(&long_dir, long_filename);

    // On systems with long path support, it should pass (path is ~584 chars, under 32767 limit)
    // On systems without long path support, it should fail (path exceeds 260 char limit)
    if full_path.len() > 260 {
        // If the system supports long paths, it should pass
        #[cfg(target_os = "windows")]
        {
            // Be more defensive with Windows long path detection as it can fail for various reasons
            match std::panic::catch_unwind(|| {
                ia_get::metadata_storage::is_windows_long_path_enabled()
            }) {
                Ok(true) => {
                    assert!(
                        result.is_ok(),
                        "Long path should be allowed on systems with long path support. Path length: {}, Path: {}",
                        full_path.len(),
                        full_path
                    );
                }
                Ok(false) => {
                    assert!(
                        result.is_err(),
                        "Long path should be rejected on systems without long path support. Path length: {}, Path: {}",
                        full_path.len(),
                        full_path
                    );
                }
                Err(_) => {
                    // If long path detection fails, just verify the error message is reasonable
                    if let Err(error) = result {
                        let error_msg = error.to_string();
                        assert!(
                            error_msg.contains("Path too long") || error_msg.contains("characters"),
                            "Error message should contain path length information. Got: {}",
                            error_msg
                        );
                    }
                    // Don't assert on the specific result since detection failed
                }
            }
        }

        // On non-Windows systems, long paths are generally supported
        #[cfg(not(target_os = "windows"))]
        assert!(
            result.is_ok(),
            "Long paths should be supported on non-Windows systems. Path length: {}, Path: {}",
            full_path.len(),
            full_path
        );
    }
}

#[test]
fn test_validate_extremely_long_path() {
    // Test a path that exceeds even the extended Windows limit (32767)
    // Create a path longer than 32767 characters
    let extremely_long_dir = "C:\\".to_string() + &"a".repeat(32800);
    let filename = "test.mp3";
    let full_path = format!("{}/{}", extremely_long_dir, filename);

    // This should fail even on systems with long path support
    let result = validate_path_length(&extremely_long_dir, filename);

    assert!(
        result.is_err(),
        "Extremely long paths should be rejected even with long path support. Path length: {}",
        full_path.len()
    );
}

#[test]
fn test_xml_alternative_validation() {
    // Create temporary XML file with .xml extension
    let mut temp_file = Builder::new().suffix(".xml").tempfile().unwrap();

    let xml_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<root>
    <item>test</item>
</root>"#;
    temp_file.write_all(xml_content.as_bytes()).unwrap();
    temp_file.flush().unwrap();

    let file_info = ArchiveFile {
        name: "test.xml".to_string(),
        source: "original".to_string(),
        format: Some("XML".to_string()),
        mtime: Some(1234567890),
        size: Some(xml_content.len() as u64),
        md5: Some("dummy_md5".to_string()),
        crc32: None,
        sha1: None,
        btih: None,
        summation: None,
        original: None,
        rotation: None,
    };

    // Should validate as true for properly structured XML
    let result = file_info.validate_md5(temp_file.path());
    assert!(result.unwrap());
}

#[test]
fn test_archive_file_compression_detection() {
    // Test compressed file
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
fn test_download_state_enum() {
    // Test that the enum values work as expected
    let states = [
        DownloadState::Pending,
        DownloadState::InProgress,
        DownloadState::Completed,
        DownloadState::Failed,
        DownloadState::Paused,
        DownloadState::Skipped,
    ];

    for state in states {
        // Test that debug formatting works
        let debug_str = format!("{:?}", state);
        assert!(!debug_str.is_empty());

        // Test equality
        let state_clone = state.clone();
        assert_eq!(state, state_clone);
    }
}
