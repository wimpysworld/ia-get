# Mobile FFI Wrapper - README

This directory contains the Rust FFI wrapper for the mobile (Flutter) application.

## Overview

The mobile app uses Dart FFI to call Rust functions directly. This provides:
- High performance for CPU-intensive operations
- Native networking and file I/O
- Shared codebase with the CLI tool

## Architecture

```
Flutter/Dart App
    ↓ (Dart FFI)
mobile/rust-ffi/ (this library)
    ↓ (Rust dependency)
ia-get core library (src/interface/ffi_simple.rs)
```

### Key Design Principles

1. **No Symbol Duplication**: The mobile library depends on the main library with the `ffi` feature. FFI symbols from the main library are automatically included in the final `cdylib`. No wrapper functions needed!

2. **Stateless FFI**: All state management happens in Dart. The FFI layer is purely functional.

3. **Minimal Wrapper**: This library only adds mobile-specific functions (version, supported architectures). All core FFI functions come from the main library.

## Building

### Debug Build
```bash
cargo build --manifest-path mobile/rust-ffi/Cargo.toml --lib
```

### Release Build
```bash
cargo build --manifest-path mobile/rust-ffi/Cargo.toml --lib --release
```

### Android Cross-Compilation
```bash
# Add Android targets
rustup target add aarch64-linux-android
rustup target add armv7-linux-androideabi
rustup target add x86_64-linux-android

# Build for each architecture
cargo build --target aarch64-linux-android --release --lib
cargo build --target armv7-linux-androideabi --release --lib
cargo build --target x86_64-linux-android --release --lib

# Libraries will be in:
# target/aarch64-linux-android/release/libia_get_mobile.so
# target/armv7-linux-androideabi/release/libia_get_mobile.so
# target/x86_64-linux-android/release/libia_get_mobile.so
```

Copy these to:
```
mobile/flutter/android/app/src/main/jniLibs/
  arm64-v8a/libia_get_mobile.so
  armeabi-v7a/libia_get_mobile.so
  x86_64/libia_get_mobile.so
```

## Verification

Run the automated verification script to ensure all symbols are properly exported:

```bash
./mobile/verify_ffi_symbols.sh
```

This script:
- Builds both debug and release versions
- Verifies all 8 FFI symbols are exported
- Runs all FFI tests
- Checks code quality with clippy

## Exported Symbols

### Core FFI Functions (from main library)
1. `ia_get_fetch_metadata` - Fetch archive metadata as JSON
2. `ia_get_download_file` - Download a file with progress callback
3. `ia_get_decompress_file` - Decompress an archive file
4. `ia_get_validate_checksum` - Validate file checksum
5. `ia_get_last_error` - Get last error message
6. `ia_get_free_string` - Free strings returned by library

### Mobile-Specific Functions (defined here)
7. `ia_get_mobile_version` - Get library version
8. `ia_get_mobile_supported_archs` - Get supported Android architectures

## Troubleshooting

### Symbol Not Found Errors
If you see "undefined symbol" errors:
1. Verify the library is built: `cargo build --manifest-path mobile/rust-ffi/Cargo.toml --lib --release`
2. Check symbols are exported: `nm -D target/release/libia_get_mobile.so | grep ia_get`
3. Ensure the Flutter app is loading the correct library path

### Release Build Failures
If release builds fail with "symbol multiply defined":
- ❌ **Don't** create wrapper functions with `#[no_mangle]` that call other `#[no_mangle]` functions
- ✅ **Do** let the dependency's symbols be automatically included in the final cdylib

### LTO Issues
The mobile library is built with LTO (`lto = true`) for size optimization. This can expose symbol duplication issues that don't appear in debug builds. Always test release builds!

## Documentation

- `FFI_SYMBOL_FIX.md` - Detailed explanation of the symbol duplication fix
- `FINAL_FIX_SUMMARY.md` - Comprehensive summary of the FFI architecture and fix
- `../flutter/lib/services/ia_get_simple_service.dart` - Dart FFI bindings

## Testing

Run FFI tests from the root directory:
```bash
cargo test --lib --features ffi
```

## Common Tasks

### Update FFI Interface
If you need to add new FFI functions:
1. Add the function to `src/interface/ffi_simple.rs` in the main library
2. Mark it with `#[no_mangle]`
3. The symbol will automatically be available in the mobile library (no changes needed here!)
4. Update the Dart bindings in `mobile/flutter/lib/services/ia_get_simple_service.dart`

### Check Symbol Exports
```bash
nm -D mobile/rust-ffi/target/release/libia_get_mobile.so | grep ia_get
```

### Size Optimization
The mobile library is built with aggressive size optimization:
- `opt-level = "z"` (optimize for size)
- `lto = true` (Link-Time Optimization)
- `strip = true` (remove debug symbols)
- `codegen-units = 1` (better optimization)

This results in a much smaller library for mobile deployment.

## Flutter Integration

The Flutter app loads this library using `DynamicLibrary.process()` on Android/iOS. See `mobile/flutter/lib/services/ia_get_simple_service.dart` for the Dart FFI bindings.

## Contributing

When making changes to the FFI layer:
1. Keep functions simple and stateless
2. Avoid creating wrapper functions (let symbols pass through)
3. Always build and test in release mode
4. Run the verification script before committing
5. Update documentation if adding new functions

## License

Same as the main ia-get project. See LICENSE in the root directory.
