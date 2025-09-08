# Android Build Fix Documentation

## Issue Summary

The Android cross-compilation builds were failing with the following error:

```
warning: ring@0.17.14: Compiler family detection failed due to error: ToolNotFound: failed to find tool "aarch64-linux-android-clang": No such file or directory (os error 2)
error: failed to run custom build command for `ring v0.17.14`
```

## Root Cause

The `ring` crate (used by dependencies like `rustls`) was looking for generic Android NDK compiler names like `aarch64-linux-android-clang`, but modern Android NDK versions only provide API-level-specific compilers like `aarch64-linux-android21-clang`, `aarch64-linux-android23-clang`, etc.

## Solution

Updated the `scripts/build-android.sh` script to properly configure cross-compilation environment variables:

### Environment Variables Added

1. **Compiler Variables (CC_*)**:
   - `CC_aarch64_linux_android` → points to `aarch64-linux-android21-clang`
   - `CC_armv7_linux_androideabi` → points to `armv7a-linux-androideabi21-clang`
   - `CC_x86_64_linux_android` → points to `x86_64-linux-android21-clang`
   - `CC_i686_linux_android` → points to `i686-linux-android21-clang`

2. **Archiver Variables (AR_*)**:
   - All targets point to `llvm-ar` from the NDK

3. **Linker Variables (CARGO_TARGET_*_LINKER)**:
   - Configured for proper final linking phase

### Validation Added

- Check for `ANDROID_NDK_HOME` environment variable
- Verify NDK directory exists
- Validate all required compilers are present
- Clear error messages when NDK is not properly set up

## Results

All four Android targets now build successfully:
- ✅ `aarch64-linux-android` (ARM64)
- ✅ `armv7-linux-androideabi` (ARM32)
- ✅ `x86_64-linux-android` (x86_64)
- ✅ `i686-linux-android` (x86)

## Usage

The Android build script now works correctly in environments where:
1. Android NDK is properly installed
2. `ANDROID_NDK_HOME` environment variable is set
3. Required Android targets are added to Rust toolchain

```bash
# Ensure NDK is configured
export ANDROID_NDK_HOME=$ANDROID_HOME/ndk/27.3.13750724

# Run the build script
./scripts/build-android.sh
```

## Environment Variables

The script uses `ANDROID_API_LEVEL=21` by default (Android 5.0), which provides good compatibility while supporting modern features. This can be overridden if needed:

```bash
export ANDROID_API_LEVEL=23
./scripts/build-android.sh
```

## Dependencies Fixed

This fix resolves build issues for any Rust crate that uses native compilation, including:
- `ring` (cryptographic library)
- `zstd-sys` (compression)
- `liblzma-sys` (compression)
- Other crates with native build scripts