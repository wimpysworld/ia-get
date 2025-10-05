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

use std::os::raw::{c_char, c_int, c_void};

// Import the FFI functions from main library
use ia_get::interface::ffi_simple;

// Re-export the result type and callback type
pub use ia_get::interface::ffi_simple::{IaGetResult, ProgressCallback};

// ============================================================================
// FFI Function Wrappers - Explicit C symbol exports
// ============================================================================
// Note: pub use doesn't export C symbols, so we need explicit #[no_mangle]
// wrapper functions that delegate to the main library

/// Fetch metadata for an Internet Archive item
///
/// Returns a JSON string containing the metadata. The caller must free
/// the returned string using `ia_get_free_string()`.
#[no_mangle]
pub unsafe extern "C" fn ia_get_fetch_metadata(identifier: *const c_char) -> *mut c_char {
    ffi_simple::ia_get_fetch_metadata(identifier)
}

/// Download a file with progress callback
#[no_mangle]
pub unsafe extern "C" fn ia_get_download_file(
    url: *const c_char,
    output_path: *const c_char,
    progress_callback: ProgressCallback,
    user_data: *mut c_void,
) -> IaGetResult {
    ffi_simple::ia_get_download_file(url, output_path, progress_callback, user_data)
}

/// Decompress an archive file
#[no_mangle]
pub unsafe extern "C" fn ia_get_decompress_file(
    archive_path: *const c_char,
    output_dir: *const c_char,
) -> *mut c_char {
    ffi_simple::ia_get_decompress_file(archive_path, output_dir)
}

/// Validate file checksum
#[no_mangle]
pub unsafe extern "C" fn ia_get_validate_checksum(
    file_path: *const c_char,
    expected_hash: *const c_char,
    hash_type: *const c_char,
) -> c_int {
    ffi_simple::ia_get_validate_checksum(file_path, expected_hash, hash_type)
}

/// Get the last error message
#[no_mangle]
pub extern "C" fn ia_get_last_error() -> *const c_char {
    ffi_simple::ia_get_last_error()
}

/// Free a string returned by this library
#[no_mangle]
pub unsafe extern "C" fn ia_get_free_string(s: *mut c_char) {
    ffi_simple::ia_get_free_string(s)
}

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
