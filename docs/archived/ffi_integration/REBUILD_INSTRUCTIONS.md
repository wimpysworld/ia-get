# How to Rebuild the Mobile App After FFI Symbol Fix

## Issue
If you're experiencing the following error when trying to search or fetch metadata:

```
Failed to fetch metadata: Exception: Failed to lookup symbol
'ia_get_fetch_metadata': undefined symbol:
ia_get_fetch_metadata
```

This means you're running an older version of the app that was built before the FFI symbol export fix was applied.

## Solution
You need to rebuild the mobile app to get the updated native library with the fixed FFI re-exports.

## Rebuild Instructions

### Quick Rebuild (Recommended)

1. **Clean previous builds:**
   ```bash
   cd mobile/flutter
   flutter clean
   cd ../..
   rm -rf mobile/rust-ffi/target
   ```

2. **Rebuild the mobile app:**
   ```bash
   ./scripts/build-mobile.sh --dev
   ```

3. **Install on your device:**
   ```bash
   cd mobile/flutter
   flutter install --flavor development
   ```

### Full Clean Rebuild

If you're still experiencing issues after the quick rebuild, try a full clean rebuild:

1. **Clean all build artifacts:**
   ```bash
   # Clean Rust builds
   cargo clean
   rm -rf mobile/rust-ffi/target
   
   # Clean Flutter builds
   cd mobile/flutter
   flutter clean
   rm -rf android/app/src/main/jniLibs
   cd ../..
   ```

2. **Rebuild native libraries:**
   ```bash
   ./scripts/build-android-libs-only.sh
   ```

3. **Rebuild Flutter app:**
   ```bash
   cd mobile/flutter
   flutter pub get
   flutter build apk --debug --flavor development
   ```

4. **Install:**
   ```bash
   flutter install --flavor development
   ```

## Verification

After rebuilding, the app should:
- ✅ Load without errors
- ✅ Allow searching for Internet Archive items
- ✅ Display metadata for items
- ✅ Download files successfully

If you still experience issues, check the logcat output:
```bash
adb logcat | grep -i "ia_get\|native"
```

Look for messages indicating whether the native library was loaded successfully.

## Technical Details

### What Was Fixed

The FFI re-export in `mobile/rust-ffi/src/lib.rs` was changed from:
```rust
pub use ia_get::interface::ffi_simple::*;
```

To:
```rust
pub use ia_get::interface::ffi_simple::{
    ia_get_decompress_file, ia_get_download_file, ia_get_fetch_metadata, ia_get_free_string,
    ia_get_last_error, ia_get_validate_checksum, IaGetResult,
};
```

This ensures all FFI functions are explicitly listed and properly exported from the mobile wrapper library.

### How to Verify the Fix

You can verify that the fix is applied by checking the exported symbols in the compiled library:

```bash
nm -D mobile/rust-ffi/target/release/libia_get_mobile.so | grep ia_get
```

You should see all these symbols:
- `ia_get_decompress_file`
- `ia_get_download_file`
- `ia_get_fetch_metadata`
- `ia_get_free_string`
- `ia_get_last_error`
- `ia_get_mobile_supported_archs`
- `ia_get_mobile_version`
- `ia_get_validate_checksum`

## Need Help?

If you continue to experience issues after rebuilding:

1. Check that you're using the latest code from the main branch
2. Ensure your Rust and Flutter toolchains are up to date
3. Try building in release mode instead of debug: `--release` instead of `--debug`
4. Open an issue on GitHub with:
   - Your build output
   - Logcat output
   - Device information (architecture, Android version)

## Related Documentation

- [FFI Symbol Fix Details](./FFI_SYMBOL_FIX.md)
- [Build Script Documentation](../scripts/README.md)
- [Mobile Development Guide](../docs/MOBILE_DEVELOPMENT_GUIDE.md)
