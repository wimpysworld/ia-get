//! CLI commands module
//!
//! Contains all subcommand implementations for the ia-get CLI.

pub mod batch;
pub mod search;

// Re-export commonly used types
pub use batch::{BatchConfig, BatchItemResult, batch_download};
pub use search::{SearchResults, display_search_results, search_archive};
