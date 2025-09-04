//! Archive operations and metadata handling
//!
//! Contains functionality for working with Internet Archive metadata and archive operations.

pub use archive_metadata::*;
pub use metadata::*;
// Note: metadata_new module is available but not re-exported as it may be deprecated

pub mod archive_metadata;
pub mod metadata;
pub mod metadata_new;
