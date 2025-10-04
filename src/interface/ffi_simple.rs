//! Simplified FFI Interface for Mobile Platforms
//!
//! This module provides a minimal, stateless C-compatible interface for Flutter/mobile integration.
//! Key principles:
//! - Only 5 core functions (down from 14+)
//! - No state management in Rust
//! - Simple request-response pattern
//! - All state managed by caller (Dart/Flutter)

use std::ffi::{c_void, CStr, CString};
use std::os::raw::{c_char, c_int};
use std::ptr;

use crate::core::stateless;

/// Result codes for FFI operations
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum IaGetResult {
    Success = 0,
    ErrorNetwork = 1,
    ErrorFileSystem = 2,
    ErrorInvalidInput = 3,
    ErrorInternal = 4,
}

// Thread-local storage for error messages
thread_local! {
    static LAST_ERROR: std::cell::RefCell<Option<CString>> = const { std::cell::RefCell::new(None) };
}

/// Set the last error message
fn set_last_error(msg: &str) {
    LAST_ERROR.with(|cell| {
        *cell.borrow_mut() = CString::new(msg).ok();
    });
}

/// Clear the last error
fn clear_last_error() {
    LAST_ERROR.with(|cell| {
        *cell.borrow_mut() = None;
    });
}

// ═══════════════════════════════════════════════════════════
// SIMPLIFIED FFI FUNCTIONS - Only 5 core functions
// ═══════════════════════════════════════════════════════════

/// Fetch metadata for an Internet Archive item
///
/// Returns a JSON string containing the metadata. The caller must free
/// the returned string using `ia_get_free_string()`.
///
/// # Arguments
///
/// * `identifier` - Archive.org identifier (e.g., "commute_test")
///
/// # Returns
///
/// * Pointer to JSON string on success (must be freed by caller)
/// * NULL on error (call `ia_get_last_error()` for details)
///
/// # Safety
///
/// The identifier must be a valid C string pointer.
/// # Safety
///
/// The identifier must be a valid null-terminated C string pointer.
#[no_mangle]
pub unsafe extern "C" fn ia_get_fetch_metadata(identifier: *const c_char) -> *mut c_char {
    clear_last_error();

    // Validate input
    if identifier.is_null() {
        set_last_error("Identifier cannot be null");
        return ptr::null_mut();
    }

    // Convert C string to Rust string
    let identifier_str = match CStr::from_ptr(identifier).to_str() {
        Ok(s) => s,
        Err(_) => {
            set_last_error("Invalid UTF-8 in identifier");
            return ptr::null_mut();
        }
    };

    // Call stateless core function
    match stateless::metadata::fetch_metadata_json(identifier_str) {
        Ok(json) => match CString::new(json) {
            Ok(c_string) => c_string.into_raw(),
            Err(_) => {
                set_last_error("Failed to create C string from JSON");
                ptr::null_mut()
            }
        },
        Err(e) => {
            set_last_error(&format!("Failed to fetch metadata: {}", e));
            ptr::null_mut()
        }
    }
}

/// Progress callback type for downloads
///
/// Arguments: (bytes_downloaded, total_bytes, user_data)
pub type ProgressCallback =
    Option<extern "C" fn(downloaded: u64, total: u64, user_data: *mut c_void)>;

/// Download a file from URL to specified path
///
/// This is a BLOCKING operation - the caller should run it in a background thread/isolate.
///
/// # Arguments
///
/// * `url` - Source URL
/// * `output_path` - Destination file path
/// * `progress_callback` - Optional callback for progress updates (can be NULL)
/// * `user_data` - User data passed to callback (can be NULL)
///
/// # Returns
///
/// * `IaGetResult::Success` on success
/// * Error code on failure (call `ia_get_last_error()` for details)
///
/// # Safety
///
/// URL and output_path must be valid C string pointers.
#[no_mangle]
pub unsafe extern "C" fn ia_get_download_file(
    url: *const c_char,
    output_path: *const c_char,
    progress_callback: ProgressCallback,
    user_data: *mut c_void,
) -> IaGetResult {
    clear_last_error();

    // Validate inputs
    if url.is_null() || output_path.is_null() {
        set_last_error("URL and output path cannot be null");
        return IaGetResult::ErrorInvalidInput;
    }

    // Convert C strings to Rust strings
    let url_str = match CStr::from_ptr(url).to_str() {
        Ok(s) => s,
        Err(_) => {
            set_last_error("Invalid UTF-8 in URL");
            return IaGetResult::ErrorInvalidInput;
        }
    };

    let path_str = match CStr::from_ptr(output_path).to_str() {
        Ok(s) => s,
        Err(_) => {
            set_last_error("Invalid UTF-8 in output path");
            return IaGetResult::ErrorInvalidInput;
        }
    };

    // Create progress callback wrapper
    let progress_fn = progress_callback.map(|cb| {
        Box::new(move |downloaded: u64, total: u64| {
            cb(downloaded, total, user_data);
        }) as stateless::download::ProgressCallback
    });

    // Call stateless core function
    match stateless::download::download_file_sync(url_str, path_str, progress_fn) {
        Ok(_) => IaGetResult::Success,
        Err(e) => {
            let error_msg = format!("Download failed: {}", e);
            set_last_error(&error_msg);

            // Classify error type
            if error_msg.contains("network") || error_msg.contains("HTTP") {
                IaGetResult::ErrorNetwork
            } else if error_msg.contains("file") || error_msg.contains("I/O") {
                IaGetResult::ErrorFileSystem
            } else {
                IaGetResult::ErrorInternal
            }
        }
    }
}

/// Decompress an archive file
///
/// Supports: zip, gzip, bzip2, xz, tar, tar.gz, tar.bz2, tar.xz
///
/// Returns a JSON array of extracted file paths. The caller must free
/// the returned string using `ia_get_free_string()`.
///
/// # Arguments
///
/// * `archive_path` - Path to archive file
/// * `output_dir` - Directory to extract to
///
/// # Returns
///
/// * Pointer to JSON array of extracted files on success (must be freed)
/// * NULL on error (call `ia_get_last_error()` for details)
///
/// # Safety
///
/// Both paths must be valid C string pointers.
#[no_mangle]
pub unsafe extern "C" fn ia_get_decompress_file(
    archive_path: *const c_char,
    output_dir: *const c_char,
) -> *mut c_char {
    clear_last_error();

    // Validate inputs
    if archive_path.is_null() || output_dir.is_null() {
        set_last_error("Archive path and output directory cannot be null");
        return ptr::null_mut();
    }

    // Convert C strings to Rust strings
    let archive_str = match CStr::from_ptr(archive_path).to_str() {
        Ok(s) => s,
        Err(_) => {
            set_last_error("Invalid UTF-8 in archive path");
            return ptr::null_mut();
        }
    };

    let output_str = match CStr::from_ptr(output_dir).to_str() {
        Ok(s) => s,
        Err(_) => {
            set_last_error("Invalid UTF-8 in output directory");
            return ptr::null_mut();
        }
    };

    // Call stateless core function
    match stateless::compression::decompress_archive_json(archive_str, output_str) {
        Ok(json) => match CString::new(json) {
            Ok(c_string) => c_string.into_raw(),
            Err(_) => {
                set_last_error("Failed to create C string from JSON");
                ptr::null_mut()
            }
        },
        Err(e) => {
            set_last_error(&format!("Decompression failed: {}", e));
            ptr::null_mut()
        }
    }
}

/// Validate file checksum
///
/// # Arguments
///
/// * `file_path` - Path to file to validate
/// * `expected_hash` - Expected hash value (hex string)
/// * `hash_type` - Hash algorithm: "md5", "sha1", or "sha256"
///
/// # Returns
///
/// * 1 if hash matches
/// * 0 if hash doesn't match
/// * -1 on error (call `ia_get_last_error()` for details)
///
/// # Safety
///
/// All arguments must be valid C string pointers.
#[no_mangle]
pub unsafe extern "C" fn ia_get_validate_checksum(
    file_path: *const c_char,
    expected_hash: *const c_char,
    hash_type: *const c_char,
) -> c_int {
    clear_last_error();

    // Validate inputs
    if file_path.is_null() || expected_hash.is_null() || hash_type.is_null() {
        set_last_error("File path, expected hash, and hash type cannot be null");
        return -1;
    }

    // Convert C strings to Rust strings
    let path_str = match CStr::from_ptr(file_path).to_str() {
        Ok(s) => s,
        Err(_) => {
            set_last_error("Invalid UTF-8 in file path");
            return -1;
        }
    };

    let hash_str = match CStr::from_ptr(expected_hash).to_str() {
        Ok(s) => s,
        Err(_) => {
            set_last_error("Invalid UTF-8 in expected hash");
            return -1;
        }
    };

    let type_str = match CStr::from_ptr(hash_type).to_str() {
        Ok(s) => s,
        Err(_) => {
            set_last_error("Invalid UTF-8 in hash type");
            return -1;
        }
    };

    // Call stateless core function
    match stateless::validation::validate_checksum(path_str, hash_str, type_str) {
        Ok(matches) => {
            if matches {
                1
            } else {
                0
            }
        }
        Err(e) => {
            set_last_error(&format!("Validation failed: {}", e));
            -1
        }
    }
}

/// Get last error message
///
/// Returns a pointer to a static string containing the last error message.
/// The returned pointer is valid until the next FFI call in the same thread.
/// DO NOT FREE this pointer.
///
/// # Returns
///
/// * Pointer to error message string (do NOT free)
/// * NULL if no error
#[no_mangle]
pub extern "C" fn ia_get_last_error() -> *const c_char {
    LAST_ERROR.with(|cell| {
        cell.borrow()
            .as_ref()
            .map(|s| s.as_ptr())
            .unwrap_or(ptr::null())
    })
}

/// Free a string returned by this library
///
/// Use this to free strings returned by `ia_get_fetch_metadata()`,
/// `ia_get_decompress_file()`, etc.
///
/// # Arguments
///
/// * `s` - Pointer to string to free (can be NULL)
///
/// # Safety
///
/// The pointer must have been returned by a function in this library.
/// Do NOT use this to free `ia_get_last_error()` results.
#[no_mangle]
pub unsafe extern "C" fn ia_get_free_string(s: *mut c_char) {
    if !s.is_null() {
        let _ = CString::from_raw(s);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_handling() {
        unsafe {
            // Test null input
            let result = ia_get_fetch_metadata(ptr::null());
            assert!(result.is_null());

            let error = ia_get_last_error();
            assert!(!error.is_null());

            let error_msg = CStr::from_ptr(error).to_str().unwrap();
            assert!(error_msg.contains("null"));
        }
    }

    #[test]
    fn test_validate_checksum_null_input() {
        unsafe {
            let result = ia_get_validate_checksum(ptr::null(), ptr::null(), ptr::null());
            assert_eq!(result, -1);

            let error = ia_get_last_error();
            assert!(!error.is_null());
        }
    }

    #[test]
    fn test_free_string_null() {
        // Should not crash
        unsafe {
            ia_get_free_string(ptr::null_mut());
        }
    }
}
