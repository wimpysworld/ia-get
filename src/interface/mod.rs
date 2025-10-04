//! User interface layer
//!
//! This module contains all user interface components including CLI, GUI, interactive interfaces,
//! and FFI bindings for mobile platforms.

pub mod cli;

// SIMPLIFIED FFI INTERFACE (v0.8.0+)
// This is the only FFI interface. The old complex FFI (14+ functions) has been removed.
// The new interface has only 6 functions for 57% less complexity and zero race conditions.
//
// Documentation: docs/SIMPLIFIED_FFI_PROGRESS.md
#[cfg(feature = "ffi")]
pub mod ffi_simple;

#[cfg(feature = "gui")]
pub mod gui;
pub mod interactive;

// Re-export commonly used interface types
pub use cli::*;

// Export simplified FFI
#[cfg(feature = "ffi")]
pub use ffi_simple::*;

#[cfg(feature = "gui")]
pub use gui::*;
pub use interactive::*;
