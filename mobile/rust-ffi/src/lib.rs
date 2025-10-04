//! Mobile FFI Wrapper for ia-get
//!
//! This library re-exports the core ia-get FFI functionality
//! with mobile-optimized defaults and convenience functions.

// Re-export all FFI functions from the main library's simplified FFI interface
pub use ia_get::interface::ffi_simple::*;

// JNI bridge for Android integration
// NOTE: This module is legacy code that references the old FFI interface (14+ functions).
// It is not currently used by the Flutter app, which uses the simplified FFI directly via Dart FFI.
// If JNI integration is needed in the future, this module should be rewritten to use the
// simplified FFI interface (6 functions) or removed entirely.
#[cfg(target_os = "android")]
pub mod jni_bridge;

/// Get library version information
#[no_mangle]
pub extern "C" fn ia_get_mobile_version() -> *const std::os::raw::c_char {
    use std::ffi::CString;
    let version = CString::new(env!("CARGO_PKG_VERSION")).unwrap();
    version.into_raw()
}

/// Get supported architectures
#[no_mangle]
pub extern "C" fn ia_get_mobile_supported_archs() -> *const std::os::raw::c_char {
    use std::ffi::CString;
    let archs = CString::new("arm64-v8a,armeabi-v7a,x86_64,x86").unwrap();
    archs.into_raw()
}
