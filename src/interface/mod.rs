//! User interface layer
//!
//! This module contains all user interface components including CLI, GUI, and interactive interfaces.
//!
//! ## Architecture Note
//!
//! The Flutter mobile app now uses a pure Dart implementation and no longer depends on FFI.
//! The Rust implementation focuses on CLI and GUI desktop applications.

pub mod cli;

#[cfg(feature = "gui")]
pub mod gui;
pub mod interactive;

// Re-export commonly used interface types
pub use cli::*;

#[cfg(feature = "gui")]
pub use gui::*;
pub use interactive::*;
