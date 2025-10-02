# Android Search Crash - Root Cause Analysis and Fixes

## Executive Summary

The Android application crash after hitting search is caused by **multiple critical issues**, not just the FFI memory safety that was previously addressed. The crash occurs during the metadata fetch operation, triggered by several compounding problems.

## Critical Issues Identified

### 1. 🔴 CRITICAL: Thread Safety Violation in FFI Callbacks

**Location**: `src/interface/ffi.rs` line 444

**Problem**: 
```rust
std::thread::spawn(move || {
    runtime.block_on(async move {
        // Callbacks are invoked from this background thread
        progress_callback(0.1, msg_ptr, user_data);
        completion_callback(true, ptr::null(), user_data);
    })
})
```

The FFI function spawns a native Rust thread and calls Dart callbacks from that thread. **This is illegal on Android/Flutter** - Dart callbacks MUST be called from the Dart thread (isolate), not from arbitrary native threads.

**Symptoms**:
- App crashes immediately after search button is clicked
- Progress bar starts to move (UI update happens) but then app crashes
- Crash occurs when native callbacks try to execute Dart code

**Why Previous Fixes Didn't Help**:
The previous fixes addressed memory safety (CString lifetime) but didn't address the fundamental threading violation.

**Solution Required**:
- Use Dart's `NativePort` or `SendPort` mechanism to send messages from native threads to Dart isolate
- OR restructure to use Dart's async/await with FFI instead of callbacks
- OR use a polling mechanism (current implementation's `_waitForMetadataCompletion`)

### 2. 🟡 HIGH: Missing Exception Handling in FFI Callbacks

**Location**: `mobile/flutter/lib/services/ia_get_service.dart` lines 545-563

**Problem**: The callbacks don't have try-catch blocks and can crash if:
- String conversion (`toDartString()`) fails on invalid pointers
- Debug printing fails
- Any Dart runtime error occurs

**Status**: ✅ **FIXED** - Added comprehensive try-catch blocks around all callback code

### 3. 🟡 MEDIUM: Callback Execution Timing Issue

**Problem**: The Dart code uses a polling mechanism (`_waitForMetadataCompletion`) but the native callbacks are being invoked on a background thread. This creates a race condition:

1. Native thread spawns and starts metadata fetch
2. Dart code starts polling for results
3. Native callback tries to execute on wrong thread → CRASH
4. Polling never sees the result because callback crashed

**Solution**: 
Since the polling mechanism is already in place and working, the callbacks should be no-ops or removed entirely. The metadata is being stored in the cache by the Rust code, and Dart polls the cache.

### 4. 🟢 LOW: Debug Mode String Operations

**Problem**: Callbacks perform string operations in debug mode that could fail, but this is now handled with try-catch.

## Recommended Fixes (In Priority Order)

### Fix #1: Disable Native Thread Callbacks (Immediate Fix)

The safest immediate fix is to make the callbacks no-ops since the polling mechanism already works:

```dart
// Make callbacks safe no-ops - they're called from native threads which is unsafe
static void _progressCallback(double progress, Pointer<Utf8> message, int userData) {
  // Do nothing - native thread callbacks are unsafe on Android
  // Progress is monitored via polling in _waitForMetadataCompletion
}

static void _completionCallback(bool success, Pointer<Utf8> errorMessage, int userData) {
  // Do nothing - native thread callbacks are unsafe on Android  
  // Completion is detected via polling in _waitForMetadataCompletion
}
```

### Fix #2: Alternative - Use Dart Async/Await Pattern

Restructure to use Dart's FFI async pattern without callbacks:

```dart
// Start async operation
final requestId = IaGetFFI.startFetchMetadata(trimmedIdentifier);

// Poll for completion (current approach)
await _waitForMetadataCompletion(trimmedIdentifier, timeout: const Duration(seconds: 30));

// Get result
final metadataJson = IaGetFFI.getMetadataJson(trimmedIdentifier);
```

This removes the need for callbacks entirely.

### Fix #3: Long-term - Implement Proper Dart-to-Rust Communication

Use Dart's `NativePort` API to safely communicate from native threads:

```dart
final port = ReceivePort();
final nativePort = port.sendPort.nativePort;

// Pass native port to Rust
IaGetFFI.fetchMetadata(identifier, nativePort);

// Listen for messages from Rust
port.listen((message) {
  // Handle progress/completion messages safely on Dart thread
});
```

## Testing Recommendations

1. **Test on Real Android Device**: Emulators may not show the same threading issues
2. **Enable Debug Logging**: Use `adb logcat` to see crash logs
3. **Test with Various Identifiers**: Test valid, invalid, and edge cases
4. **Monitor Thread Crashes**: Look for JNI/thread-related crashes in logs
5. **Test Network Conditions**: Try with WiFi, cellular, and no network

## Why This Wasn't Caught Earlier

1. **Platform-Specific**: This is an Android-specific threading issue that might not occur on iOS
2. **Timing-Dependent**: Race conditions may not always manifest
3. **Debug vs Release**: Behavior can differ between debug and release builds
4. **Device-Specific**: Different Android versions handle threading differently

## Implementation Priority

**IMMEDIATE** (This PR):
- ✅ Fix callback exception handling (already done)
- 🔧 Make callbacks no-ops (safe immediate fix)
- 📝 Document the threading issue

**SHORT-TERM** (Next PR):
- Restructure to remove callbacks entirely
- Use pure polling mechanism

**LONG-TERM** (Future):
- Implement proper NativePort communication
- Add comprehensive threading tests
- Add Android-specific integration tests
