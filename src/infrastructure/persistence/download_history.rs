//! Download history and task persistence module
//!
//! Manages the ia-get-db.json file that stores download history,
//! task status, and download settings used.

use crate::{Result, core::session::DownloadConfig, error::IaGetError};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Status of a download task
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TaskStatus {
    /// Download is currently in progress
    InProgress,
    /// Download completed successfully
    Success,
    /// Download failed with error
    Failed(String),
    /// Download was cancelled by user
    Cancelled,
    /// Download is paused/resumed
    Paused,
}

/// A single download history entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadHistoryEntry {
    /// Unique identifier for this download task
    pub id: String,
    /// Archive identifier that was downloaded
    pub archive_identifier: String,
    /// Original URL/identifier used to start download
    pub original_input: String,
    /// Output directory where files were downloaded
    pub output_directory: String,
    /// Current status of the download
    pub status: TaskStatus,
    /// When the download was started
    pub started_at: DateTime<Utc>,
    /// When the download was completed/failed (if applicable)
    pub completed_at: Option<DateTime<Utc>>,
    /// Download configuration used for this task
    pub download_config: DownloadConfig,
    /// Total number of files to download
    pub total_files: usize,
    /// Number of files completed
    pub completed_files: usize,
    /// Number of files that failed
    pub failed_files: usize,
    /// Total bytes downloaded
    pub bytes_downloaded: u64,
    /// Total bytes expected
    pub total_bytes: u64,
    /// Error message if download failed
    pub error_message: Option<String>,
    /// Additional metadata about the download
    pub metadata: serde_json::Value,
}

/// Download history manager
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadHistory {
    /// Version of the database format
    pub version: String,
    /// When this database was created
    pub created_at: DateTime<Utc>,
    /// When this database was last updated
    pub last_updated: DateTime<Utc>,
    /// List of download entries
    pub entries: Vec<DownloadHistoryEntry>,
    /// Maximum number of entries to keep (older entries are removed)
    pub max_entries: usize,
}

impl Default for DownloadHistory {
    fn default() -> Self {
        let now = Utc::now();
        Self {
            version: "1.0.0".to_string(),
            created_at: now,
            last_updated: now,
            entries: Vec::new(),
            max_entries: 1000, // Keep last 1000 downloads
        }
    }
}

impl DownloadHistory {
    /// Create a new download history or load existing one
    pub fn load_or_create(db_path: &Path) -> Result<Self> {
        if db_path.exists() {
            Self::load_from_file(db_path)
        } else {
            Ok(Self::default())
        }
    }

    /// Load download history from file
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = fs::read_to_string(path.as_ref())
            .map_err(|e| IaGetError::Config(format!("Failed to read download history: {}", e)))?;

        let mut history: Self = serde_json::from_str(&content)
            .map_err(|e| IaGetError::Config(format!("Failed to parse download history: {}", e)))?;

        // Validate and clean up old entries if needed
        history.cleanup_old_entries();

        Ok(history)
    }

    /// Save download history to file
    pub fn save_to_file<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        self.last_updated = Utc::now();
        self.cleanup_old_entries();

        // Ensure parent directory exists
        if let Some(parent) = path.as_ref().parent() {
            fs::create_dir_all(parent).map_err(|e| {
                IaGetError::Config(format!("Failed to create config directory: {}", e))
            })?;
        }

        let content = serde_json::to_string_pretty(self).map_err(|e| {
            IaGetError::Config(format!("Failed to serialize download history: {}", e))
        })?;

        fs::write(path.as_ref(), content)
            .map_err(|e| IaGetError::Config(format!("Failed to write download history: {}", e)))?;

        Ok(())
    }

    /// Add a new download entry
    pub fn add_entry(&mut self, entry: DownloadHistoryEntry) {
        self.entries.push(entry);
        self.cleanup_old_entries();
    }

    /// Update an existing entry by ID
    pub fn update_entry<F>(&mut self, id: &str, updater: F) -> Result<()>
    where
        F: FnOnce(&mut DownloadHistoryEntry),
    {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.id == id) {
            updater(entry);
            Ok(())
        } else {
            Err(IaGetError::Config(format!(
                "Download entry with ID '{}' not found",
                id
            )))
        }
    }

    /// Get entry by ID
    pub fn get_entry(&self, id: &str) -> Option<&DownloadHistoryEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// Get all entries for a specific archive
    pub fn get_entries_for_archive(&self, archive_identifier: &str) -> Vec<&DownloadHistoryEntry> {
        self.entries
            .iter()
            .filter(|e| e.archive_identifier == archive_identifier)
            .collect()
    }

    /// Get entries by status
    pub fn get_entries_by_status(&self, status: &TaskStatus) -> Vec<&DownloadHistoryEntry> {
        self.entries
            .iter()
            .filter(|e| std::mem::discriminant(&e.status) == std::mem::discriminant(status))
            .collect()
    }

    /// Get recent entries (newest first)
    pub fn get_recent_entries(&self, limit: usize) -> Vec<&DownloadHistoryEntry> {
        let mut entries = self.entries.iter().collect::<Vec<_>>();
        entries.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        entries.into_iter().take(limit).collect()
    }

    /// Remove entries older than specified days
    pub fn cleanup_entries_older_than(&mut self, days: i64) {
        let cutoff = Utc::now() - chrono::Duration::days(days);
        self.entries.retain(|entry| entry.started_at > cutoff);
    }

    /// Remove entries to stay within max_entries limit
    fn cleanup_old_entries(&mut self) {
        if self.entries.len() > self.max_entries {
            // Sort by start time (newest first) and keep only max_entries
            self.entries.sort_by(|a, b| b.started_at.cmp(&a.started_at));
            self.entries.truncate(self.max_entries);
        }
    }

    /// Get statistics about downloads
    pub fn get_statistics(&self) -> DownloadStatistics {
        let total_downloads = self.entries.len();
        let successful = self
            .entries
            .iter()
            .filter(|e| matches!(e.status, TaskStatus::Success))
            .count();
        let failed = self
            .entries
            .iter()
            .filter(|e| matches!(e.status, TaskStatus::Failed(_)))
            .count();
        let in_progress = self
            .entries
            .iter()
            .filter(|e| matches!(e.status, TaskStatus::InProgress))
            .count();
        let cancelled = self
            .entries
            .iter()
            .filter(|e| matches!(e.status, TaskStatus::Cancelled))
            .count();

        let total_bytes_downloaded: u64 = self.entries.iter().map(|e| e.bytes_downloaded).sum();
        let total_files_downloaded: usize = self.entries.iter().map(|e| e.completed_files).sum();

        DownloadStatistics {
            total_downloads,
            successful_downloads: successful,
            failed_downloads: failed,
            in_progress_downloads: in_progress,
            cancelled_downloads: cancelled,
            total_bytes_downloaded,
            total_files_downloaded,
        }
    }

    /// Clear all entries
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Remove entry by ID
    pub fn remove_entry(&mut self, id: &str) -> bool {
        let initial_len = self.entries.len();
        self.entries.retain(|entry| entry.id != id);
        self.entries.len() != initial_len
    }
}

/// Statistics about download history
#[derive(Debug, Clone)]
pub struct DownloadStatistics {
    pub total_downloads: usize,
    pub successful_downloads: usize,
    pub failed_downloads: usize,
    pub in_progress_downloads: usize,
    pub cancelled_downloads: usize,
    pub total_bytes_downloaded: u64,
    pub total_files_downloaded: usize,
}

impl DownloadHistoryEntry {
    /// Create a new download history entry
    pub fn new(
        archive_identifier: String,
        original_input: String,
        output_directory: String,
        download_config: DownloadConfig,
    ) -> Self {
        let id = format!("{}-{}", archive_identifier, Utc::now().timestamp());

        Self {
            id,
            archive_identifier,
            original_input,
            output_directory,
            status: TaskStatus::InProgress,
            started_at: Utc::now(),
            completed_at: None,
            download_config,
            total_files: 0,
            completed_files: 0,
            failed_files: 0,
            bytes_downloaded: 0,
            total_bytes: 0,
            error_message: None,
            metadata: serde_json::Value::Object(serde_json::Map::new()),
        }
    }

    /// Mark as completed successfully
    pub fn mark_completed(&mut self) {
        self.status = TaskStatus::Success;
        self.completed_at = Some(Utc::now());
    }

    /// Mark as failed with error message
    pub fn mark_failed(&mut self, error: String) {
        self.status = TaskStatus::Failed(error.clone());
        self.error_message = Some(error);
        self.completed_at = Some(Utc::now());
    }

    /// Mark as cancelled
    pub fn mark_cancelled(&mut self) {
        self.status = TaskStatus::Cancelled;
        self.completed_at = Some(Utc::now());
    }

    /// Update progress information
    pub fn update_progress(
        &mut self,
        completed_files: usize,
        failed_files: usize,
        bytes_downloaded: u64,
    ) {
        self.completed_files = completed_files;
        self.failed_files = failed_files;
        self.bytes_downloaded = bytes_downloaded;
    }

    /// Get completion percentage
    pub fn completion_percentage(&self) -> f32 {
        if self.total_files == 0 {
            0.0
        } else {
            (self.completed_files as f32 / self.total_files as f32) * 100.0
        }
    }

    /// Get duration of download
    pub fn duration(&self) -> Option<chrono::Duration> {
        if let Some(completed_at) = self.completed_at {
            Some(completed_at - self.started_at)
        } else {
            Some(Utc::now() - self.started_at)
        }
    }
}

/// Get the default path for the download history database
pub fn get_default_history_db_path() -> Result<PathBuf> {
    let config_dir = crate::infrastructure::config::ConfigManager::get_config_directory()?;
    Ok(config_dir.join("ia-get-db.json"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_download_history_creation() {
        let history = DownloadHistory::default();
        assert_eq!(history.entries.len(), 0);
        assert_eq!(history.version, "1.0.0");
        assert_eq!(history.max_entries, 1000);
    }

    #[test]
    fn test_download_history_entry_creation() {
        let config = DownloadConfig {
            output_dir: "/tmp/test".to_string(),
            max_concurrent: 3,
            format_filters: vec![],
            min_size: None,
            max_size: None,
            enable_compression: true,
            auto_decompress: false,
            decompress_formats: vec![],
            verify_md5: true,
            preserve_mtime: true,
            user_agent: "test-agent".to_string(),
        };

        let entry = DownloadHistoryEntry::new(
            "test-archive".to_string(),
            "https://archive.org/details/test-archive".to_string(),
            "/tmp/test".to_string(),
            config,
        );

        assert_eq!(entry.archive_identifier, "test-archive");
        assert!(matches!(entry.status, TaskStatus::InProgress));
        assert_eq!(entry.completed_files, 0);
    }

    #[test]
    fn test_download_history_persistence() -> Result<()> {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test-db.json");

        let mut history = DownloadHistory::default();
        let config = DownloadConfig {
            output_dir: "/tmp/test".to_string(),
            max_concurrent: 3,
            format_filters: vec![],
            min_size: None,
            max_size: None,
            enable_compression: true,
            auto_decompress: false,
            decompress_formats: vec![],
            verify_md5: true,
            preserve_mtime: true,
            user_agent: "test-agent".to_string(),
        };

        let entry = DownloadHistoryEntry::new(
            "test-archive".to_string(),
            "test-input".to_string(),
            "/tmp/test".to_string(),
            config,
        );

        history.add_entry(entry);
        history.save_to_file(&db_path)?;

        let loaded_history = DownloadHistory::load_from_file(&db_path)?;
        assert_eq!(loaded_history.entries.len(), 1);
        assert_eq!(loaded_history.entries[0].archive_identifier, "test-archive");

        Ok(())
    }

    #[test]
    fn test_entry_progress_tracking() {
        let config = DownloadConfig {
            output_dir: "/tmp/test".to_string(),
            max_concurrent: 3,
            format_filters: vec![],
            min_size: None,
            max_size: None,
            enable_compression: true,
            auto_decompress: false,
            decompress_formats: vec![],
            verify_md5: true,
            preserve_mtime: true,
            user_agent: "test-agent".to_string(),
        };

        let mut entry = DownloadHistoryEntry::new(
            "test-archive".to_string(),
            "test-input".to_string(),
            "/tmp/test".to_string(),
            config,
        );

        entry.total_files = 100;
        entry.update_progress(50, 5, 1024 * 1024);

        assert_eq!(entry.completion_percentage(), 50.0);
        assert_eq!(entry.completed_files, 50);
        assert_eq!(entry.failed_files, 5);
        assert_eq!(entry.bytes_downloaded, 1024 * 1024);
    }
}
