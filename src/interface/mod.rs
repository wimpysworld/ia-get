//! User interface layer
//!
//! This module contains all user interface components including CLI, GUI, interactive interfaces,
//! and FFI bindings for mobile platforms.

pub mod cli;

// OLD FFI INTERFACE - DEPRECATED
// The old FFI interface (ffi.rs) with 14+ functions is deprecated as of v0.8.0
// and will be removed in v1.0.0. It has been replaced by the simplified FFI interface
// (ffi_simple.rs) with only 6 functions for 57% less complexity and zero race conditions.
//
// Migration guide: docs/MIGRATION_TO_SIMPLIFIED_FFI.md
#[deprecated(
    since = "0.8.0",
    note = "Use `ffi_simple` instead. The old FFI interface will be removed in v1.0.0. See docs/MIGRATION_TO_SIMPLIFIED_FFI.md for migration guide."
)]
#[cfg(feature = "ffi")]
pub mod ffi;

// NEW SIMPLIFIED FFI INTERFACE - RECOMMENDED
#[cfg(feature = "ffi")]
pub mod ffi_simple;

#[cfg(feature = "gui")]
pub mod gui;
pub mod interactive;

// Re-export commonly used interface types
pub use cli::*;

// Export new simplified FFI (recommended)
#[cfg(feature = "ffi")]
pub use ffi_simple::*;

#[cfg(feature = "gui")]
pub use gui::*;
pub use interactive::*;
