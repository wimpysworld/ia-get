//! Shared utilities layer
//!
//! This module contains shared utilities and helper functions used across the application.

pub mod common;
pub mod compression;
pub mod filters;

// Re-export commonly used utilities with careful naming to avoid conflicts
pub use common::{constants::*, performance::*, progress::*, url_processing::*, utils::*};
pub use compression::*;
// Note: format_size is available from filters::filters module to avoid naming conflicts
pub use filters::{file_formats::*, format_help::*};
