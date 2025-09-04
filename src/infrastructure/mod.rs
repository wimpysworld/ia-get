//! Infrastructure layer
//!
//! This module contains infrastructure components like HTTP clients, configuration, and API integration.

pub mod api;
pub mod config;
pub mod http;

// Re-export commonly used infrastructure types with specific imports to avoid conflicts
pub use api::{
    get_archive_servers, validate_identifier, ApiStats, ArchiveOrgApiClient,
    EnhancedArchiveApiClient, ItemDetails, ServiceStatus,
};
pub use config::*;
pub use http::*;
