//! Download Flow Testing Module
//!
//! Comprehensive end-to-end tests for download workflows including:
//! - Complete download flow validation
//! - Step-by-step process verification
//! - User interaction flows
//! - Automatic progression between steps
//! - Error recovery and continuation

use ia_get::{
    cli::{Cli, Commands},
    metadata_storage::{ArchiveMetadata, ArchiveFile, DownloadSession, DownloadConfig, DownloadState},
    url_processing::{validate_and_process_url, extract_identifier_from_url},
    metadata::{get_json_url, parse_archive_metadata},
    filters::{filter_files, parse_size_string},
    error::IaGetError,
};
use clap::Parser;
use std::collections::HashMap;

/// Create test archive metadata for flow testing
fn create_test_archive_metadata() -> ArchiveMetadata {
    ArchiveMetadata {
        created: Some(1234567890),
        d1: Some("archive.org".to_string()),
        dir: "/test-archive".to_string(),
        files: vec![
            ArchiveFile {
                name: "document.pdf".to_string(),
                source: Some("original".to_string()),
                mtime: Some("1234567890".to_string()),
                size: Some("1048576".to_string()),
                md5: Some("abc123".to_string()),
                crc32: None,
                sha1: None,
                format: Some("PDF".to_string()),
                width: None,
                height: None,
                length: None,
            },
            ArchiveFile {
                name: "image.jpg".to_string(),
                source: Some("original".to_string()),
                mtime: Some("1234567890".to_string()),
                size: Some("524288".to_string()),
                md5: Some("def456".to_string()),
                crc32: None,
                sha1: None,
                format: Some("JPEG".to_string()),
                width: Some("1920".to_string()),
                height: Some("1080".to_string()),
                length: None,
            },
            ArchiveFile {
                name: "text.txt".to_string(),
                source: Some("original".to_string()),
                mtime: Some("1234567890".to_string()),
                size: Some("2048".to_string()),
                md5: Some("ghi789".to_string()),
                crc32: None,
                sha1: None,
                format: Some("Text".to_string()),
                width: None,
                height: None,
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

/// Test Step 1: URL Processing and Validation
#[test]
fn test_flow_step1_url_processing() {
    // Test valid Internet Archive URL
    let valid_url = "https://archive.org/details/test-archive";
    let result = validate_and_process_url(valid_url);
    assert!(result.is_ok());
    
    // Test identifier extraction
    let identifier = extract_identifier_from_url(valid_url).unwrap();
    assert_eq!(identifier, "test-archive");
    
    // Test metadata URL generation
    let metadata_url = get_json_url(valid_url);
    assert_eq!(metadata_url, "https://archive.org/metadata/test-archive");
    
    // Test invalid URL rejection
    let invalid_url = "https://example.com/not-archive";
    let result = validate_and_process_url(invalid_url);
    assert!(result.is_err());
    
    // Flow continues to Step 2 if URL is valid
    if result.is_ok() {
        println!("✓ Step 1 complete: URL validated, proceeding to metadata fetch");
    }
}

/// Test Step 2: Metadata Fetching and Parsing
#[test]
fn test_flow_step2_metadata_processing() {
    // Simulate metadata JSON response
    let json_data = r#"{
        "files": [
            {
                "name": "test.pdf",
                "size": "1048576",
                "format": "PDF",
                "md5": "abc123"
            }
        ],
        "server": "ia801234.us.archive.org",
        "dir": "/test"
    }"#;
    
    // Test metadata parsing
    let result = parse_archive_metadata(json_data);
    assert!(result.is_ok());
    
    let metadata = result.unwrap();
    assert_eq!(metadata.files.len(), 1);
    assert_eq!(metadata.files[0].name, "test.pdf");
    assert_eq!(metadata.server, "ia801234.us.archive.org");
    
    // Flow continues to Step 3 if metadata is valid
    println!("✓ Step 2 complete: Metadata parsed, proceeding to file filtering");
}

/// Test Step 3: File Filtering and Selection
#[test]
fn test_flow_step3_file_filtering() {
    let metadata = create_test_archive_metadata();
    
    // Test no filters (all files selected)
    let all_files = filter_files(&metadata.files, &[], &[], None);
    assert_eq!(all_files.len(), 3);
    
    // Test extension filtering
    let include_ext = vec!["pdf".to_string()];
    let pdf_files = filter_files(&metadata.files, &include_ext, &[], None);
    assert_eq!(pdf_files.len(), 1);
    assert_eq!(pdf_files[0].name, "document.pdf");
    
    // Test size filtering
    let max_size = parse_size_string("600KB").unwrap();
    let small_files = filter_files(&metadata.files, &[], &[], Some(max_size));
    assert_eq!(small_files.len(), 2); // image.jpg and text.txt
    
    // Test combined filters
    let include_ext = vec!["jpg".to_string(), "txt".to_string()];
    let max_size = parse_size_string("1MB").unwrap();
    let filtered = filter_files(&metadata.files, &include_ext, &[], Some(max_size));
    assert_eq!(filtered.len(), 2);
    
    // Flow continues to Step 4 if files are selected
    if !filtered.is_empty() {
        println!("✓ Step 3 complete: {} files selected, proceeding to download preparation", filtered.len());
    }
}

/// Test Step 4: Download Session Creation
#[test] 
fn test_flow_step4_session_creation() {
    let metadata = create_test_archive_metadata();
    let config = DownloadConfig {
        output_dir: "downloads".to_string(),
        concurrent_downloads: 3,
        max_retries: 3,
        include_extensions: vec!["pdf".to_string()],
        exclude_extensions: vec![],
        max_file_size: None,
        resume: false,
        compress: false,
        decompress: false,
        decompress_formats: vec![],
        dry_run: false,
        verbose: false,
        log_hash_errors: false,
    };
    
    let requested_files = vec!["document.pdf".to_string()];
    
    // Test session creation
    let session = DownloadSession::new(
        "https://archive.org/details/test-archive".to_string(),
        "test-archive".to_string(),
        metadata,
        config,
        requested_files,
    );
    
    assert_eq!(session.original_url, "https://archive.org/details/test-archive");
    assert_eq!(session.identifier, "test-archive");
    assert_eq!(session.requested_files.len(), 1);
    assert_eq!(session.file_status.len(), 1);
    
    // Verify file status initialization
    let file_status = session.file_status.get("document.pdf").unwrap();
    assert_eq!(file_status.status, DownloadState::Pending);
    assert_eq!(file_status.bytes_downloaded, 0);
    assert_eq!(file_status.retry_count, 0);
    
    // Flow continues to Step 5 for actual downloading
    println!("✓ Step 4 complete: Download session created, proceeding to download execution");
}

/// Test Step 5: Download Progress Tracking
#[test]
fn test_flow_step5_progress_tracking() {
    let metadata = create_test_archive_metadata();
    let config = DownloadConfig {
        output_dir: "downloads".to_string(),
        concurrent_downloads: 3,
        max_retries: 3,
        include_extensions: vec![],
        exclude_extensions: vec![],
        max_file_size: None,
        resume: false,
        compress: false,
        decompress: false,
        decompress_formats: vec![],
        dry_run: false,
        verbose: false,
        log_hash_errors: false,
    };
    
    let requested_files = vec!["document.pdf".to_string(), "text.txt".to_string()];
    let mut session = DownloadSession::new(
        "https://archive.org/details/test-archive".to_string(),
        "test-archive".to_string(),
        metadata,
        config,
        requested_files,
    );
    
    // Test initial progress
    let progress = session.get_progress_summary();
    assert_eq!(progress.total_files, 2);
    assert_eq!(progress.completed_files, 0);
    assert_eq!(progress.failed_files, 0);
    assert_eq!(progress.in_progress_files, 0);
    
    // Simulate starting download
    session.update_file_status("document.pdf", DownloadState::InProgress);
    let progress = session.get_progress_summary();
    assert_eq!(progress.in_progress_files, 1);
    
    // Simulate completion
    session.update_file_status("document.pdf", DownloadState::Completed);
    let progress = session.get_progress_summary();
    assert_eq!(progress.completed_files, 1);
    assert_eq!(progress.in_progress_files, 0);
    
    // Simulate failure
    session.update_file_status("text.txt", DownloadState::Failed);
    let progress = session.get_progress_summary();
    assert_eq!(progress.failed_files, 1);
    
    println!("✓ Step 5 complete: Progress tracking verified, download flow complete");
}

/// Test Complete End-to-End Flow Simulation
#[test]
fn test_complete_download_flow() {
    // Step 1: Parse CLI arguments
    let cli = Cli::parse_from([
        "ia-get",
        "--include-ext", "pdf,txt",
        "--max-file-size", "2MB",
        "--verbose",
        "https://archive.org/details/test-archive"
    ]);
    
    assert!(cli.verbose);
    assert_eq!(cli.include_ext, Some("pdf,txt".to_string()));
    assert_eq!(cli.max_file_size, Some("2MB".to_string()));
    assert_eq!(cli.url, Some("https://archive.org/details/test-archive".to_string()));
    
    // Step 2: Validate CLI configuration
    assert!(cli.validate().is_ok());
    
    // Step 3: Extract and validate URL
    let url = cli.get_url().unwrap();
    let validation_result = validate_and_process_url(url);
    assert!(validation_result.is_ok());
    
    // Step 4: Extract identifier
    let identifier = extract_identifier_from_url(url).unwrap();
    assert_eq!(identifier, "test-archive");
    
    // Step 5: Generate metadata URL
    let metadata_url = get_json_url(url);
    assert_eq!(metadata_url, "https://archive.org/metadata/test-archive");
    
    // Step 6: Parse filter configuration
    let include_extensions = cli.include_extensions();
    assert_eq!(include_extensions, vec!["pdf", "txt"]);
    
    let max_file_size = cli.max_file_size_bytes().unwrap();
    assert_eq!(max_file_size, 2097152); // 2MB in bytes
    
    // Step 7: Filter files (simulated metadata)
    let metadata = create_test_archive_metadata();
    let filtered_files = filter_files(&metadata.files, &include_extensions, &[], Some(max_file_size));
    assert_eq!(filtered_files.len(), 2); // PDF and TXT files under 2MB
    
    // Step 8: Create download configuration
    let config = DownloadConfig {
        output_dir: cli.get_output_dir().unwrap_or("downloads").to_string(),
        concurrent_downloads: cli.concurrent_downloads,
        max_retries: cli.max_retries,
        include_extensions: cli.include_extensions(),
        exclude_extensions: cli.exclude_extensions(),
        max_file_size: cli.max_file_size_bytes(),
        resume: cli.resume,
        compress: cli.compress,
        decompress: cli.decompress,
        decompress_formats: cli.decompress_formats,
        dry_run: cli.dry_run,
        verbose: cli.verbose,
        log_hash_errors: cli.log_hash_errors,
    };
    
    // Step 9: Create download session
    let requested_files: Vec<String> = filtered_files.iter().map(|f| f.name.clone()).collect();
    let session = DownloadSession::new(
        url.to_string(),
        identifier.to_string(),
        metadata,
        config,
        requested_files,
    );
    
    // Step 10: Verify session is ready for download
    assert_eq!(session.get_pending_files().len(), 2);
    let progress = session.get_progress_summary();
    assert_eq!(progress.total_files, 2);
    assert_eq!(progress.completed_files, 0);
    
    println!("✓ Complete flow test passed: All steps validated successfully");
}

/// Test Error Recovery Flow
#[test]
fn test_error_recovery_flow() {
    // Test invalid URL handling
    let invalid_url = "https://example.com/not-archive";
    let result = validate_and_process_url(invalid_url);
    assert!(result.is_err());
    
    if let Err(error) = result {
        match error {
            IaGetError::Parse(_) => {
                println!("✓ URL validation error handled correctly");
            }
            _ => panic!("Unexpected error type"),
        }
    }
    
    // Test invalid metadata handling
    let invalid_json = "{ invalid json }";
    let result = parse_archive_metadata(invalid_json);
    assert!(result.is_err());
    
    // Test filter validation
    let metadata = create_test_archive_metadata();
    let invalid_size = parse_size_string("invalid");
    assert!(invalid_size.is_err());
    
    // Valid filters should still work after error
    let valid_filters = filter_files(&metadata.files, &["pdf".to_string()], &[], None);
    assert_eq!(valid_filters.len(), 1);
    
    println!("✓ Error recovery flow validated");
}

/// Test User Interaction Flow Simulation
#[test]
fn test_user_interaction_flow() {
    // Simulate user providing minimal input (just URL)
    let cli = Cli::parse_from(["ia-get", "https://archive.org/details/test-archive"]);
    
    // System should use defaults for missing options
    assert!(!cli.verbose); // Default: false
    assert_eq!(cli.concurrent_downloads, 3); // Default: 3
    assert_eq!(cli.max_retries, 3); // Default: 3
    assert_eq!(cli.include_ext, None); // Default: none (include all)
    
    // System should detect non-interactive mode
    assert!(!cli.is_interactive_mode());
    
    // System should proceed with download automatically
    let url = cli.get_url().unwrap();
    assert_eq!(url, "https://archive.org/details/test-archive");
    
    println!("✓ User interaction flow: Minimal input handled with sensible defaults");
    
    // Simulate user providing comprehensive configuration
    let cli_detailed = Cli::parse_from([
        "ia-get",
        "--verbose",
        "--dry-run",
        "--concurrent-downloads", "5",
        "--include-ext", "pdf,txt",
        "--exclude-ext", "xml",
        "--max-file-size", "10MB",
        "--output", "custom-dir",
        "https://archive.org/details/test-archive"
    ]);
    
    // System should respect all user preferences
    assert!(cli_detailed.verbose);
    assert!(cli_detailed.dry_run);
    assert_eq!(cli_detailed.concurrent_downloads, 5);
    assert_eq!(cli_detailed.include_ext, Some("pdf,txt".to_string()));
    assert_eq!(cli_detailed.exclude_ext, Some("xml".to_string()));
    assert_eq!(cli_detailed.max_file_size, Some("10MB".to_string()));
    assert_eq!(cli_detailed.output_path, Some("custom-dir".to_string()));
    
    println!("✓ User interaction flow: Detailed configuration handled correctly");
}

/// Test Automatic Flow Progression
#[test]
fn test_automatic_flow_progression() {
    // Test that each successful step enables the next step
    
    // Step 1 → Step 2: Valid URL enables metadata fetching
    let url = "https://archive.org/details/test-archive";
    let url_valid = validate_and_process_url(url).is_ok();
    assert!(url_valid);
    
    if url_valid {
        // Step 2 → Step 3: Valid metadata enables file filtering
        let metadata_url = get_json_url(url);
        assert!(!metadata_url.is_empty());
        
        // Step 3 → Step 4: Selected files enable session creation
        let metadata = create_test_archive_metadata();
        let files = filter_files(&metadata.files, &[], &[], None);
        let has_files = !files.is_empty();
        assert!(has_files);
        
        if has_files {
            // Step 4 → Step 5: Session creation enables download execution
            let config = DownloadConfig {
                output_dir: "downloads".to_string(),
                concurrent_downloads: 3,
                max_retries: 3,
                include_extensions: vec![],
                exclude_extensions: vec![],
                max_file_size: None,
                resume: false,
                compress: false,
                decompress: false,
                decompress_formats: vec![],
                dry_run: false,
                verbose: false,
                log_hash_errors: false,
            };
            
            let requested_files: Vec<String> = files.iter().map(|f| f.name.clone()).collect();
            let session = DownloadSession::new(
                url.to_string(),
                "test-archive".to_string(),
                metadata,
                config,
                requested_files,
            );
            
            // Session is ready for download execution
            assert!(!session.get_pending_files().is_empty());
            println!("✓ Automatic progression: All steps flow correctly into the next");
        }
    }
}

/// Test Flow with Different CLI Configurations
#[test]
fn test_flow_with_different_configurations() {
    // Test with subcommand syntax
    let cli_subcommand = Cli::parse_from([
        "ia-get", 
        "download", 
        "--output", "test-output",
        "--include-ext", "pdf",
        "test-identifier"
    ]);
    
    match cli_subcommand.command {
        Some(Commands::Download { url, output, include_ext, .. }) => {
            assert_eq!(url, "test-identifier");
            assert_eq!(output, Some("test-output".to_string()));
            assert_eq!(include_ext, Some("pdf".to_string()));
        }
        _ => panic!("Expected Download command"),
    }
    
    // Test URL extraction from subcommand
    assert_eq!(cli_subcommand.get_url(), Some("test-identifier"));
    assert_eq!(cli_subcommand.get_output_dir(), Some("test-output"));
    
    // Test with direct URL syntax
    let cli_direct = Cli::parse_from([
        "ia-get",
        "--include-ext", "txt",
        "--output", "direct-output", 
        "https://archive.org/details/direct-test"
    ]);
    
    assert_eq!(cli_direct.get_url(), Some("https://archive.org/details/direct-test"));
    assert_eq!(cli_direct.get_output_dir(), Some("direct-output"));
    assert_eq!(cli_direct.include_ext, Some("txt".to_string()));
    
    println!("✓ Flow works with different CLI configuration syntaxes");
}