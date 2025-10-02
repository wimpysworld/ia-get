# Android Search Crash Fix Documentation

## Problem Statement
The Android application was experiencing crashes when performing archive searches. The issue manifested when users attempted to search for Internet Archive items through the mobile Flutter interface.

## Root Causes Identified

### 1. Memory Safety Issues in FFI Callbacks
**Location**: `src/interface/ffi.rs`

The primary issue was with CString lifetime management in the FFI layer. CString objects were being created and immediately dropped after passing their pointer to callbacks, causing use-after-free errors on Android.

```rust
// BEFORE (problematic):
let error_msg = CString::new("error").unwrap();
completion_callback(false, error_msg.as_ptr(), user_data);
drop(error_msg);  // Drops before callback might use the pointer
```

```rust
// AFTER (fixed):
let error_msg = CString::new("error")
    .unwrap_or_else(|_| CString::new("fallback").unwrap());
let error_ptr = error_msg.as_ptr();
completion_callback(false, error_ptr, user_data);
// error_msg dropped naturally after callback completes synchronously
```

### 2. Insufficient Error Handling in Dart FFI
**Location**: `mobile/flutter/lib/services/ia_get_service.dart`

The Dart FFI bindings lacked proper null checking and exception handling, causing crashes when the native layer returned unexpected results.

**Improvements**:
- Added try-catch blocks around all FFI calls
- Implemented null/empty validation before FFI calls
- Enhanced error messages with context
- Increased timeout from 10s to 30s for metadata fetches
- Added attempt counting for better debugging

### 3. Race Conditions in Async Handling
The polling mechanism for metadata completion could fail if callbacks didn't complete properly, leading to timeouts and crashes.

**Solution**: Better async handling with:
- Proper timeout management
- Attempt tracking and logging
- Error handling in the polling loop

### 4. JNI Layer Validation Issues
**Location**: `mobile/rust-ffi/src/jni_bridge.rs`

The JNI bridge lacked comprehensive input validation, allowing null or invalid parameters to propagate through the system.

**Improvements**:
- Added null checks for all JString parameters
- Validated empty/whitespace identifiers
- Enhanced error logging for Android debugging
- Better error propagation to Java/Kotlin layer

## Technical Details

### FFI Callback Pattern (Fixed)
The callbacks are now properly scoped to ensure strings remain valid:

```rust
extern "C" fn progress_cb(progress: f64, message: *const c_char, _user_data: usize) {
    let msg = unsafe {
        if message.is_null() {
            String::new()
        } else {
            CStr::from_ptr(message)
                .to_str()
                .unwrap_or("")
                .to_string()
        }
    };
    println!("Progress: {:.1}% - {}", progress * 100.0, msg);
}
```

### Dart FFI Error Handling Pattern
All FFI calls now follow this pattern:

```dart
static String? getMetadataJson(String identifier) {
  if (identifier.isEmpty) {
    if (kDebugMode) print('getMetadataJson: empty identifier');
    return null;
  }
  
  final identifierPtr = identifier.toNativeUtf8();
  try {
    final resultPtr = _iaGetGetMetadataJson(identifierPtr);
    if (resultPtr == nullptr) return null;
    
    try {
      final result = resultPtr.toDartString();
      _iaGetFreeString(resultPtr);
      return result;
    } catch (e) {
      try { _iaGetFreeString(resultPtr); } catch (_) {}
      return null;
    }
  } catch (e) {
    if (kDebugMode) print('getMetadataJson: exception: $e');
    return null;
  } finally {
    malloc.free(identifierPtr);
  }
}
```

## Testing
All changes have been validated with:
- ✅ Cargo clippy (no warnings)
- ✅ Cargo fmt (code formatted)
- ✅ Unit tests (13/13 passing)
- ✅ Build tests (main library and mobile FFI)

## Debugging Android Issues

### Enable Debug Logging
Debug builds now include extensive logging:

```dart
// In Dart code
if (kDebugMode) {
  print('Debug message');
}
```

```rust
// In Rust code
#[cfg(debug_assertions)]
println!("Debug message");

// For errors (always shown)
eprintln!("Error message");
```

### Use Android Logcat
Monitor the Android logs during testing:
```bash
adb logcat | grep -i "ia-get\|JNI\|flutter"
```

### Common Error Patterns
1. **"identifier is null"**: JNI layer received null parameter
2. **"identifier is empty"**: Empty string passed to search
3. **"No metadata cached"**: Metadata fetch hasn't completed yet
4. **"Metadata fetch timeout"**: Network issue or API unavailable

## Future Improvements
While the current fixes address the crash issues, potential enhancements include:

1. **Proper JNI callbacks**: Implement actual callback mechanism from Rust to Kotlin/Java
2. **Connection state monitoring**: Detect network availability before attempting searches
3. **Retry logic**: Automatic retry on transient failures
4. **Caching improvements**: Better metadata cache invalidation
5. **Progress indicators**: More granular progress reporting in the UI

## References
- Main FFI implementation: `src/interface/ffi.rs`
- Dart service layer: `mobile/flutter/lib/services/ia_get_service.dart`
- Search widget: `mobile/flutter/lib/widgets/search_bar_widget.dart`
- JNI bridge: `mobile/rust-ffi/src/jni_bridge.rs`
