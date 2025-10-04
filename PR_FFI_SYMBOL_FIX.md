# PR Summary: Fix FFI Symbol Lookup Error

## Issue
Users reported the following error when trying to search or fetch metadata in the mobile app:

```
Failed to fetch metadata: Exception: Failed to lookup symbol
'ia_get_fetch_metadata': undefined symbol:
ia_get_fetch_metadata
```

## Root Cause Analysis

The mobile FFI wrapper library (`mobile/rust-ffi/src/lib.rs`) was using a wildcard re-export:

```rust
pub use ia_get::interface::ffi_simple::*;
```

While this should technically work, using wildcard re-exports with FFI functions can sometimes lead to issues with symbol visibility, especially when building across different architectures and with aggressive optimization settings (LTO, stripping, etc.).

## Solution

Made the FFI re-exports explicit by listing each function individually:

```rust
pub use ia_get::interface::ffi_simple::{
    ia_get_decompress_file, 
    ia_get_download_file, 
    ia_get_fetch_metadata, 
    ia_get_free_string,
    ia_get_last_error, 
    ia_get_validate_checksum, 
    IaGetResult,
};
```

This ensures:
1. Maximum clarity about what's being exported
2. Better compatibility with the linker and optimization passes
3. Easier debugging if issues arise
4. Follows Rust best practices for explicit exports

## Files Changed

1. **mobile/rust-ffi/src/lib.rs**
   - Changed from wildcard to explicit re-exports
   - Added comment explaining the approach
   - Formatted with cargo fmt

2. **mobile/rust-ffi/src/jni_bridge.rs**
   - Minor formatting changes (cargo fmt)
   - No functional changes

3. **mobile/FFI_SYMBOL_FIX.md**
   - Updated documentation to reflect explicit re-exports
   - Clarified the solution approach

4. **mobile/REBUILD_INSTRUCTIONS.md** (new)
   - Comprehensive guide for users experiencing the issue
   - Step-by-step rebuild instructions
   - Verification steps
   - Troubleshooting guidance

## Verification

All 8 FFI symbols verified as exported:
```bash
$ nm -D mobile/rust-ffi/target/release/libia_get_mobile.so | grep ia_get
ia_get_decompress_file      ✓
ia_get_download_file        ✓
ia_get_fetch_metadata       ✓
ia_get_free_string          ✓
ia_get_last_error           ✓
ia_get_mobile_supported_archs ✓
ia_get_mobile_version       ✓
ia_get_validate_checksum    ✓
```

Symbol loading tested successfully using dlopen/dlsym.

## Testing

- ✅ All library tests pass
- ✅ Code formatted with `cargo fmt`
- ✅ No clippy warnings
- ✅ Release build successful (2.6MB stripped)
- ✅ All FFI symbols present in binary
- ✅ Symbol lookup test passes

## Impact

**Minimal code change, maximum reliability:**
- Only changed the re-export statement in one file
- No changes to the actual FFI function implementations
- No changes to the Dart/Flutter code
- Fully backward compatible

**Users need to rebuild:**
- Existing installations will continue to have the issue
- Users must rebuild the app to get the fix
- See `mobile/REBUILD_INSTRUCTIONS.md` for guidance

## Recommendations

For users experiencing this issue:

1. **Quick fix:** Rebuild the mobile app
   ```bash
   ./scripts/build-mobile.sh --dev
   ```

2. **If issues persist:** Clean rebuild
   ```bash
   flutter clean
   rm -rf mobile/rust-ffi/target
   ./scripts/build-mobile.sh --dev
   ```

3. **Verification:** Check logcat for library loading messages
   ```bash
   adb logcat | grep -i "ia_get\|native"
   ```

## Related Issues

This fix addresses the core issue described in `mobile/FFI_SYMBOL_FIX.md` with a more robust approach. The explicit re-export strategy ensures maximum compatibility across different build configurations and optimization levels.

## Next Steps

- Monitor for any additional reports of this issue
- Consider adding automated tests that verify FFI symbols are exported
- Document FFI best practices for future development
