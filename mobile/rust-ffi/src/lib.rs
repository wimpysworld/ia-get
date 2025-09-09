//! Mobile FFI Wrapper for ia-get
//!
//! This library re-exports the core ia-get FFI functionality
//! with mobile-optimized defaults and convenience functions.

// Re-export all FFI functions from the main library
pub use ia_get::interface::ffi::*;

// JNI bridge for Android integration
#[cfg(target_os = "android")]
pub mod jni_bridge;

/// Initialize the mobile library
/// Simplified initialization for mobile platforms
#[no_mangle]
pub extern "C" fn ia_get_mobile_init() -> ia_get::interface::ffi::IaGetErrorCode {
    ia_get_init()
}

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
