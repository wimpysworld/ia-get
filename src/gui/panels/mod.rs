//! GUI panels module
//!
//! Contains different UI panels for the ia-get GUI application

pub mod archive_health;
pub mod config;
pub mod download;
pub mod filters;

pub use archive_health::ArchiveHealthPanel;
pub use config::ConfigPanel;
pub use download::DownloadPanel;
pub use filters::FiltersPanel;
