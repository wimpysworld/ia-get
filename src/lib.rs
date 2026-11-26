//! # ia-get
//!
//! A command-line tool for downloading files from the Internet Archive.
//!
//! This tool takes an archive.org details URL and downloads all associated files,
//! with support for resumable downloads and MD5 hash verification.

pub mod archive_metadata;
pub mod constants;
pub mod cookie;
pub mod disk_space;
pub mod downloader;
pub mod error;
pub mod filter;
pub mod utils;
pub mod verify;

#[cfg(test)]
extern crate self as ia_get;
#[cfg(test)]
pub(crate) mod test_support;

// Re-export common types for convenience
pub use cookie::CookieSource;
pub use error::{IaGetError, Result};
pub use filter::FileFilter;
pub use utils::VerboseLogger;
