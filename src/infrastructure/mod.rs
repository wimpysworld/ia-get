//! Infrastructure layer
//!
//! This module contains infrastructure components like HTTP clients, configuration, and API integration.

pub mod api;
pub mod config;
pub mod http;

// Re-export commonly used infrastructure types
pub use api::*;
pub use config::*;
pub use http::*;
