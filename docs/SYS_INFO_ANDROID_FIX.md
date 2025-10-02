# Android sys-info Build Fix

## Problem

The `sys-info` crate v0.9.1 fails to compile for Android targets due to missing platform-specific functions in the Android NDK:

```
error: call to undeclared function 'get_nprocs'
error: call to undeclared library function 'index'
```

These C functions are not available in Android's Bionic libc, causing build failures for all Android architectures:
- `aarch64-linux-android` (ARM64)
- `armv7-linux-androideabi` (ARM32)
- `x86_64-linux-android` (x86_64)
- `i686-linux-android` (x86)

## Root Cause

The `sys-info` crate uses platform-specific C code that relies on glibc functions not present in Android's NDK. The crate is used in this project to check available disk space before downloading files.

## Solution

Made `sys-info` a platform-conditional dependency using Cargo's target-specific dependencies feature:

### 1. Cargo.toml Changes

```toml
[target.'cfg(not(target_os = "android"))'.dependencies]
sys-info = "0.9.1"
```

This ensures `sys-info` is only compiled for non-Android platforms.

### 2. Code Changes (utils.rs)

Updated `get_available_disk_space()` to use conditional compilation:

```rust
pub fn get_available_disk_space<P: AsRef<std::path::Path>>(_path: P) -> Option<u64> {
    #[cfg(not(target_os = "android"))]
    {
        use sys_info::disk_info;
        // ... original implementation
    }

    #[cfg(target_os = "android")]
    {
        // On Android, return None to skip disk space checks
        None
    }
}
```

## Impact

- **Non-Android platforms**: No change in behavior - disk space checks work as before
- **Android platforms**: Disk space checks are skipped (function returns `None`)
- **Download service**: Gracefully handles `None` by skipping disk space validation

This is an acceptable trade-off for mobile platforms where:
1. Disk space APIs may not be reliably available
2. The OS typically manages storage more actively
3. Users expect apps to work within system constraints

## Testing

All tests pass and Android builds succeed:

```bash
# Verified on multiple Android targets
cargo build --target aarch64-linux-android --release --features ffi
cargo build --target x86_64-linux-android --release --features ffi

# All 93 tests pass
cargo test
```

## Alternative Solutions Considered

1. **Replace sys-info with sysinfo crate**: Would require more code changes and testing
2. **Fork sys-info with Android support**: Maintenance burden and ongoing sync issues
3. **Use libc directly for Android**: Complex, platform-specific implementation

The chosen solution is minimal, maintainable, and follows Rust ecosystem best practices for platform-specific dependencies.

## Related Documentation

- [ANDROID_BUILD_FIX.md](./ANDROID_BUILD_FIX.md) - Previous Android NDK configuration fixes
- [Cargo Target-Specific Dependencies](https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html#platform-specific-dependencies)
