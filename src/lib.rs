//! # ia-get
//!
//! A command-line tool for downloading files from the Internet Archive.
//!
//! This tool takes an archive.org details URL (a whole item) or a download
//! URL naming a single file, and downloads the associated files, with
//! support for resumable downloads and MD5 hash verification.

pub mod archive_metadata;
pub mod check;
pub mod cookie;
pub mod display;
pub mod downloader;
pub mod error;
pub mod file_filter;
pub mod filename;
pub mod fs;
pub mod plan;
#[cfg(test)]
pub mod test_support;
pub mod verbose;

// Re-export the error types for convenience
pub use error::{IaGetError, Result};
