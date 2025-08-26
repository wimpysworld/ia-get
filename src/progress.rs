//! Enhanced progress tracking for ia-get downloads
//!
//! Provides detailed statistics, ETA calculations, and summary reports.

use std::time::{Duration, Instant};
use colored::*;
use crate::filters::format_size;

/// Download statistics tracker
#[derive(Debug, Clone)]
pub struct DownloadStats {
    pub total_files: usize,
    pub completed_files: usize,
    pub skipped_files: usize,
    pub failed_files: usize,
    pub total_bytes: u64,
    pub downloaded_bytes: u64,
    pub start_time: Instant,
    pub current_speed: f64, // bytes per second
}

impl DownloadStats {
    pub fn new(total_files: usize, total_bytes: u64) -> Self {
        Self {
            total_files,
            completed_files: 0,
            skipped_files: 0,
            failed_files: 0,
            total_bytes,
            downloaded_bytes: 0,
            start_time: Instant::now(),
            current_speed: 0.0,
        }
    }

    /// Calculate estimated time remaining
    pub fn eta(&self) -> Option<Duration> {
        if self.current_speed <= 0.0 || self.downloaded_bytes >= self.total_bytes {
            return None;
        }
        
        let remaining_bytes = self.total_bytes - self.downloaded_bytes;
        let eta_seconds = (remaining_bytes as f64 / self.current_speed) as u64;
        Some(Duration::from_secs(eta_seconds))
    }

    /// Get overall completion percentage
    pub fn completion_percentage(&self) -> u8 {
        if self.total_bytes == 0 {
            return (self.completed_files * 100 / self.total_files.max(1)) as u8;
        }
        ((self.downloaded_bytes * 100) / self.total_bytes.max(1)) as u8
    }

    /// Update download speed (call periodically)
    pub fn update_speed(&mut self, bytes_downloaded: u64) {
        self.downloaded_bytes = bytes_downloaded;
        let elapsed = self.start_time.elapsed().as_secs_f64();
        if elapsed > 1.0 {
            self.current_speed = bytes_downloaded as f64 / elapsed;
        }
    }

    /// Format current speed for display
    pub fn speed_string(&self) -> String {
        if self.current_speed <= 0.0 {
            return "calculating...".to_string();
        }
        
        let speed_per_sec = self.current_speed;
        if speed_per_sec >= 1024.0 * 1024.0 * 1024.0 {
            format!("{:.1} GB/s", speed_per_sec / (1024.0 * 1024.0 * 1024.0))
        } else if speed_per_sec >= 1024.0 * 1024.0 {
            format!("{:.1} MB/s", speed_per_sec / (1024.0 * 1024.0))
        } else if speed_per_sec >= 1024.0 {
            format!("{:.1} KB/s", speed_per_sec / 1024.0)
        } else {
            format!("{:.0} B/s", speed_per_sec)
        }
    }

    /// Format ETA for display
    pub fn eta_string(&self) -> String {
        match self.eta() {
            Some(eta) => {
                let total_seconds = eta.as_secs();
                if total_seconds >= 3600 {
                    format!("{}h {}m", total_seconds / 3600, (total_seconds % 3600) / 60)
                } else if total_seconds >= 60 {
                    format!("{}m {}s", total_seconds / 60, total_seconds % 60)
                } else {
                    format!("{}s", total_seconds)
                }
            }
            None => "calculating...".to_string(),
        }
    }

    /// Generate final download summary
    pub fn generate_summary(&self) -> String {
        let elapsed = self.start_time.elapsed();
        let hours = elapsed.as_secs() / 3600;
        let minutes = (elapsed.as_secs() % 3600) / 60;
        let seconds = elapsed.as_secs() % 60;
        
        let mut summary = String::new();
        summary.push_str(&format!("\n{}\n", "=".repeat(60).cyan()));
        summary.push_str(&format!("{}\n", "📊 DOWNLOAD SUMMARY".bold().cyan()));
        summary.push_str(&format!("{}\n", "=".repeat(60).cyan()));
        
        // File statistics
        summary.push_str(&format!(
            "📁 Files: {} {} | {} {} | {} {}\n",
            self.completed_files.to_string().green().bold(),
            "completed".green(),
            self.skipped_files.to_string().yellow().bold(),
            "skipped".yellow(),
            self.failed_files.to_string().red().bold(),
            "failed".red()
        ));
        
        // Data statistics
        if self.total_bytes > 0 {
            summary.push_str(&format!(
                "💾 Data: {} downloaded of {} total ({}%)\n",
                format_size(self.downloaded_bytes).green().bold(),
                format_size(self.total_bytes),
                self.completion_percentage().to_string().green().bold()
            ));
        }
        
        // Time and speed
        summary.push_str(&format!(
            "⏱️  Time: {}h {}m {}s | Average speed: {}\n",
            hours,
            minutes,
            seconds,
            self.speed_string().cyan().bold()
        ));
        
        // Status
        if self.failed_files == 0 {
            summary.push_str(&format!("✅ {}\n", "All downloads completed successfully!".green().bold()));
        } else {
            summary.push_str(&format!(
                "⚠️  {} {} with errors (check batchlog.json)\n",
                self.failed_files.to_string().red().bold(),
                if self.failed_files == 1 { "file" } else { "files" }.red()
            ));
        }
        
        summary.push_str(&format!("{}\n", "=".repeat(60).cyan()));
        summary
    }
}

/// Progress message formatter for consistent display
pub struct ProgressFormatter;

impl ProgressFormatter {
    /// Format a progress line for download display
    pub fn format_download_line(
        current_file: &str,
        file_progress: u8,
        overall_progress: u8,
        stats: &DownloadStats,
        remaining_files: usize,
    ) -> String {
        format!(
            "{}% │ {} │ {} │ {} │ ETA: {} │ {} files left",
            overall_progress.to_string().cyan().bold(),
            current_file.truncate_to(25).bold(),
            format!("{}%", file_progress).green(),
            stats.speed_string().cyan(),
            stats.eta_string().yellow(),
            remaining_files.to_string().dimmed()
        )
    }
    
    /// Format concurrent download status
    pub fn format_concurrent_status(
        active_downloads: &[(String, u8)], // (filename, progress)
        stats: &DownloadStats,
    ) -> String {
        let active_list: Vec<String> = active_downloads
            .iter()
            .map(|(name, progress)| format!("{}({}%)", name.truncate_to(15), progress))
            .collect();
        
        format!(
            "Downloading: {} │ Speed: {} │ Overall: {}% │ ETA: {}",
            active_list.join(", ").bold(),
            stats.speed_string().cyan(),
            stats.completion_percentage().to_string().green().bold(),
            stats.eta_string().yellow()
        )
    }
}

/// String truncation helper trait
trait StringTruncate {
    fn truncate_to(&self, max_len: usize) -> String;
}

impl StringTruncate for str {
    fn truncate_to(&self, max_len: usize) -> String {
        if self.len() <= max_len {
            self.to_string()
        } else {
            format!("{}…", &self[..max_len.saturating_sub(1)])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_download_stats() {
        let mut stats = DownloadStats::new(10, 1024 * 1024); // 10 files, 1MB total
        assert_eq!(stats.completion_percentage(), 0);
        
        stats.update_speed(512 * 1024); // 512KB downloaded
        assert_eq!(stats.completion_percentage(), 50);
    }

    #[test]
    fn test_string_truncate() {
        assert_eq!("hello".truncate_to(10), "hello");
        assert_eq!("hello world test".truncate_to(10), "hello wor…");
    }
}
