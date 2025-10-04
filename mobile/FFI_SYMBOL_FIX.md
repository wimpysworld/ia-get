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
The mobile FFI wrapper library (`mobile/rust-ffi/src/lib.rs`) was using an incorrect import path:

```rust
// INCORRECT (before fix):
pub use ia_get::interface::ffi::*;
```

However, the FFI interface module was renamed to `ffi_simple` as part of the simplified FFI architecture introduced in version 0.8.0+. The old complex FFI interface (with 14+ functions) was removed and replaced with a simplified 6-function interface.

This mismatch caused:
1. The Rust compiler to fail building the mobile library with the old import
2. No FFI symbols to be exported from the mobile library
3. Flutter/Dart code unable to find `ia_get_fetch_metadata` and other FFI functions
4. All metadata and search operations to fail

## Solution
Updated the import path in `mobile/rust-ffi/src/lib.rs`:

```rust
// CORRECT (after fix):
pub use ia_get::interface::ffi_simple::*;
```

Additionally removed the incompatible `ia_get_mobile_init()` function that was calling a non-existent `ia_get_init()` function. The simplified FFI doesn't have an initialization function - it's stateless by design.

## Verification
After the fix, all 6 FFI functions are properly exported:

```
$ nm -D target/release/libia_get_mobile.so | grep ia_get
00000000001392be T ia_get_decompress_file
0000000000138247 T ia_get_download_file
0000000000137a10 T ia_get_fetch_metadata      ← The missing symbol is now present!
000000000013a6d7 T ia_get_free_string
000000000013a69f T ia_get_last_error
00000000000fb502 T ia_get_mobile_supported_archs
00000000000fb4a4 T ia_get_mobile_version
0000000000139d28 T ia_get_validate_checksum
```

## Impact
✅ **Fixed**: Search and metadata functionality now works
✅ **Fixed**: All 6 simplified FFI functions are accessible to Flutter/Dart code
✅ **Fixed**: Both debug and release builds succeed
✅ **Fixed**: FFI tests pass

## Related Documentation
- Main library FFI: `src/interface/ffi_simple.rs`
- Flutter FFI service: `mobile/flutter/lib/services/ia_get_simple_service.dart`
- Simplified FFI architecture: `docs/SIMPLIFIED_FFI_PROGRESS.md` (if exists)

## Note on JNI Bridge
The JNI bridge (`mobile/rust-ffi/src/jni_bridge.rs`) has been updated to import from the correct FFI module (`ffi_simple`). However, it still references old FFI functions (like `ia_get_init`, `ia_get_cleanup`, etc.) that don't exist in the simplified FFI interface. This module is legacy code that's not currently used by the Flutter app (which uses Dart FFI directly via isolates). The module has been marked as deprecated and will need to be rewritten or removed if JNI integration is needed in the future.
