//! GUI panels module
//!
//! Contains different UI panels for the ia-get GUI application

pub mod archive_health;
pub mod config;
pub mod download;
pub mod file_browser;
pub mod filters;

pub use archive_health::ArchiveHealthPanel;
pub use config::ConfigPanel;
pub use download::DownloadPanel;
pub use file_browser::FileBrowserPanel;
pub use filters::FiltersPanel;
