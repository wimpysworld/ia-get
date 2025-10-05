//! Mobile FFI Wrapper for ia-get
//!
//! This library re-exports the core ia-get FFI functionality
//! with mobile-optimized defaults and convenience functions.
//!
//! ## Architecture
//!
//! This library provides a thin wrapper around the simplified FFI interface
//! from the main ia-get library. The Flutter/Dart code uses these functions
//! directly via Dart FFI (DynamicLibrary), providing a clean separation of concerns:
//!
//! - **Rust (this library)**: Computation engine, I/O operations, performance
//! - **Dart/Flutter**: State management, UI, user interaction
//! - **FFI**: Thin bridge with no logic, just 6 simple functions
//!
//! ## Simplified FFI Functions (6 total)
//!
//! This library re-exports exactly 6 stateless functions from the main ia-get library:
//!
//! 1. `ia_get_fetch_metadata()` - Fetch archive metadata as JSON
//! 2. `ia_get_download_file()` - Download a file with progress callback
//! 3. `ia_get_decompress_file()` - Decompress an archive file
//! 4. `ia_get_validate_checksum()` - Validate file checksum
//! 5. `ia_get_last_error()` - Get last error message
//! 6. `ia_get_free_string()` - Free strings returned by library
//!
//! All state management is done on the Dart side, keeping this layer stateless and simple.

// Re-export the FFI functions from main library
// The main library already has #[no_mangle] on these functions, so we just
// need to ensure they're included in the compilation. We do this by depending
// on the ia-get crate with the ffi feature enabled.
//
// Re-exporting types that Dart/Flutter might need
pub use ia_get::interface::ffi_simple::{IaGetResult, ProgressCallback};

// ============================================================================
// Mobile-specific FFI Functions
// ============================================================================
// These functions are specific to the mobile wrapper and not in the main library

/// Get library version information
///
/// Returns the version string of the ia-get mobile library.
/// Caller should NOT free the returned string (it's static).
#[no_mangle]
pub extern "C" fn ia_get_mobile_version() -> *const std::os::raw::c_char {
    // Return a static string that doesn't need to be freed
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr() as *const std::os::raw::c_char
}

/// Get supported architectures
///
/// Returns a comma-separated list of supported Android architectures.
/// Caller should NOT free the returned string (it's static).
#[no_mangle]
pub extern "C" fn ia_get_mobile_supported_archs() -> *const std::os::raw::c_char {
    // Return a static string that doesn't need to be freed
    c"arm64-v8a,armeabi-v7a,x86_64,x86".as_ptr()
}
