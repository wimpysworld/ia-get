//! Session Management Testing Module
//!
//! Tests for download session functionality including:
//! - Session creation and initialization
//! - Session persistence and resumption
//! - Progress tracking and state management
//! - File status updates and monitoring

use ia_get::metadata_storage::{
    ArchiveFile, ArchiveMetadata, DownloadConfig, DownloadProgress, DownloadSession, DownloadState,
    FileDownloadStatus,
};
use std::collections::HashMap;
use tempfile::TempDir;

/// Create test archive metadata for session testing
fn create_test_metadata() -> ArchiveMetadata {
    ArchiveMetadata {
        created: Some(1234567890),
        d1: Some("archive.org".to_string()),
        dir: "/test-session".to_string(),
        files: vec![
            ArchiveFile {
                name: "file1.pdf".to_string(),
                source: Some("original".to_string()),
                mtime: Some("1234567890".to_string()),
                size: Some("1048576".to_string()),
                md5: Some("hash1".to_string()),
                crc32: None,
                sha1: None,
                format: Some("PDF".to_string()),
                width: None,
                height: None,
                length: None,
            },
            ArchiveFile {
                name: "file2.txt".to_string(),
                source: Some("original".to_string()),
                mtime: Some("1234567890".to_string()),
                size: Some("2048".to_string()),
                md5: Some("hash2".to_string()),
                crc32: None,
                sha1: None,
                format: Some("Text".to_string()),
                width: None,
                height: None,
                length: None,
            },
            ArchiveFile {
                name: "file3.jpg".to_string(),
                source: Some("original".to_string()),
                mtime: Some("1234567890".to_string()),
                size: Some("524288".to_string()),
                md5: Some("hash3".to_string()),
                crc32: None,
                sha1: None,
                format: Some("JPEG".to_string()),
                width: Some("1920".to_string()),
                height: Some("1080".to_string()),
                length: None,
            },
        ],
        metadata: None,
        misc: None,
        reviews: None,
        server: "ia801234.us.archive.org".to_string(),
        uniq: Some(1234567890),
        workable_servers: Some(vec!["ia801234.us.archive.org".to_string()]),
    }
}

/// Create test download configuration
fn create_test_config() -> DownloadConfig {
    DownloadConfig {
        output_dir: "test-downloads".to_string(),
        concurrent_downloads: 3,
        max_retries: 3,
        include_extensions: vec!["pdf".to_string(), "txt".to_string()],
        exclude_extensions: vec!["log".to_string()],
        max_file_size: Some(2097152), // 2MB
        resume: true,
        compress: false,
        decompress: false,
        decompress_formats: vec![],
        dry_run: false,
        verbose: true,
        log_hash_errors: false,
    }
}

/// Test session creation and initialization
#[test]
fn test_session_creation() {
    let metadata = create_test_metadata();
    let config = create_test_config();
    let requested_files = vec!["file1.pdf".to_string(), "file2.txt".to_string()];

    let session = DownloadSession::new(
        "https://archive.org/details/test-session".to_string(),
        "test-session".to_string(),
        metadata.clone(),
        config.clone(),
        requested_files.clone(),
    );

    // Verify basic session properties
    assert_eq!(
        session.original_url,
        "https://archive.org/details/test-session"
    );
    assert_eq!(session.identifier, "test-session");
    assert_eq!(session.archive_metadata.files.len(), 3);
    assert_eq!(session.download_config.output_dir, "test-downloads");
    assert_eq!(session.requested_files, requested_files);

    // Verify file status initialization
    assert_eq!(session.file_status.len(), 2);

    let file1_status = session.file_status.get("file1.pdf").unwrap();
    assert_eq!(file1_status.status, DownloadState::Pending);
    assert_eq!(file1_status.bytes_downloaded, 0);
    assert_eq!(file1_status.retry_count, 0);
    assert!(file1_status.started_at.is_none());
    assert!(file1_status.completed_at.is_none());
    assert!(file1_status.error_message.is_none());
    assert_eq!(file1_status.local_path, "test-downloads/file1.pdf");

    let file2_status = session.file_status.get("file2.txt").unwrap();
    assert_eq!(file2_status.status, DownloadState::Pending);
    assert_eq!(file2_status.local_path, "test-downloads/file2.txt");

    // Verify timing
    assert!(session.session_start > 0);
    assert!(session.last_updated > 0);
    assert_eq!(session.session_start, session.last_updated);
}

/// Test file status updates
#[test]
fn test_file_status_updates() {
    let metadata = create_test_metadata();
    let config = create_test_config();
    let requested_files = vec!["file1.pdf".to_string(), "file2.txt".to_string()];

    let mut session = DownloadSession::new(
        "https://archive.org/details/test-session".to_string(),
        "test-session".to_string(),
        metadata,
        config,
        requested_files,
    );

    // Test status update to InProgress
    session.update_file_status("file1.pdf", DownloadState::InProgress);
    let status = session.file_status.get("file1.pdf").unwrap();
    assert_eq!(status.status, DownloadState::InProgress);

    // Test status update to Completed
    session.update_file_status("file1.pdf", DownloadState::Completed);
    let status = session.file_status.get("file1.pdf").unwrap();
    assert_eq!(status.status, DownloadState::Completed);

    // Test status update to Failed
    session.update_file_status("file2.txt", DownloadState::Failed);
    let status = session.file_status.get("file2.txt").unwrap();
    assert_eq!(status.status, DownloadState::Failed);

    // Test updating non-existent file (should not panic)
    session.update_file_status("nonexistent.file", DownloadState::Completed);
    assert!(!session.file_status.contains_key("nonexistent.file"));
}

/// Test getting pending files
#[test]
fn test_get_pending_files() {
    let metadata = create_test_metadata();
    let config = create_test_config();
    let requested_files = vec![
        "file1.pdf".to_string(),
        "file2.txt".to_string(),
        "file3.jpg".to_string(),
    ];

    let mut session = DownloadSession::new(
        "https://archive.org/details/test-session".to_string(),
        "test-session".to_string(),
        metadata,
        config,
        requested_files,
    );

    // Initially all files should be pending
    let pending = session.get_pending_files();
    assert_eq!(pending.len(), 3);
    assert!(pending.contains(&"file1.pdf"));
    assert!(pending.contains(&"file2.txt"));
    assert!(pending.contains(&"file3.jpg"));

    // Complete one file
    session.update_file_status("file1.pdf", DownloadState::Completed);
    let pending = session.get_pending_files();
    assert_eq!(pending.len(), 2);
    assert!(!pending.contains(&"file1.pdf"));
    assert!(pending.contains(&"file2.txt"));
    assert!(pending.contains(&"file3.jpg"));

    // Start another file (InProgress should not be pending)
    session.update_file_status("file2.txt", DownloadState::InProgress);
    let pending = session.get_pending_files();
    assert_eq!(pending.len(), 1);
    assert!(pending.contains(&"file3.jpg"));

    // Fail the in-progress file
    session.update_file_status("file2.txt", DownloadState::Failed);
    let pending = session.get_pending_files();
    assert_eq!(pending.len(), 1);
    assert!(pending.contains(&"file3.jpg"));
}

/// Test progress summary calculation
#[test]
fn test_progress_summary() {
    let metadata = create_test_metadata();
    let config = create_test_config();
    let requested_files = vec![
        "file1.pdf".to_string(),
        "file2.txt".to_string(),
        "file3.jpg".to_string(),
    ];

    let mut session = DownloadSession::new(
        "https://archive.org/details/test-session".to_string(),
        "test-session".to_string(),
        metadata,
        config,
        requested_files,
    );

    // Initial progress
    let progress = session.get_progress_summary();
    assert_eq!(progress.total_files, 3);
    assert_eq!(progress.completed_files, 0);
    assert_eq!(progress.failed_files, 0);
    assert_eq!(progress.in_progress_files, 0);
    assert_eq!(progress.total_bytes, 1048576 + 2048 + 524288); // Sum of file sizes
    assert_eq!(progress.downloaded_bytes, 0);

    // Start one download
    session.update_file_status("file1.pdf", DownloadState::InProgress);
    let progress = session.get_progress_summary();
    assert_eq!(progress.in_progress_files, 1);

    // Complete one download
    session.update_file_status("file1.pdf", DownloadState::Completed);
    let progress = session.get_progress_summary();
    assert_eq!(progress.completed_files, 1);
    assert_eq!(progress.in_progress_files, 0);

    // Fail one download
    session.update_file_status("file2.txt", DownloadState::Failed);
    let progress = session.get_progress_summary();
    assert_eq!(progress.failed_files, 1);

    // Complete remaining
    session.update_file_status("file3.jpg", DownloadState::Completed);
    let progress = session.get_progress_summary();
    assert_eq!(progress.completed_files, 2);
    assert_eq!(progress.failed_files, 1);
    assert_eq!(progress.in_progress_files, 0);
}

/// Test session persistence (save/load)
#[test]
fn test_session_persistence() {
    let temp_dir = TempDir::new().unwrap();
    let session_file = temp_dir.path().join("session.json");

    let metadata = create_test_metadata();
    let config = create_test_config();
    let requested_files = vec!["file1.pdf".to_string(), "file2.txt".to_string()];

    let mut original_session = DownloadSession::new(
        "https://archive.org/details/test-session".to_string(),
        "test-session".to_string(),
        metadata,
        config,
        requested_files,
    );

    // Update some file statuses
    original_session.update_file_status("file1.pdf", DownloadState::InProgress);
    original_session.update_file_status("file2.txt", DownloadState::Completed);

    // Save session
    let save_result = original_session.save_to_file(&session_file);
    assert!(save_result.is_ok());
    assert!(session_file.exists());

    // Load session
    let load_result = DownloadSession::load_from_file(&session_file);
    assert!(load_result.is_ok());

    let loaded_session = load_result.unwrap();

    // Verify loaded session matches original
    assert_eq!(loaded_session.original_url, original_session.original_url);
    assert_eq!(loaded_session.identifier, original_session.identifier);
    assert_eq!(
        loaded_session.requested_files,
        original_session.requested_files
    );
    assert_eq!(
        loaded_session.file_status.len(),
        original_session.file_status.len()
    );

    // Verify file statuses were preserved
    let file1_status = loaded_session.file_status.get("file1.pdf").unwrap();
    assert_eq!(file1_status.status, DownloadState::InProgress);

    let file2_status = loaded_session.file_status.get("file2.txt").unwrap();
    assert_eq!(file2_status.status, DownloadState::Completed);
}

/// Test session persistence error handling
#[test]
fn test_session_persistence_errors() {
    // Test loading non-existent file
    let result = DownloadSession::load_from_file("nonexistent.json");
    assert!(result.is_err());

    // Test loading invalid JSON
    let temp_dir = TempDir::new().unwrap();
    let invalid_file = temp_dir.path().join("invalid.json");
    std::fs::write(&invalid_file, "invalid json content").unwrap();

    let result = DownloadSession::load_from_file(&invalid_file);
    assert!(result.is_err());

    // Test saving to invalid path
    let metadata = create_test_metadata();
    let config = create_test_config();
    let session = DownloadSession::new(
        "https://archive.org/details/test".to_string(),
        "test".to_string(),
        metadata,
        config,
        vec!["file1.pdf".to_string()],
    );

    let invalid_path = "/invalid/path/that/does/not/exist/session.json";
    let result = session.save_to_file(invalid_path);
    assert!(result.is_err());
}

/// Test session resumption workflow
#[test]
fn test_session_resumption() {
    let temp_dir = TempDir::new().unwrap();
    let session_file = temp_dir.path().join("resume_session.json");

    let metadata = create_test_metadata();
    let config = create_test_config();
    let requested_files = vec![
        "file1.pdf".to_string(),
        "file2.txt".to_string(),
        "file3.jpg".to_string(),
    ];

    // Create initial session
    let mut session = DownloadSession::new(
        "https://archive.org/details/test-session".to_string(),
        "test-session".to_string(),
        metadata,
        config,
        requested_files,
    );

    // Simulate partial download completion
    session.update_file_status("file1.pdf", DownloadState::Completed);
    session.update_file_status("file2.txt", DownloadState::Failed);
    // file3.jpg remains Pending

    // Save session state
    session.save_to_file(&session_file).unwrap();

    // Resume session
    let resumed_session = DownloadSession::load_from_file(&session_file).unwrap();

    // Verify resumption state
    let pending_files = resumed_session.get_pending_files();
    assert_eq!(pending_files.len(), 1);
    assert!(pending_files.contains(&"file3.jpg"));

    let progress = resumed_session.get_progress_summary();
    assert_eq!(progress.completed_files, 1);
    assert_eq!(progress.failed_files, 1);
    assert_eq!(progress.total_files, 3);

    // Verify specific file states
    let file1_status = resumed_session.file_status.get("file1.pdf").unwrap();
    assert_eq!(file1_status.status, DownloadState::Completed);

    let file2_status = resumed_session.file_status.get("file2.txt").unwrap();
    assert_eq!(file2_status.status, DownloadState::Failed);

    let file3_status = resumed_session.file_status.get("file3.jpg").unwrap();
    assert_eq!(file3_status.status, DownloadState::Pending);
}

/// Test download config validation and usage
#[test]
fn test_download_config() {
    let config = create_test_config();

    // Verify all configuration options
    assert_eq!(config.output_dir, "test-downloads");
    assert_eq!(config.concurrent_downloads, 3);
    assert_eq!(config.max_retries, 3);
    assert_eq!(config.include_extensions, vec!["pdf", "txt"]);
    assert_eq!(config.exclude_extensions, vec!["log"]);
    assert_eq!(config.max_file_size, Some(2097152));
    assert!(config.resume);
    assert!(!config.compress);
    assert!(!config.decompress);
    assert!(config.decompress_formats.is_empty());
    assert!(!config.dry_run);
    assert!(config.verbose);
    assert!(!config.log_hash_errors);
}

/// Test file download status structure
#[test]
fn test_file_download_status() {
    let metadata = create_test_metadata();
    let config = create_test_config();
    let session = DownloadSession::new(
        "https://archive.org/details/test".to_string(),
        "test".to_string(),
        metadata,
        config,
        vec!["file1.pdf".to_string()],
    );

    let status = session.file_status.get("file1.pdf").unwrap();

    // Verify file info is copied correctly
    assert_eq!(status.file_info.name, "file1.pdf");
    assert_eq!(status.file_info.size, Some("1048576".to_string()));
    assert_eq!(status.file_info.md5, Some("hash1".to_string()));
    assert_eq!(status.file_info.format, Some("PDF".to_string()));

    // Verify initial status
    assert_eq!(status.status, DownloadState::Pending);
    assert_eq!(status.bytes_downloaded, 0);
    assert_eq!(status.retry_count, 0);
    assert!(status.started_at.is_none());
    assert!(status.completed_at.is_none());
    assert!(status.error_message.is_none());
    assert!(status.server_used.is_none());
    assert_eq!(status.local_path, "test-downloads/file1.pdf");
}

/// Test download state enumeration
#[test]
fn test_download_state_enum() {
    // Test state equality
    assert_eq!(DownloadState::Pending, DownloadState::Pending);
    assert_ne!(DownloadState::Pending, DownloadState::InProgress);

    // Test state creation
    let states = [
        DownloadState::Pending,
        DownloadState::InProgress,
        DownloadState::Completed,
        DownloadState::Failed,
    ];

    for state in &states {
        // Each state should be equal to itself
        assert_eq!(*state, *state);
    }

    // Test state transitions are possible
    let mut session = {
        let metadata = create_test_metadata();
        let config = create_test_config();
        DownloadSession::new(
            "https://archive.org/details/test".to_string(),
            "test".to_string(),
            metadata,
            config,
            vec!["file1.pdf".to_string()],
        )
    };

    // Pending → InProgress
    session.update_file_status("file1.pdf", DownloadState::InProgress);
    let status = session.file_status.get("file1.pdf").unwrap();
    assert_eq!(status.status, DownloadState::InProgress);

    // InProgress → Completed
    session.update_file_status("file1.pdf", DownloadState::Completed);
    let status = session.file_status.get("file1.pdf").unwrap();
    assert_eq!(status.status, DownloadState::Completed);

    // Can transition from any state to Failed
    session.update_file_status("file1.pdf", DownloadState::Failed);
    let status = session.file_status.get("file1.pdf").unwrap();
    assert_eq!(status.status, DownloadState::Failed);
}

/// Test progress calculation edge cases
#[test]
fn test_progress_calculation_edge_cases() {
    let metadata = create_test_metadata();
    let config = create_test_config();

    // Test with empty requested files
    let empty_session = DownloadSession::new(
        "https://archive.org/details/test".to_string(),
        "test".to_string(),
        metadata.clone(),
        config.clone(),
        vec![],
    );

    let progress = empty_session.get_progress_summary();
    assert_eq!(progress.total_files, 0);
    assert_eq!(progress.completed_files, 0);
    assert_eq!(progress.failed_files, 0);
    assert_eq!(progress.in_progress_files, 0);
    assert_eq!(progress.total_bytes, 0);
    assert_eq!(progress.downloaded_bytes, 0);

    // Test with files that have no size information
    let mut modified_metadata = metadata.clone();
    modified_metadata.files[0].size = None; // Remove size from first file

    let session_no_size = DownloadSession::new(
        "https://archive.org/details/test".to_string(),
        "test".to_string(),
        modified_metadata,
        config,
        vec!["file1.pdf".to_string()],
    );

    let progress = session_no_size.get_progress_summary();
    assert_eq!(progress.total_files, 1);
    assert_eq!(progress.total_bytes, 0); // File without size contributes 0 bytes
}
