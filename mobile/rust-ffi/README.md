# Mobile FFI Wrapper Library

This directory contains a specialized FFI wrapper that re-exports the core ia-get functionality for mobile platforms using the **simplified FFI interface**.

## Architecture

The mobile library provides a clean separation between Rust and Flutter:

```
┌─────────────────────────────────────────────────────────┐
│              Flutter Application (State Owner)          │
│  • Dart State Management (all state lives here)        │
│  • Download queue and progress tracking                │
│  • UI state and callbacks                              │
│  • Error handling and retry logic                      │
└─────────────────────────────────────────────────────────┘
                          ↓
         Simple FFI (6 stateless functions)
                          ↓
┌─────────────────────────────────────────────────────────┐
│  Rust Computation Engine (Stateless)                   │
│  • fetch_metadata() → JSON                             │
│  • download_file() with progress callback              │
│  • decompress_file() → extracted files                 │
│  • validate_checksum() → result                        │
│  • last_error() → error message                        │
│  • free_string() → memory management                   │
└─────────────────────────────────────────────────────────┘
```

## Simplified FFI Functions

This library re-exports exactly **6 stateless functions** from the main ia-get library:

1. **`ia_get_fetch_metadata(identifier) → *mut c_char`**
   - Fetches metadata, returns JSON string
   - Caller must free with `ia_get_free_string()`

2. **`ia_get_download_file(url, path, callback, user_data) → IaGetResult`**
   - Downloads file with progress callback
   - Blocking operation (caller uses Dart Isolates)

3. **`ia_get_decompress_file(archive_path, output_dir) → *mut c_char`**
   - Decompresses archive
   - Returns JSON array of extracted file paths

4. **`ia_get_validate_checksum(file_path, expected_hash, hash_type) → c_int`**
   - Validates file checksum
   - Returns 1 (match), 0 (no match), -1 (error)

5. **`ia_get_last_error() → *const c_char`**
   - Returns last error message (thread-local)
   - DO NOT free (static storage)

6. **`ia_get_free_string(s: *mut c_char)`**
   - Frees strings returned by library

## Structure

- `Cargo.toml` - Mobile-specific build configuration (no JNI dependency)
- `src/lib.rs` - Mobile FFI re-exports (simple wrapper)

## Key Principles

- ✅ **Stateless**: No global state, all state managed by Flutter/Dart
- ✅ **Thread-safe**: All functions can be called from any thread
- ✅ **Simple**: Request-response pattern, no complex state management
- ✅ **Memory-safe**: Clear ownership model for strings
- ✅ **No JNI**: Flutter uses Dart FFI directly via `DynamicLibrary`

## Building

```bash
# Build for development
cargo build

# Build for release (optimized for size)
cargo build --release
```

For cross-compilation to Android targets, use the build scripts in the parent mobile directory.

## Integration

Flutter integrates with this library using Dart FFI (`dart:ffi` and `DynamicLibrary`):

```dart
// Load library
final dylib = Platform.isAndroid
    ? DynamicLibrary.open('libia_get_mobile.so')
    : DynamicLibrary.process();

// Bind functions
final fetchMetadata = dylib.lookupFunction<
    Pointer<Utf8> Function(Pointer<Utf8>),
    Pointer<Utf8> Function(Pointer<Utf8>)>('ia_get_fetch_metadata');

// Use in isolates for blocking operations
await Isolate.run(() => _fetchMetadataIsolate(identifier));
```

See `mobile/flutter/lib/services/ia_get_simple_service.dart` for the complete integration.

## Benefits of This Architecture

- **57% reduction** in FFI complexity (14+ → 6 functions)
- No state management in FFI layer = no race conditions
- Simple request-response pattern
- Easy to test and debug
- Clear separation of concerns
