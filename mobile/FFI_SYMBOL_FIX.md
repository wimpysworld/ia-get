# FFI Symbol Lookup Error Fix

## Issue
Users were experiencing the following error when attempting to search or fetch metadata:

```
Failed to fetch metadata: Exception: Failed to lookup symbol
'ia_get_fetch_metadata': undefined symbol:
ia_get_fetch_metadata
```

This occurred regardless of the validity of the search term, preventing all search and metadata functionality from working.

## Root Cause
The mobile FFI wrapper library (`mobile/rust-ffi/src/lib.rs`) was creating duplicate symbol definitions by re-wrapping FFI functions that were already marked with `#[no_mangle]` in the main library.

The issue manifested during release builds with LTO (Link-Time Optimization) enabled:

```rust
// PROBLEMATIC CODE (before fix):
// In mobile/rust-ffi/src/lib.rs
#[no_mangle]
pub unsafe extern "C" fn ia_get_fetch_metadata(identifier: *const c_char) -> *mut c_char {
    ffi_simple::ia_get_fetch_metadata(identifier)  // This function is ALSO #[no_mangle]
}
```

When building with `--release`, the linker would encounter the same symbol defined twice:
1. Once in the main `ia-get` library (from `src/interface/ffi_simple.rs`)
2. Once in the mobile wrapper library (from `mobile/rust-ffi/src/lib.rs`)

This caused the build error:
```
warning: Linking globals named 'ia_get_fetch_metadata': symbol multiply defined!
error: failed to load bitcode of module "ia_get.ia_get.df84771c3f58e6b-cgu.0.rcgu.o"
```

## Solution
Removed the redundant wrapper functions from `mobile/rust-ffi/src/lib.rs`. The mobile library now simply depends on the main library with the `ffi` feature enabled, allowing the symbols to be automatically included in the final cdylib.

```rust
// CORRECT (after fix):
// In mobile/rust-ffi/src/lib.rs
// Re-export the FFI functions from main library
// The main library already has #[no_mangle] on these functions, so we just
// need to ensure they're included in the compilation. We do this by depending
// on the ia-get crate with the ffi feature enabled.
//
// Re-exporting types that Dart/Flutter might need
pub use ia_get::interface::ffi_simple::{IaGetResult, ProgressCallback};

// Only mobile-specific functions are defined here
#[no_mangle]
pub extern "C" fn ia_get_mobile_version() -> *const std::os::raw::c_char {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr() as *const std::os::raw::c_char
}
```

The key insight is that when building a cdylib that depends on another crate with `#[no_mangle]` functions, those symbols are automatically included in the final library. There's no need to re-wrap them.

## Verification
After the fix, all 6 FFI functions plus 2 mobile-specific functions are properly exported:

```
$ nm -D target/release/libia_get_mobile.so | grep ia_get
000000000013919a T ia_get_decompress_file
0000000000138123 T ia_get_download_file
0000000000137858 T ia_get_fetch_metadata      ← The symbol is now present!
000000000013a5b3 T ia_get_free_string
000000000013a57b T ia_get_last_error
00000000000fb201 T ia_get_mobile_supported_archs
00000000000fb1f9 T ia_get_mobile_version
0000000000139c04 T ia_get_validate_checksum
```

## Impact
✅ **Fixed**: Build succeeds in both debug and release modes
✅ **Fixed**: No symbol duplication errors during linking
✅ **Fixed**: All 6 simplified FFI functions are accessible to Flutter/Dart code
✅ **Fixed**: Search and metadata functionality now works
✅ **Fixed**: FFI tests pass

## Technical Details

### Why the Previous Approach Failed
The original approach tried to create wrapper functions in the mobile library:

```rust
// This caused duplicate symbols:
#[no_mangle]
pub unsafe extern "C" fn ia_get_fetch_metadata(identifier: *const c_char) -> *mut c_char {
    ffi_simple::ia_get_fetch_metadata(identifier)
}
```

This looked reasonable but caused issues because:
1. The function `ffi_simple::ia_get_fetch_metadata` is already marked with `#[no_mangle]` in the main library
2. When building a cdylib with LTO enabled, both definitions end up in the final library
3. The linker sees two symbols with the same name and fails

### Why the New Approach Works
By removing the wrapper functions and simply depending on the main library with the `ffi` feature:

```rust
// Cargo.toml
[dependencies]
ia-get = { path = "../../", features = ["ffi"] }
```

The `#[no_mangle]` functions from the main library are automatically included in the mobile cdylib. No wrapper needed.

## Related Documentation
- Main library FFI: `src/interface/ffi_simple.rs`
- Flutter FFI service: `mobile/flutter/lib/services/ia_get_simple_service.dart`
- Simplified FFI architecture: `docs/SIMPLIFIED_FFI_PROGRESS.md` (if exists)

## Note on JNI Bridge
The JNI bridge (`mobile/rust-ffi/src/jni_bridge.rs`) has been updated to import from the correct FFI module (`ffi_simple`). However, it still references old FFI functions (like `ia_get_init`, `ia_get_cleanup`, etc.) that don't exist in the simplified FFI interface. This module is legacy code that's not currently used by the Flutter app (which uses Dart FFI directly via isolates). The module has been marked as deprecated and will need to be rewritten or removed if JNI integration is needed in the future.
