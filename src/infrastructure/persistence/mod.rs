//! Persistence module for ia-get
//!
//! Handles all persistent data including download history, task information,
//! and configuration management with proper priority handling.

pub mod config_persistence;
pub mod download_history;

pub use config_persistence::{ConfigPersistence, ConfigPriority, ConfigSource};
pub use download_history::{DownloadHistory, DownloadHistoryEntry, TaskStatus};
