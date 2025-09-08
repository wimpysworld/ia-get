//! User interface layer
//!
//! This module contains all user interface components including CLI, GUI, interactive interfaces,
//! and FFI bindings for mobile platforms.

pub mod cli;
#[cfg(feature = "ffi")]
pub mod ffi;
#[cfg(feature = "gui")]
pub mod gui;
pub mod interactive;

// Re-export commonly used interface types
pub use cli::*;
#[cfg(feature = "ffi")]
pub use ffi::*;
#[cfg(feature = "gui")]
pub use gui::*;
pub use interactive::*;
