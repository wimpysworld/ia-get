//! Metadata storage module for ia-get
//!
//! Handles storing and retrieving complete Internet Archive JSON metadata
//! for download resumption and comprehensive file management.

use crate::{IaGetError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Complete Internet Archive metadata response structure
/// Based on <https://archive.org/developers/md-read.html>
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ArchiveMetadata {
    /// Timestamp when the item was created
    pub created: u64,
    /// Primary datanode server
    pub d1: String,
    /// Secondary datanode server  
    pub d2: String,
    /// Directory path on servers
    pub dir: String,
    /// Complete list of files in the archive
    pub files: Vec<ArchiveFile>,
    /// Total number of files
    pub files_count: u32,
    /// Last time the item was updated
    pub item_last_updated: u64,
    /// Total size of all files in bytes
    pub item_size: u64,
    /// Item metadata (title, description, etc.)
    pub metadata: serde_json::Value,
    /// Primary server for downloads
    pub server: String,
    /// Unique identifier for the record
    pub uniq: u64,
    /// List of servers that can serve the files
    pub workable_servers: Vec<String>,
    /// Optional reviews data
    #[serde(default)]
    pub reviews: Vec<serde_json::Value>,
}

/// Individual file entry from Internet Archive
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ArchiveFile {
    /// File name
    pub name: String,
    /// Source type (original, metadata, etc.)
    pub source: String,
    /// File format
    pub format: Option<String>,
    /// Last modified time as Unix timestamp string
    #[serde(default, deserialize_with = "deserialize_string_to_u64_option")]
    pub mtime: Option<u64>,
    /// File size in bytes as string
    #[serde(default, deserialize_with = "deserialize_string_to_u64_option")]
    pub size: Option<u64>,
    /// MD5 hash of the file
    pub md5: Option<String>,
    /// CRC32 checksum
    pub crc32: Option<String>,
    /// SHA1 hash
    pub sha1: Option<String>,
    /// BitTorrent info hash
    pub btih: Option<String>,
    /// Summation type for checksums
    pub summation: Option<String>,
    /// Original file reference
    pub original: Option<String>,
    /// Rotation angle for images
    #[serde(default, deserialize_with = "deserialize_string_to_u32_option")]
    pub rotation: Option<u32>,
}

/// Download session metadata for resumption
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DownloadSession {
    /// Original URL requested
    pub original_url: String,
    /// Archive identifier
    pub identifier: String,
    /// Complete archive metadata
    pub archive_metadata: ArchiveMetadata,
    /// Download configuration used
    pub download_config: DownloadConfig,
    /// Files that were requested for download
    pub requested_files: Vec<String>,
    /// Download status for each file
    pub file_status: HashMap<String, FileDownloadStatus>,
    /// Session start time
    pub session_start: u64,
    /// Last update time
    pub last_updated: u64,
}

/// Configuration used for downloads
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DownloadConfig {
    /// Output directory
    pub output_dir: String,
    /// Maximum concurrent downloads
    pub max_concurrent: u32,
    /// File format filters
    pub format_filters: Vec<String>,
    /// Minimum file size
    pub min_size: Option<u64>,
    /// Maximum file size  
    pub max_size: Option<u64>,
    /// Whether to verify MD5 hashes
    pub verify_md5: bool,
    /// Whether to preserve file modification times
    pub preserve_mtime: bool,
    /// User agent string
    pub user_agent: String,
    /// Whether to enable compression during downloads
    pub enable_compression: bool,
    /// Whether to automatically decompress downloaded files
    pub auto_decompress: bool,
    /// Compression formats to decompress automatically
    pub decompress_formats: Vec<String>,
}

/// Status of an individual file download
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FileDownloadStatus {
    /// File information from archive
    pub file_info: ArchiveFile,
    /// Download state
    pub status: DownloadState,
    /// Bytes downloaded so far
    pub bytes_downloaded: u64,
    /// Download start time
    pub started_at: Option<u64>,
    /// Download completion time
    pub completed_at: Option<u64>,
    /// Error message if download failed
    pub error_message: Option<String>,
    /// Number of retry attempts
    pub retry_count: u32,
    /// Server used for download
    pub server_used: Option<String>,
    /// Local file path
    pub local_path: String,
}

/// Download state enumeration
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum DownloadState {
    /// Not yet started
    Pending,
    /// Currently downloading
    InProgress,
    /// Successfully completed
    Completed,
    /// Failed with error
    Failed,
    /// Paused/cancelled
    Paused,
    /// Skipped (e.g., already exists)
    Skipped,
}

/// Custom deserializer for string numbers to u64 Option with default support
fn deserialize_string_to_u64_option<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{self, Visitor};
    use std::fmt;

    struct StringToU64Visitor;

    impl<'de> Visitor<'de> for StringToU64Visitor {
        type Value = Option<u64>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a string, number, or null that can be converted to u64")
        }

        fn visit_str<E>(self, value: &str) -> std::result::Result<Option<u64>, E>
        where
            E: de::Error,
        {
            if value.is_empty() {
                Ok(None)
            } else {
                value.parse::<u64>().map(Some).map_err(de::Error::custom)
            }
        }

        fn visit_u64<E>(self, value: u64) -> std::result::Result<Option<u64>, E>
        where
            E: de::Error,
        {
            Ok(Some(value))
        }

        fn visit_i64<E>(self, value: i64) -> std::result::Result<Option<u64>, E>
        where
            E: de::Error,
        {
            if value >= 0 {
                Ok(Some(value as u64))
            } else {
                Err(de::Error::custom(
                    "negative number cannot be converted to u64",
                ))
            }
        }

        fn visit_none<E>(self) -> std::result::Result<Option<u64>, E>
        where
            E: de::Error,
        {
            Ok(None)
        }

        fn visit_unit<E>(self) -> std::result::Result<Option<u64>, E>
        where
            E: de::Error,
        {
            Ok(None)
        }
    }

    deserializer.deserialize_any(StringToU64Visitor)
}

/// Custom deserializer for string numbers to u32 Option with default support
fn deserialize_string_to_u32_option<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<u32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{self, Visitor};
    use std::fmt;

    struct StringToU32Visitor;

    impl<'de> Visitor<'de> for StringToU32Visitor {
        type Value = Option<u32>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a string, number, or null that can be converted to u32")
        }

        fn visit_str<E>(self, value: &str) -> std::result::Result<Option<u32>, E>
        where
            E: de::Error,
        {
            if value.is_empty() {
                Ok(None)
            } else {
                value.parse::<u32>().map(Some).map_err(de::Error::custom)
            }
        }

        fn visit_u32<E>(self, value: u32) -> std::result::Result<Option<u32>, E>
        where
            E: de::Error,
        {
            Ok(Some(value))
        }

        fn visit_i32<E>(self, value: i32) -> std::result::Result<Option<u32>, E>
        where
            E: de::Error,
        {
            if value >= 0 {
                Ok(Some(value as u32))
            } else {
                Err(de::Error::custom(
                    "negative number cannot be converted to u32",
                ))
            }
        }

        fn visit_none<E>(self) -> std::result::Result<Option<u32>, E>
        where
            E: de::Error,
        {
            Ok(None)
        }

        fn visit_unit<E>(self) -> std::result::Result<Option<u32>, E>
        where
            E: de::Error,
        {
            Ok(None)
        }
    }

    deserializer.deserialize_any(StringToU32Visitor)
}

impl DownloadSession {
    /// Create a new download session
    pub fn new(
        original_url: String,
        identifier: String,
        archive_metadata: ArchiveMetadata,
        download_config: DownloadConfig,
        requested_files: Vec<String>,
    ) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let mut file_status = HashMap::new();
        for file_name in &requested_files {
            if let Some(file_info) = archive_metadata.files.iter().find(|f| f.name == *file_name) {
                let local_path = format!("{}/{}", download_config.output_dir, file_name);
                file_status.insert(
                    file_name.clone(),
                    FileDownloadStatus {
                        file_info: file_info.clone(),
                        status: DownloadState::Pending,
                        bytes_downloaded: 0,
                        started_at: None,
                        completed_at: None,
                        error_message: None,
                        retry_count: 0,
                        server_used: None,
                        local_path,
                    },
                );
            }
        }

        Self {
            original_url,
            identifier,
            archive_metadata,
            download_config,
            requested_files,
            file_status,
            session_start: now,
            last_updated: now,
        }
    }

    /// Save session to disk
    pub fn save_to_file<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| IaGetError::JsonParsing(format!("Failed to serialize session: {}", e)))?;

        std::fs::write(path, json)
            .map_err(|e| IaGetError::FileSystem(format!("Failed to write session file: {}", e)))?;

        Ok(())
    }

    /// Load session from disk
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| IaGetError::FileSystem(format!("Failed to read session file: {}", e)))?;

        serde_json::from_str(&content)
            .map_err(|e| IaGetError::JsonParsing(format!("Failed to parse session file: {}", e)))
    }

    /// Update file status
    pub fn update_file_status(&mut self, file_name: &str, status: DownloadState) {
        if let Some(file_status) = self.file_status.get_mut(file_name) {
            file_status.status = status;
            self.last_updated = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
        }
    }

    /// Get files that still need to be downloaded
    pub fn get_pending_files(&self) -> Vec<&str> {
        self.file_status
            .iter()
            .filter(|(_, status)| {
                matches!(
                    status.status,
                    DownloadState::Pending | DownloadState::Failed
                )
            })
            .map(|(name, _)| name.as_str())
            .collect()
    }

    /// Get download progress summary
    pub fn get_progress_summary(&self) -> DownloadProgress {
        let mut completed = 0;
        let mut failed = 0;
        let mut in_progress = 0;
        let mut total_bytes = 0;
        let mut downloaded_bytes = 0;

        for status in self.file_status.values() {
            match status.status {
                DownloadState::Completed => completed += 1,
                DownloadState::Failed => failed += 1,
                DownloadState::InProgress => in_progress += 1,
                _ => {}
            }

            if let Some(size) = status.file_info.size {
                total_bytes += size;
            }
            downloaded_bytes += status.bytes_downloaded;
        }

        DownloadProgress {
            total_files: self.file_status.len(),
            completed_files: completed,
            failed_files: failed,
            in_progress_files: in_progress,
            total_bytes,
            downloaded_bytes,
        }
    }
}

/// Progress summary for downloads
#[derive(Debug)]
pub struct DownloadProgress {
    pub total_files: usize,
    pub completed_files: usize,
    pub failed_files: usize,
    pub in_progress_files: usize,
    pub total_bytes: u64,
    pub downloaded_bytes: u64,
}

impl ArchiveFile {
    /// Get the download URL for this file using the specified server
    pub fn get_download_url(&self, server: &str, dir: &str) -> String {
        format!("https://{}{}/{}", server, dir, self.name)
    }

    /// Check if this file matches the given format filters
    pub fn matches_format_filter(&self, filters: &[String]) -> bool {
        if filters.is_empty() {
            return true;
        }

        if let Some(format) = &self.format {
            filters
                .iter()
                .any(|filter| format.to_lowercase().contains(&filter.to_lowercase()))
        } else {
            false
        }
    }

    /// Check if this file meets size requirements
    pub fn meets_size_requirements(&self, min_size: Option<u64>, max_size: Option<u64>) -> bool {
        if let Some(size) = self.size {
            if let Some(min) = min_size {
                if size < min {
                    return false;
                }
            }
            if let Some(max) = max_size {
                if size > max {
                    return false;
                }
            }
        }
        true
    }

    /// Validate MD5 hash of a local file
    pub fn validate_md5<P: AsRef<Path>>(&self, file_path: P) -> Result<bool> {
        if let Some(expected_md5) = &self.md5 {
            let actual_md5 = crate::utils::calculate_md5(file_path)?;
            Ok(actual_md5.to_lowercase() == expected_md5.to_lowercase())
        } else {
            Ok(true) // No MD5 to validate
        }
    }

    /// Set the modification time of a local file to match the archive
    pub fn set_file_mtime<P: AsRef<Path>>(&self, file_path: P) -> Result<()> {
        if let Some(mtime) = self.mtime {
            use std::time::UNIX_EPOCH;

            let _modified_time = UNIX_EPOCH + std::time::Duration::from_secs(mtime);
            let metadata = std::fs::metadata(&file_path).map_err(|e| {
                IaGetError::FileSystem(format!("Failed to get file metadata: {}", e))
            })?;

            let permissions = metadata.permissions();
            std::fs::set_permissions(&file_path, permissions)
                .map_err(|e| IaGetError::FileSystem(format!("Failed to set permissions: {}", e)))?;

            // Note: Setting file times on Windows requires additional handling
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                let _atime = metadata.atime();
                use std::process::Command;

                let _mtime_str = mtime.to_string();
                Command::new("touch")
                    .args([
                        "-t",
                        &format!("{}", mtime),
                        file_path.as_ref().to_str().unwrap(),
                    ])
                    .output()
                    .map_err(|e| IaGetError::FileSystem(format!("Failed to set mtime: {}", e)))?;
            }

            #[cfg(windows)]
            {
                // On Windows, we'll use the filetime crate if available, or skip mtime setting
                eprintln!("Warning: Setting file modification time not implemented on Windows");
            }
        }
        Ok(())
    }

    /// Check if this file is compressed based on its format or extension
    pub fn is_compressed(&self) -> bool {
        // Check format field first
        if let Some(format) = &self.format {
            let format_lower = format.to_lowercase();
            if matches!(
                format_lower.as_str(),
                "zip" | "gzip" | "bzip2" | "xz" | "tar" | "7z" | "rar" | "lz4" | "zstd"
            ) {
                return true;
            }
        }

        // Check file extension as fallback
        let name_lower = self.name.to_lowercase();
        name_lower.ends_with(".zip")
            || name_lower.ends_with(".gz")
            || name_lower.ends_with(".bz2")
            || name_lower.ends_with(".xz")
            || name_lower.ends_with(".tar")
            || name_lower.ends_with(".tar.gz")
            || name_lower.ends_with(".tar.bz2")
            || name_lower.ends_with(".tar.xz")
            || name_lower.ends_with(".7z")
            || name_lower.ends_with(".rar")
            || name_lower.ends_with(".lz4")
            || name_lower.ends_with(".zst")
    }

    /// Get the compression format of this file
    pub fn get_compression_format(&self) -> Option<String> {
        if !self.is_compressed() {
            return None;
        }

        // Check format field first
        if let Some(format) = &self.format {
            let format_lower = format.to_lowercase();
            if matches!(
                format_lower.as_str(),
                "zip" | "gzip" | "bzip2" | "xz" | "tar" | "7z" | "rar" | "lz4" | "zstd"
            ) {
                return Some(format_lower);
            }
        }

        // Determine from file extension
        let name_lower = self.name.to_lowercase();
        if name_lower.ends_with(".zip") {
            Some("zip".to_string())
        } else if name_lower.ends_with(".gz") || name_lower.ends_with(".tar.gz") {
            Some("gzip".to_string())
        } else if name_lower.ends_with(".bz2") || name_lower.ends_with(".tar.bz2") {
            Some("bzip2".to_string())
        } else if name_lower.ends_with(".xz") || name_lower.ends_with(".tar.xz") {
            Some("xz".to_string())
        } else if name_lower.ends_with(".tar") {
            Some("tar".to_string())
        } else if name_lower.ends_with(".7z") {
            Some("7z".to_string())
        } else if name_lower.ends_with(".rar") {
            Some("rar".to_string())
        } else if name_lower.ends_with(".lz4") {
            Some("lz4".to_string())
        } else if name_lower.ends_with(".zst") {
            Some("zstd".to_string())
        } else {
            None
        }
    }

    /// Get the expected decompressed file name
    pub fn get_decompressed_name(&self) -> String {
        if !self.is_compressed() {
            return self.name.clone();
        }

        let name = &self.name;
        // Remove compression extensions
        if name.ends_with(".tar.gz") {
            name.trim_end_matches(".gz").to_string()
        } else if name.ends_with(".tar.bz2") {
            name.trim_end_matches(".bz2").to_string()
        } else if name.ends_with(".tar.xz") {
            name.trim_end_matches(".xz").to_string()
        } else if name.ends_with(".gz") {
            name.trim_end_matches(".gz").to_string()
        } else if name.ends_with(".bz2") {
            name.trim_end_matches(".bz2").to_string()
        } else if name.ends_with(".xz") {
            name.trim_end_matches(".xz").to_string()
        } else if name.ends_with(".zip") {
            // For ZIP files, we'll extract to a directory with the same name
            name.trim_end_matches(".zip").to_string()
        } else {
            // For other formats, remove the extension
            std::path::Path::new(name)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(name)
                .to_string()
        }
    }
}

/// Sanitize an identifier for safe filesystem use across platforms
///
/// This function ensures identifiers are safe for use in Windows, macOS, and Linux filesystems:
/// - Replaces invalid characters with safe alternatives
/// - Limits length to prevent path length issues on Windows
/// - Generates a stable hash suffix for overly long identifiers
/// - Preserves readability while ensuring filesystem compatibility
fn sanitize_identifier_for_filesystem(identifier: &str) -> String {
    // Windows has stricter filename rules, so we optimize for Windows compatibility
    // Invalid characters for Windows: < > : " | ? * \ /
    // We also avoid characters that could cause issues in shells: & $ ! % ^ ( ) [ ] { } ;

    let mut sanitized = identifier
        .chars()
        .filter_map(|c| match c {
            // Windows-forbidden characters
            '<' | '>' | ':' | '"' | '|' | '?' | '*' | '\\' | '/' => None,
            // Shell-problematic characters
            '&' | '$' | '!' | '%' | '^' | '(' | ')' | '[' | ']' | '{' | '}' | ';' => None,
            // Replace spaces with underscores
            ' ' => Some('_'),
            // Remove control characters
            c if c.is_control() => None,
            // Keep everything else
            c => Some(c),
        })
        .collect::<String>();

    // Remove consecutive hyphens and underscores
    while sanitized.contains("--") {
        sanitized = sanitized.replace("--", "-");
    }
    while sanitized.contains("__") {
        sanitized = sanitized.replace("__", "_");
    }

    // Trim leading/trailing hyphens and underscores
    sanitized = sanitized.trim_matches(&['-', '_'] as &[char]).to_string();

    // Ensure we have something if the identifier was all invalid characters
    if sanitized.is_empty() {
        sanitized = "archive".to_string();
    }

    // Windows filename limit is 255 characters, but we need to account for:
    // - "ia-get-session-" prefix (16 chars)
    // - "-{timestamp}.json" suffix (~15 chars)
    // - Some buffer for safety
    // So we limit the identifier portion to 200 characters to be safe
    const MAX_IDENTIFIER_LENGTH: usize = 200;

    if sanitized.len() <= MAX_IDENTIFIER_LENGTH {
        sanitized
    } else {
        // For overly long identifiers, use first part + hash of full identifier
        // This ensures uniqueness while keeping readability
        let hash = {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut hasher = DefaultHasher::new();
            identifier.hash(&mut hasher);
            format!("{:x}", hasher.finish())
        };

        // Take first 180 chars and add hash (8-16 chars)
        let truncated_length = MAX_IDENTIFIER_LENGTH - hash.len() - 1; // -1 for the separator
        let truncated = &sanitized[..truncated_length.min(sanitized.len())];
        format!(
            "{}-{}",
            truncated.trim_end_matches(&['-', '_'] as &[char]),
            hash
        )
    }
}

/// Generate a session file name based on identifier and timestamp
pub fn generate_session_filename(identifier: &str) -> String {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let sanitized_identifier = sanitize_identifier_for_filesystem(identifier);
    format!("ia-get-session-{}-{}.json", sanitized_identifier, timestamp)
}

/// Find the most recent session file for an identifier
pub fn find_latest_session_file(identifier: &str, session_dir: &str) -> Result<Option<String>> {
    // Try both the new sanitized identifier and the original identifier for backward compatibility
    let sanitized_identifier = sanitize_identifier_for_filesystem(identifier);
    let patterns = [
        format!("ia-get-session-{}-", sanitized_identifier),
        format!("ia-get-session-{}-", identifier), // For backward compatibility
    ];

    let entries = std::fs::read_dir(session_dir)
        .map_err(|e| IaGetError::FileSystem(format!("Failed to read session directory: {}", e)))?;

    let mut session_files = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| {
            IaGetError::FileSystem(format!("Failed to read directory entry: {}", e))
        })?;
        let file_name = entry.file_name();
        let file_name_str = file_name.to_string_lossy();

        // Check if the file matches any of our patterns
        let matches_pattern = patterns
            .iter()
            .any(|pattern| file_name_str.starts_with(pattern) && file_name_str.ends_with(".json"));

        if matches_pattern {
            session_files.push(entry.path());
        }
    }

    // Sort by modification time, newest first
    session_files.sort_by_key(|path| {
        std::fs::metadata(path)
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
    });

    Ok(session_files
        .last()
        .map(|p| p.to_string_lossy().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_identifier_normal() {
        let identifier = "normal-identifier_123";
        let result = sanitize_identifier_for_filesystem(identifier);
        assert_eq!(result, "normal-identifier_123");
    }

    #[test]
    fn test_sanitize_identifier_with_invalid_characters() {
        let identifier = "test<>:|?*\\";
        let result = sanitize_identifier_for_filesystem(identifier);
        assert_eq!(result, "test");
    }

    #[test]
    fn test_sanitize_identifier_with_spaces() {
        let identifier = "test with spaces";
        let result = sanitize_identifier_for_filesystem(identifier);
        assert_eq!(result, "test_with_spaces");
    }

    #[test]
    fn test_sanitize_identifier_windows_problematic() {
        let identifier = "file<name>:with|invalid?chars*";
        let result = sanitize_identifier_for_filesystem(identifier);
        assert_eq!(result, "filenamewithinvalidchars");
    }

    #[test]
    fn test_sanitize_identifier_consecutive_separators() {
        let identifier = "test--with__consecutive___separators";
        let result = sanitize_identifier_for_filesystem(identifier);
        assert_eq!(result, "test-with_consecutive_separators");
    }

    #[test]
    fn test_sanitize_identifier_trim_edges() {
        let identifier = "--test_identifier--";
        let result = sanitize_identifier_for_filesystem(identifier);
        assert_eq!(result, "test_identifier");
    }

    #[test]
    fn test_sanitize_identifier_empty_after_cleaning() {
        let identifier = "!$%^&*()";
        let result = sanitize_identifier_for_filesystem(identifier);
        assert_eq!(result, "archive");
    }

    #[test]
    fn test_sanitize_identifier_long_identifier() {
        // Create an identifier longer than 200 characters
        let long_identifier = "a".repeat(250);
        let result = sanitize_identifier_for_filesystem(&long_identifier);

        // Should be truncated and include a hash
        assert!(result.len() <= 200);
        assert!(result.contains("-")); // Should have hash separator
        assert!(result.starts_with("a")); // Should start with original content
    }

    #[test]
    fn test_sanitize_identifier_real_world_case() {
        let identifier =
            "ikaos-som-dragon-ball-complete-001-153-r2j-dragon-box-multi-audio-v4_202301";
        let result = sanitize_identifier_for_filesystem(identifier);
        // This identifier is already valid, should remain unchanged
        assert_eq!(result, identifier);
    }

    #[test]
    fn test_sanitize_identifier_control_characters() {
        let identifier = "test\x00\x01\x02\x03identifier";
        let result = sanitize_identifier_for_filesystem(identifier);
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
    fn test_sanitize_preserves_reasonable_length() {
        let identifier = "moderately-long-but-reasonable-identifier-name";
        let result = sanitize_identifier_for_filesystem(identifier);
        assert_eq!(result, identifier);
    }

    #[test]
    fn test_hash_consistency_for_long_identifiers() {
        let long_identifier = "a".repeat(250);
        let result1 = sanitize_identifier_for_filesystem(&long_identifier);
        let result2 = sanitize_identifier_for_filesystem(&long_identifier);

        // Should generate the same result for the same input
        assert_eq!(result1, result2);
    }
}
