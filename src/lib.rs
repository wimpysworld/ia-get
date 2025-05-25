//! # ia-get
//!
//! A command-line tool for downloading files from the Internet Archive.
//! 
//! This tool takes an archive.org details URL and downloads all associated files,
//! with support for resumable downloads and MD5 hash verification.

pub mod error;
pub mod utils;
pub mod archive_metadata;

// Re-export the error types for convenience
pub use error::{IaGetError, Result};