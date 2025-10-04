//! Stateless core functions for FFI integration
//!
//! This module provides pure, stateless computation functions designed for
//! easy FFI integration. All state management is handled by the caller (Dart/Flutter).
//!
//! Key principles:
//! - No global state
//! - No sessions or persistent state
//! - Simple input/output
//! - Synchronous operations for easy FFI binding
//! - Return JSON for complex data structures

pub mod compression;
pub mod download;
pub mod metadata;
pub mod validation;
