//! GUI module for ia-get
//!
//! Provides a graphical user interface wrapper around the core ia-get functionality
//! using egui for cross-platform compatibility.

pub mod app;
pub mod panels;

#[cfg(test)]
mod tests;

pub use app::IaGetApp;
