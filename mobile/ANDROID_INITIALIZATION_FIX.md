# Android Initialization Hang Fix

## Issue Summary

The Android application "Internet Archive Helper" was hanging indefinitely on the "Initializing IA Get..." screen, preventing any use of the app.

## Root Cause

The issue was caused by a **JNI package name mismatch** between the Kotlin code and the Rust JNI bridge:

### Package Name Mismatch
- **Kotlin package**: `com.gameaday.internet_archive_helper`
- **JNI functions expected**: `com.gameaday.ia_get_mobile` (encoded as `ia_1get_1mobile` in JNI)

When the Flutter/Dart code called `IaGetFFI.init()`, which in turn called the Kotlin native method `iaGetInit()`, the JNI layer could not find the corresponding native function because:

1. Kotlin was looking for: `Java_com_gameaday_internet_1archive_1helper_IaGetNativeWrapper_iaGetInit`
2. Rust had defined: `Java_com_gameaday_ia_1get_1mobile_IaGetNativeWrapper_iaGetInit`

This mismatch caused the native call to fail silently, leaving `_isInitialized` as `false` and the app stuck showing "Initializing IA Get...".

## Solution

Updated all JNI function names in `mobile/rust-ffi/src/jni_bridge.rs` to use the correct package name:

```rust
// BEFORE (incorrect)
pub extern "system" fn Java_com_gameaday_ia_1get_1mobile_IaGetNativeWrapper_iaGetInit(
    _env: JNIEnv,
    _class: JClass,
) -> jint { ... }

// AFTER (correct)
pub extern "system" fn Java_com_gameaday_internet_1archive_1helper_IaGetNativeWrapper_iaGetInit(
    _env: JNIEnv,
    _class: JClass,
) -> jint { ... }
```

## Functions Updated

The following 13 JNI functions were updated:

1. `iaGetInit` - Initialize the library
2. `iaGetCleanup` - Cleanup the library
3. `iaGetFetchMetadata` - Fetch archive metadata
4. `iaGetGetMetadataJson` - Get cached metadata
5. `iaGetFilterFiles` - Filter files by criteria
6. `iaGetCreateSession` - Create download session
7. `iaGetStartDownload` - Start a download
8. `iaGetPauseDownload` - Pause a download
9. `iaGetResumeDownload` - Resume a download
10. `iaGetCancelDownload` - Cancel a download
11. `iaGetGetDownloadProgress` - Get download progress
12. `iaGetCalculateTotalSize` - Calculate total file size
13. `iaGetFreeString` - Free native string memory

Additionally, the class lookup for `DownloadProgressInfo` was updated to use the correct package path.

## JNI Naming Convention

JNI function names follow this pattern:
```
Java_<package>_<class>_<method>
```

Where:
- Dots (`.`) in package names become underscores (`_`)
- Underscores in package/class names become `_1`
- Forward slashes are not used in function names

Example:
- Package: `com.gameaday.internet_archive_helper`
- Class: `IaGetNativeWrapper`
- Method: `iaGetInit`
- JNI function: `Java_com_gameaday_internet_1archive_1helper_IaGetNativeWrapper_iaGetInit`

Note the `_1` in `internet_1archive_1helper` because the original package name has underscores.

## Testing

The fix was validated with:
- ✅ Cargo build successful
- ✅ Cargo fmt check passed
- ✅ Cargo clippy passed (no warnings)

## App Name Clarification

The app is correctly named "Internet Archive Helper" (also known as "IA Helper") throughout the codebase. This was mentioned in the issue for context but did not require any changes.

## Impact

This fix resolves the initialization hang and allows the Android app to:
1. Successfully initialize the native FFI library
2. Load and display the home screen
3. Access all core functionality (search, download, etc.)

## Related Files

- `mobile/rust-ffi/src/jni_bridge.rs` - JNI bridge implementation
- `mobile/flutter/android/app/src/main/kotlin/com/gameaday/internet_archive_helper/IaGetNativeWrapper.kt` - Kotlin native wrapper
- `mobile/flutter/lib/services/ia_get_service.dart` - Dart FFI service
- `mobile/flutter/lib/main.dart` - App entry point
- `mobile/flutter/lib/screens/home_screen.dart` - Home screen with initialization UI

## Future Considerations

To prevent similar issues in the future:

1. **Consistent naming**: Keep package names consistent across Kotlin and Rust layers
2. **Build validation**: Add tests that verify JNI function linking at build time
3. **Error reporting**: Enhance error messages when JNI function lookup fails
4. **Documentation**: Document the package structure and JNI naming conventions clearly

## References

- [JNI Specification](https://docs.oracle.com/javase/8/docs/technotes/guides/jni/spec/design.html)
- [Android NDK Documentation](https://developer.android.com/ndk/guides)
- [Rust JNI Crate Documentation](https://docs.rs/jni/)
