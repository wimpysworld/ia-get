//! GUI module for ia-get
//!
//! Provides a graphical user interface wrapper around the core ia-get functionality
//! using egui for cross-platform compatibility.

pub mod app;
pub mod panels;

// Re-export the main app struct for easy importing
pub use app::IaGetApp;
