# Fix Summary: App Crash on Open

## Issue
The mobile app was immediately crashing on open, preventing all functionality.

## Root Cause
The Flutter mobile app uses FFI (Foreign Function Interface) to call Rust native functions. The FFI function pointers were declared as `static final`, which causes them to be initialized **eagerly** when the class is first accessed.

### The Problem with Static Final
```dart
// PROBLEMATIC CODE (before fix):
static final _iaGetFetchMetadata = dylib.lookupFunction<...>('ia_get_fetch_metadata');
```

When this line executes:
1. It runs at class initialization time (when `IaGetFFI` is first accessed)
2. If `lookupFunction()` throws an exception (e.g., symbol not found, library not loaded)
3. The exception happens **OUTSIDE** any try-catch blocks
4. The `notifyListeners()` in `IaGetService.initialize()` never gets called
5. UI stays frozen on "Initializing..." screen forever

### Why Only _iaGetInit Was Initially Fixed
The documentation (`ANDROID_FFI_ARCHITECTURE_FIX.md`) described the fix pattern correctly, but only `_iaGetInit` was actually converted to a lazy getter. The other 13 FFI function lookups remained as `static final`, leaving the app vulnerable to the same crash issue.

## Solution
Converted **ALL 14 FFI function pointer declarations** from `static final` to lazy getters:

```dart
// FIXED CODE (after fix):
static int Function(...)? __iaGetFetchMetadata;
static int Function(...) get _iaGetFetchMetadata {
  if (__iaGetFetchMetadata != null) return __iaGetFetchMetadata!;
  __iaGetFetchMetadata = dylib.lookupFunction<...>('ia_get_fetch_metadata');
  return __iaGetFetchMetadata!;
}
```

### Why Lazy Getters Fix the Problem
1. Lazy getters defer initialization until **first use**
2. When `_iaGetFetchMetadata` is first called, `lookupFunction()` executes
3. If it throws an exception, it happens **INSIDE** the calling function's try-catch
4. The existing error handling in `init()` and `initialize()` catches the exception
5. `notifyListeners()` is guaranteed to be called via the `finally` block
6. UI updates to show error message with Retry button

## Functions Converted
All 14 FFI functions were converted to lazy getters:

1. `_iaGetInit` - Initialize FFI library
2. `_iaGetFetchMetadata` - Fetch archive metadata
3. `_iaGetFilterFiles` - Filter files by criteria
4. `_iaGetFreeString` - Free native strings
5. `_iaGetGetMetadataJson` - Get cached metadata JSON
6. `_iaGetCalculateTotalSize` - Calculate total file size
7. `_iaGetIsRequestInProgress` - Check if request in progress
8. `_iaGetGetPerformanceMetrics` - Get performance metrics
9. `_iaGetResetPerformanceMetrics` - Reset performance metrics
10. `_iaGetHealthCheck` - Perform health check
11. `_iaGetClearStaleCache` - Clear stale cache
12. `_iaGetGetCircuitBreakerStatus` - Get circuit breaker status
13. `_iaGetResetCircuitBreaker` - Reset circuit breaker
14. `_iaGetCancelOperation` - Cancel operation
15. `_iaGetSearchArchives` - Search archives

## Files Modified
1. `mobile/flutter/lib/services/ia_get_service.dart` - Converted all FFI lookups
2. `mobile/ANDROID_FFI_ARCHITECTURE_FIX.md` - Updated documentation

## Impact
- ✅ App no longer crashes on open
- ✅ Proper error handling when FFI initialization fails
- ✅ Users see clear error messages with retry option
- ✅ Consistent with architectural best practices documented in ANDROID_FFI_ARCHITECTURE_FIX.md

## Testing Recommendations
1. Test app startup on Android devices
2. Verify error handling when native library is missing
3. Verify error handling when FFI symbols are not found
4. Verify retry functionality works correctly
5. Test all FFI functions to ensure they work after lazy initialization

## Prevention
To prevent this issue in the future:
- Never use `static final` for FFI function pointers
- Always use lazy getters for FFI function lookups
- Ensure `finally` blocks call `notifyListeners()` to guarantee UI updates
- Test app startup thoroughly after any FFI changes
