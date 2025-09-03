//! Progress Reporting Support Layer Tests
//!
//! Tests for download progress tracking, statistics calculation,
//! and display formatting functionality.

use ia_get::progress::{DownloadStats, StringTruncate};

#[test]
fn test_download_stats() {
    let mut stats = DownloadStats::new(10, 1024 * 1024); // 10 files, 1MB total
    assert_eq!(stats.completion_percentage(), 0);

    stats.update_speed(512 * 1024); // 512KB downloaded
    assert_eq!(stats.completion_percentage(), 50);
}

#[test]
fn test_download_stats_initialization() {
    let stats = DownloadStats::new(5, 2048);
    assert_eq!(stats.completion_percentage(), 0);
    assert_eq!(stats.files_completed(), 0);
    assert_eq!(stats.total_files(), 5);
    assert_eq!(stats.total_size(), 2048);
}

#[test]
fn test_download_stats_completion() {
    let mut stats = DownloadStats::new(3, 3072);

    // Update with partial progress
    stats.update_speed(1024);
    assert_eq!(stats.completion_percentage(), 33);

    // Complete download
    stats.update_speed(3072);
    assert_eq!(stats.completion_percentage(), 100);
}

#[test]
fn test_string_truncate() {
    assert_eq!("hello".truncate_to(10), "hello");
    assert_eq!("hello world test".truncate_to(10), "hello wor…");
}

#[test]
fn test_string_truncate_edge_cases() {
    // Empty string
    assert_eq!("".truncate_to(5), "");

    // String shorter than limit
    assert_eq!("hi".truncate_to(10), "hi");

    // String exactly at limit
    assert_eq!("exactly10".truncate_to(9), "exactly10");

    // Very short limit
    assert_eq!("test".truncate_to(1), "…");
    assert_eq!("test".truncate_to(2), "t…");

    // Zero limit (edge case)
    assert_eq!("test".truncate_to(0), "");
}

#[test]
fn test_string_truncate_unicode() {
    // Test with unicode characters
    assert_eq!("café".truncate_to(6), "café");
    assert_eq!("café world".truncate_to(6), "café …");
    assert_eq!("🎉🎊🎈".truncate_to(3), "🎉🎊🎈"); // 3 chars exactly fits
    assert_eq!("🎉🎊🎈".truncate_to(2), "🎉…"); // Truncate to 2 chars
}
