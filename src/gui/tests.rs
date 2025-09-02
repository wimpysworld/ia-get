//! Tests for GUI components

use crate::config::Config;
use crate::download_service::DownloadService;
use crate::gui::panels::{ConfigPanel, DownloadPanel, FiltersPanel};

#[test]
fn test_gui_panels_creation() {
    // Test that all GUI panels can be created successfully
    let config = Config::default();

    let _config_panel = ConfigPanel::new(config.clone());
    let _download_panel = DownloadPanel::new();
    let _filters_panel = FiltersPanel::new();
}

#[test]
fn test_download_service_creation() {
    // Test that download service can be created
    let result = DownloadService::new();
    assert!(
        result.is_ok(),
        "Download service should be created successfully"
    );
}

#[test]
fn test_filter_settings() {
    // Test filter panel functionality
    let filters_panel = FiltersPanel::new();

    // Test getting empty filter settings
    let (include, exclude, min_size, max_size) = filters_panel.get_filter_settings();
    assert!(include.is_empty());
    assert!(exclude.is_empty());
    assert!(min_size.is_empty());
    assert!(max_size.is_empty());
}
