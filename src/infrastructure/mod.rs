//! Infrastructure layer
//!
//! This module contains infrastructure components like HTTP clients, configuration, and API integration.

pub mod api;
pub mod config;
pub mod http;
pub mod persistence;

// Re-export commonly used infrastructure types with specific imports to avoid conflicts
pub use api::{
    ApiStats, ArchiveOrgApiClient, EnhancedArchiveApiClient, ItemDetails, ServiceStatus,
    get_archive_servers, validate_identifier,
};
pub use config::*;
pub use http::*;
