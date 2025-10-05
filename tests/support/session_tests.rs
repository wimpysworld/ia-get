//! Session Management Support Layer Tests
//!
//! Tests for download session functionality including session creation,
//! persistence, file status tracking, and progress monitoring.

use ia_get::metadata_storage::{
    ArchiveFile, ArchiveMetadata, DownloadConfig, DownloadSession, DownloadState,
    generate_session_filename,
};
use std::time::{SystemTime, UNIX_EPOCH};

fn create_test_metadata() -> ArchiveMetadata {
    ArchiveMetadata {
        created: 1234567890,
        d1: "ia801234.us.archive.org".to_string(),
        d2: "ia901234.us.archive.org".to_string(),
        dir: "/12/items/test-archive".to_string(),
        files: vec![
            ArchiveFile {
                name: "test-file.txt".to_string(),
                source: "original".to_string(),
                format: Some("Text".to_string()),
                mtime: Some(1234567890),
                size: Some(1024),
                md5: Some("abcd1234".to_string()),
                crc32: None,
                sha1: None,
                btih: None,
                summation: None,
                original: None,
                rotation: None,
            },
            ArchiveFile {
                name: "image.jpg".to_string(),
                source: "original".to_string(),
                format: Some("JPEG".to_string()),
                mtime: Some(1234567890),
                size: Some(2048),
                md5: Some("efgh5678".to_string()),
                crc32: None,
                sha1: None,
                btih: None,
                summation: None,
                original: None,
                rotation: None,
            },
        ],
        files_count: 2,
        item_last_updated: 1234567890,
        item_size: 3072,
        metadata: serde_json::json!({
            "title": "Test Archive",
            "description": "A test archive for testing purposes"
        }),
        server: "ia801234.us.archive.org".to_string(),
        uniq: 123456789,
        workable_servers: vec![
            "ia801234.us.archive.org".to_string(),
            "ia901234.us.archive.org".to_string(),
        ],
        reviews: vec![],
    }
}

fn create_test_config() -> DownloadConfig {
    DownloadConfig {
        output_dir: "/tmp/downloads".to_string(),
        max_concurrent: 4,
        format_filters: vec!["txt".to_string(), "jpg".to_string()],
        min_size: Some(100),
        max_size: Some(10000),
        verify_md5: true,
        preserve_mtime: true,
        user_agent: "ia-get/1.6.0 test".to_string(),
        enable_compression: true,
        auto_decompress: false,
        decompress_formats: vec!["gz".to_string(), "zip".to_string()],
    }
}

#[test]
fn test_session_creation() {
    let metadata = create_test_metadata();
    let config = create_test_config();
    let requested_files = vec!["test-file.txt".to_string(), "image.jpg".to_string()];

    let session = DownloadSession::new(
        "https://archive.org/details/test-archive".to_string(),
        "test-archive".to_string(),
        metadata.clone(),
        config.clone(),
        requested_files.clone(),
    );

    assert_eq!(
        session.original_url,
        "https://archive.org/details/test-archive"
    );
    assert_eq!(session.identifier, "test-archive");
    assert_eq!(session.archive_metadata.files.len(), 2);
    assert_eq!(session.download_config.max_concurrent, 4);
    assert_eq!(session.requested_files.len(), 2);
    assert_eq!(session.file_status.len(), 2);

    // Check that all files start as pending
    for file_name in &requested_files {
        let status = session.file_status.get(file_name).unwrap();
        assert_eq!(status.status, DownloadState::Pending);
        assert_eq!(status.bytes_downloaded, 0);
        assert!(status.started_at.is_none());
        assert!(status.completed_at.is_none());
    }
}

#[test]
fn test_session_file_status_updates() {
    let metadata = create_test_metadata();
    let config = create_test_config();
    let requested_files = vec!["test-file.txt".to_string()];

    let mut session = DownloadSession::new(
        "https://archive.org/details/test-archive".to_string(),
        "test-archive".to_string(),
        metadata,
        config,
        requested_files,
    );

    // Update file status to in progress
    session.update_file_status("test-file.txt", DownloadState::InProgress);
    let status = session.file_status.get("test-file.txt").unwrap();
    assert_eq!(status.status, DownloadState::InProgress);

    // Update to completed
    session.update_file_status("test-file.txt", DownloadState::Completed);
    let status = session.file_status.get("test-file.txt").unwrap();
    assert_eq!(status.status, DownloadState::Completed);

    // Update to failed
    session.update_file_status("test-file.txt", DownloadState::Failed);
    let status = session.file_status.get("test-file.txt").unwrap();
    assert_eq!(status.status, DownloadState::Failed);
}

#[test]
fn test_session_get_pending_files() {
    let metadata = create_test_metadata();
    let config = create_test_config();
    let requested_files = vec!["test-file.txt".to_string(), "image.jpg".to_string()];

    let mut session = DownloadSession::new(
        "https://archive.org/details/test-archive".to_string(),
        "test-archive".to_string(),
        metadata,
        config,
        requested_files,
    );

    // Initially all files should be pending
    let pending = session.get_pending_files();
    assert_eq!(pending.len(), 2);
    assert!(pending.contains(&"test-file.txt"));
    assert!(pending.contains(&"image.jpg"));

    // Update one file to completed
    session.update_file_status("test-file.txt", DownloadState::Completed);
    let pending = session.get_pending_files();
    assert_eq!(pending.len(), 1);
    assert!(pending.contains(&"image.jpg"));
    assert!(!pending.contains(&"test-file.txt"));

    // Update the other file to in progress
    session.update_file_status("image.jpg", DownloadState::InProgress);
    let pending = session.get_pending_files();
    assert_eq!(pending.len(), 0);
}

#[test]
fn test_session_progress_summary() {
    let metadata = create_test_metadata();
    let config = create_test_config();
    let requested_files = vec!["test-file.txt".to_string(), "image.jpg".to_string()];

    let mut session = DownloadSession::new(
        "https://archive.org/details/test-archive".to_string(),
        "test-archive".to_string(),
        metadata,
        config,
        requested_files,
    );

    let progress = session.get_progress_summary();
    assert_eq!(progress.total_files, 2);
    assert_eq!(progress.completed_files, 0);
    assert_eq!(progress.failed_files, 0);
    assert_eq!(progress.total_bytes, 3072); // 1024 + 2048

    // Complete one file
    session.update_file_status("test-file.txt", DownloadState::Completed);
    let progress = session.get_progress_summary();
    assert_eq!(progress.completed_files, 1);
    assert_eq!(progress.failed_files, 0);

    // Fail the other file
    session.update_file_status("image.jpg", DownloadState::Failed);
    let progress = session.get_progress_summary();
    assert_eq!(progress.completed_files, 1);
    assert_eq!(progress.failed_files, 1);
}

#[test]
fn test_generate_session_filename() {
    let filename = generate_session_filename("test-archive");
    assert!(filename.starts_with("ia-get-session-test-archive-"));
    assert!(filename.ends_with(".json"));

    // Test with problematic identifier
    let filename = generate_session_filename("test/archive?with&special*chars");
    assert!(filename.starts_with("ia-get-session-"));
    assert!(filename.contains("test"));
    assert!(filename.contains("archive"));
    assert!(!filename.contains("/"));
    assert!(!filename.contains("?"));
    assert!(!filename.contains("&"));
    assert!(!filename.contains("*"));
    assert!(filename.ends_with(".json"));
}

#[test]
fn test_session_filename_consistency() {
    let identifier = "test-archive-123";
    let filename1 = generate_session_filename(identifier);
    let filename2 = generate_session_filename(identifier);

    // Each session should have a unique filename to prevent collisions
    // even with the same identifier
    assert_ne!(filename1, filename2);

    // But both should have the same prefix pattern
    assert!(filename1.starts_with("ia-get-session-test-archive-123-"));
    assert!(filename2.starts_with("ia-get-session-test-archive-123-"));
    assert!(filename1.ends_with(".json"));
    assert!(filename2.ends_with(".json"));
}

#[test]
fn test_session_filename_uniqueness() {
    let filename1 = generate_session_filename("archive-1");
    let filename2 = generate_session_filename("archive-2");

    // Different identifiers should produce different filenames
    assert_ne!(filename1, filename2);
}

#[test]
fn test_session_timestamps() {
    let metadata = create_test_metadata();
    let config = create_test_config();
    let requested_files = vec!["test-file.txt".to_string()];

    let before_creation = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let session = DownloadSession::new(
        "https://archive.org/details/test-archive".to_string(),
        "test-archive".to_string(),
        metadata,
        config,
        requested_files,
    );

    let after_creation = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    // Session timestamps should be within reasonable range
    assert!(session.session_start >= before_creation);
    assert!(session.session_start <= after_creation);
    assert!(session.last_updated >= before_creation);
    assert!(session.last_updated <= after_creation);

    // Initially session_start and last_updated should be the same
    assert_eq!(session.session_start, session.last_updated);
}

#[test]
fn test_session_file_status_structure() {
    let metadata = create_test_metadata();
    let config = create_test_config();
    let requested_files = vec!["test-file.txt".to_string()];

    let session = DownloadSession::new(
        "https://archive.org/details/test-archive".to_string(),
        "test-archive".to_string(),
        metadata,
        config,
        requested_files,
    );

    let file_status = session.file_status.get("test-file.txt").unwrap();

    // Check file status structure
    assert_eq!(file_status.file_info.name, "test-file.txt");
    assert_eq!(file_status.file_info.source, "original");
    assert_eq!(file_status.file_info.size, Some(1024));
    assert_eq!(file_status.status, DownloadState::Pending);
    assert_eq!(file_status.bytes_downloaded, 0);
    assert!(file_status.started_at.is_none());
    assert!(file_status.completed_at.is_none());
    assert!(file_status.error_message.is_none());
    assert_eq!(file_status.retry_count, 0);
    assert!(file_status.server_used.is_none());
    assert!(file_status.local_path.contains("test-file.txt"));
}

#[test]
fn test_session_with_no_requested_files() {
    let metadata = create_test_metadata();
    let config = create_test_config();
    let requested_files = vec![];

    let session = DownloadSession::new(
        "https://archive.org/details/test-archive".to_string(),
        "test-archive".to_string(),
        metadata,
        config,
        requested_files,
    );

    assert_eq!(session.requested_files.len(), 0);
    assert_eq!(session.file_status.len(), 0);

    let pending = session.get_pending_files();
    assert_eq!(pending.len(), 0);

    let progress = session.get_progress_summary();
    assert_eq!(progress.total_files, 0);
    assert_eq!(progress.completed_files, 0);
    assert_eq!(progress.failed_files, 0);
}

#[test]
fn test_session_with_nonexistent_requested_files() {
    let metadata = create_test_metadata();
    let config = create_test_config();
    let requested_files = vec![
        "test-file.txt".to_string(),
        "nonexistent-file.txt".to_string(),
    ];

    let session = DownloadSession::new(
        "https://archive.org/details/test-archive".to_string(),
        "test-archive".to_string(),
        metadata,
        config,
        requested_files.clone(),
    );

    assert_eq!(session.requested_files.len(), 2);
    // Only files that exist in metadata should have status tracking
    assert_eq!(session.file_status.len(), 1);
    assert!(session.file_status.contains_key("test-file.txt"));
    assert!(!session.file_status.contains_key("nonexistent-file.txt"));
}
