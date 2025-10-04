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

// Re-export all FFI functions from the main library's simplified FFI interface
// Explicitly list each function to ensure they are included in the binary
pub use ia_get::interface::ffi_simple::{
    ia_get_decompress_file, ia_get_download_file, ia_get_fetch_metadata, ia_get_free_string,
    ia_get_last_error, ia_get_validate_checksum, IaGetResult,
};

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
    "arm64-v8a,armeabi-v7a,x86_64,x86\0".as_ptr() as *const std::os::raw::c_char
}
