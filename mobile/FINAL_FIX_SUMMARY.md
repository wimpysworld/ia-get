# Final Fix Summary: FFI Symbol Duplication Issue

## Problem Statement
Users reported the error:
```
Failed to fetch metadata: Exception: Failed to lookup symbol 'ia_get_fetch_metadata': undefined symbol: ia_get_fetch_metadata
```

This error prevented all search and metadata operations in the Flutter mobile app.

## Root Cause Analysis

### The Real Issue
The problem was **NOT** an import path issue as previously documented. The actual issue was **symbol duplication** during release builds with Link-Time Optimization (LTO).

When building the mobile FFI library in release mode, we encountered:
```
warning: Linking globals named 'ia_get_fetch_metadata': symbol multiply defined!
error: failed to load bitcode of module "ia_get.ia_get.df84771c3f58e6b-cgu.0.rcgu.o"
```

### Why It Happened
The mobile wrapper library (`mobile/rust-ffi/src/lib.rs`) was creating wrapper functions:

```rust
// PROBLEMATIC CODE:
#[no_mangle]
pub unsafe extern "C" fn ia_get_fetch_metadata(identifier: *const c_char) -> *mut c_char {
    ffi_simple::ia_get_fetch_metadata(identifier)
}
```

However, the function being called (`ffi_simple::ia_get_fetch_metadata`) is **ALSO** marked with `#[no_mangle]` in the main library (`src/interface/ffi_simple.rs`).

When building a `cdylib` with LTO enabled (which happens in release builds), both symbols end up in the final library, causing the linker error.

### Why Debug Builds Worked
In debug builds without LTO, the linker doesn't merge code units as aggressively, so the duplicate symbols might not conflict. However, release builds with `lto = true` always failed.

## The Solution

### What We Changed
We simplified `mobile/rust-ffi/src/lib.rs` to remove all wrapper functions and just re-export types:

```rust
// FIXED CODE:
// Re-export the FFI functions from main library
// The main library already has #[no_mangle] on these functions, so we just
// need to ensure they're included in the compilation.
pub use ia_get::interface::ffi_simple::{IaGetResult, ProgressCallback};

// Only define mobile-specific functions
#[no_mangle]
pub extern "C" fn ia_get_mobile_version() -> *const std::os::raw::c_char {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr() as *const std::os::raw::c_char
}
```

### Why This Works
When you build a `cdylib` that depends on another crate with `#[no_mangle]` functions:
1. The dependency is compiled as an `rlib` (Rust library)
2. The `#[no_mangle]` symbols from the dependency are automatically included in the final `cdylib`
3. No wrapper functions are needed - the symbols are already there!

This is the standard way to re-export FFI functions in Rust.

## Verification

### Build Success
Both debug and release builds now succeed:
```bash
$ cargo build --manifest-path mobile/rust-ffi/Cargo.toml --lib --release
   Finished `release` profile [optimized] target(s) in 13.25s
```

### Symbol Verification
All required symbols are exported:
```bash
$ nm -D target/release/libia_get_mobile.so | grep ia_get
000000000013919a T ia_get_decompress_file
0000000000138123 T ia_get_download_file
0000000000137858 T ia_get_fetch_metadata      ✅
000000000013a5b3 T ia_get_free_string
000000000013a57b T ia_get_last_error
00000000000fb201 T ia_get_mobile_supported_archs
00000000000fb1f9 T ia_get_mobile_version
0000000000139c04 T ia_get_validate_checksum
```

### Test Results
All tests pass:
- FFI unit tests: 33/33 ✅
- Integration tests: All pass ✅
- Total: 77+ tests ✅

### Code Quality
- Clippy: No warnings ✅
- Formatting: Consistent with `cargo fmt` ✅

## Files Modified
1. **mobile/rust-ffi/src/lib.rs** - Removed redundant wrapper functions (reduced from ~113 lines to ~61 lines)
2. **mobile/FFI_SYMBOL_FIX.md** - Updated documentation to reflect actual fix

## Impact
✅ **Fixed**: Release builds succeed without linker errors  
✅ **Fixed**: All FFI symbols properly exported  
✅ **Fixed**: Search and metadata functionality works  
✅ **Fixed**: No code duplication or maintenance burden  
✅ **Improved**: Cleaner, more maintainable code  
✅ **Improved**: Better alignment with Rust FFI best practices  

## Lessons Learned

1. **Don't wrap #[no_mangle] functions**: If a dependency already exports FFI symbols, don't re-wrap them. The symbols are automatically included in the final `cdylib`.

2. **Test release builds**: Always test with `--release` as LTO can expose issues that don't appear in debug builds.

3. **Use standard patterns**: The Rust FFI ecosystem has established patterns. Follow them rather than creating custom wrappers.

4. **Minimal is better**: The fix actually involved *removing* code, not adding it. Less code = less complexity = fewer bugs.

## Related Issues
This fix resolves the issue reported in: "Fix search - Failed to fetch metadata: undefined symbol: ia_get_fetch_metadata"
