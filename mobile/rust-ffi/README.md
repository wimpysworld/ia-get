# Mobile FFI Wrapper Library

This directory contains a specialized FFI wrapper that re-exports the core ia-get functionality for mobile platforms.

## Structure

- `Cargo.toml` - Mobile-specific build configuration
- `src/lib.rs` - Mobile FFI exports
- `build.rs` - Cross-compilation build script
- `android/` - Android-specific build artifacts

## Building

```bash
# Build for Android targets
./build-android.sh

# Build for iOS targets (future)
./build-ios.sh
```

The build process creates libraries for multiple Android architectures in `target/android/`.