//! CLI commands module
//!
//! Contains all subcommand implementations for the ia-get CLI.

pub mod batch;
pub mod search;

// Re-export commonly used types
pub use batch::{batch_download, BatchConfig, BatchItemResult};
pub use search::{display_search_results, search_archive, SearchResults};
