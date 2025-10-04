# Simplified FFI Interface for Flutter Integration

This directory contains the C header and documentation for the simplified FFI interface to integrate ia-get with Flutter.

## Overview

The simplified FFI provides **only 6 functions** for stateless operations, dramatically reducing complexity compared to the previous 14+ functions.

### Key Features

- ✅ **Stateless**: No global state, all state managed by caller (Flutter/Dart)
- ✅ **Thread-safe**: All functions can be called from any thread
- ✅ **Simple**: Request-response pattern, no complex state management
- ✅ **Memory-safe**: Clear ownership model for strings

## Files

- **`ia_get_simple.h`** - C header file with all function declarations
- **`README.md`** - This file

## The 6 Core Functions

### 1. Fetch Metadata

```c
char* ia_get_fetch_metadata(const char* identifier);
```

Fetches metadata for an archive.org item, returns JSON string.  
**Must free** with `ia_get_free_string()`.

### 2. Download File

```c
IaGetResult ia_get_download_file(
    const char* url,
    const char* output_path,
    ProgressCallback progress_callback,
    void* user_data
);
```

Downloads a file with optional progress callback.  
**Blocking operation** - use Dart Isolates!

### 3. Decompress Archive

```c
char* ia_get_decompress_file(const char* archive_path, const char* output_dir);
```

Decompresses archive (zip, tar, gzip, etc.), returns JSON array of extracted files.  
**Must free** with `ia_get_free_string()`.

### 4. Validate Checksum

```c
int ia_get_validate_checksum(
    const char* file_path,
    const char* expected_hash,
    const char* hash_type
);
```

Validates file checksum. Returns:
- `1` - Hash matches
- `0` - Hash mismatch
- `-1` - Error (check `ia_get_last_error()`)

### 5. Get Last Error

```c
const char* ia_get_last_error(void);
```

Returns last error message (thread-local).  
**Do NOT free** this pointer.

### 6. Free String

```c
void ia_get_free_string(char* s);
```

Frees strings returned by `ia_get_fetch_metadata()` and `ia_get_decompress_file()`.

## Flutter/Dart Integration Guide

### Step 1: Generate Dart FFI Bindings

Use `ffigen` to generate Dart bindings from the C header:

```yaml
# pubspec.yaml
dev_dependencies:
  ffigen: ^latest

# ffigen_config.yaml
name: IaGetSimpleBindings
description: FFI bindings for ia-get simplified interface
output: lib/src/ffi/ia_get_bindings.dart
headers:
  entry-points:
    - '../rust/include/ia_get_simple.h'
```

Run: `dart run ffigen --config ffigen_config.yaml`

### Step 2: Load the Rust Library

```dart
import 'dart:ffi' as ffi;
import 'dart:io';
import 'package:path/path.dart' as path;

class IaGetNative {
  static final ffi.DynamicLibrary _lib = _loadLibrary();
  
  static ffi.DynamicLibrary _loadLibrary() {
    if (Platform.isAndroid) {
      return ffi.DynamicLibrary.open('libia_get.so');
    } else if (Platform.isIOS) {
      return ffi.DynamicLibrary.process();
    } else if (Platform.isLinux) {
      return ffi.DynamicLibrary.open('libia_get.so');
    } else if (Platform.isMacOS) {
      return ffi.DynamicLibrary.open('libia_get.dylib');
    } else if (Platform.isWindows) {
      return ffi.DynamicLibrary.open('ia_get.dll');
    }
    throw UnsupportedError('Platform not supported');
  }
  
  static final IaGetSimpleBindings bindings = IaGetSimpleBindings(_lib);
}
```

### Step 3: Use Isolates for Blocking Operations

**IMPORTANT**: All FFI calls are blocking! Use Dart Isolates:

```dart
import 'dart:isolate';

// Top-level function for isolate
Future<String> _fetchMetadataIsolate(String identifier) async {
  final ptr = IaGetNative.bindings.ia_get_fetch_metadata(
    identifier.toNativeUtf8().cast(),
  );
  
  if (ptr == nullptr) {
    final errorPtr = IaGetNative.bindings.ia_get_last_error();
    final error = errorPtr.cast<Utf8>().toDartString();
    throw Exception('Failed to fetch metadata: $error');
  }
  
  final json = ptr.cast<Utf8>().toDartString();
  IaGetNative.bindings.ia_get_free_string(ptr);
  return json;
}

// Call from main isolate
Future<String> fetchMetadata(String identifier) async {
  return await Isolate.run(() => _fetchMetadataIsolate(identifier));
}
```

### Step 4: Implement Progress Callbacks

For download progress:

```dart
import 'dart:ffi' as ffi;
import 'package:ffi/ffi.dart';

typedef ProgressCallbackNative = ffi.Void Function(
  ffi.Uint64 downloaded,
  ffi.Uint64 total,
  ffi.Pointer<ffi.Void> userData,
);

typedef ProgressCallbackDart = void Function(
  int downloaded,
  int total,
  ffi.Pointer<ffi.Void> userData,
);

// Create native callback
final progressCallbackPointer = ffi.Pointer.fromFunction<ProgressCallbackNative>(
  _progressCallback,
);

void _progressCallback(int downloaded, int total, ffi.Pointer<ffi.Void> userData) {
  // Send progress to main isolate via SendPort
  final port = ffi.Pointer<SendPort>.fromAddress(userData.address).value;
  port.send({'downloaded': downloaded, 'total': total});
}
```

### Step 5: State Management in Dart

All state lives in Dart - use Provider, Riverpod, or Bloc:

```dart
class DownloadProvider extends ChangeNotifier {
  Map<String, DownloadState> _downloads = {};
  
  Future<void> startDownload(String url, String outputPath) async {
    final id = uuid.v4();
    _downloads[id] = DownloadState.inProgress(0, 0);
    notifyListeners();
    
    try {
      await Isolate.run(() => _downloadIsolate(url, outputPath, id));
      _downloads[id] = DownloadState.completed();
    } catch (e) {
      _downloads[id] = DownloadState.failed(e.toString());
    }
    notifyListeners();
  }
}
```

## Memory Management Rules

### ✅ DO

- **Free strings** from `ia_get_fetch_metadata()` and `ia_get_decompress_file()` with `ia_get_free_string()`
- **Use Isolates** for all FFI calls (they're blocking!)
- **Check for NULL** returns and call `ia_get_last_error()`

### ❌ DON'T

- **DON'T free** the string from `ia_get_last_error()`
- **DON'T call FFI functions** from the main Dart isolate (use `Isolate.run()`)
- **DON'T share state** between Rust and Dart (all state in Dart)

## Error Handling

Always check for errors:

```dart
String? fetchMetadataSafe(String identifier) {
  final ptr = IaGetNative.bindings.ia_get_fetch_metadata(
    identifier.toNativeUtf8().cast(),
  );
  
  if (ptr == nullptr) {
    final errorPtr = IaGetNative.bindings.ia_get_last_error();
    if (errorPtr != nullptr) {
      final error = errorPtr.cast<Utf8>().toDartString();
      print('Error: $error');
    }
    return null;
  }
  
  final json = ptr.cast<Utf8>().toDartString();
  IaGetNative.bindings.ia_get_free_string(ptr);
  return json;
}
```

## Building the Rust Library

### For Android

```bash
# Add Android targets
rustup target add aarch64-linux-android
rustup target add armv7-linux-androideabi

# Build
cargo build --release --target aarch64-linux-android --lib
cargo build --release --target armv7-linux-androideabi --lib

# Libraries will be in:
# target/aarch64-linux-android/release/libia_get.so
# target/armv7-linux-androideabi/release/libia_get.so
```

Place these in your Flutter project:
```
android/app/src/main/jniLibs/
  arm64-v8a/libia_get.so
  armeabi-v7a/libia_get.so
```

### For iOS

```bash
rustup target add aarch64-apple-ios
rustup target add x86_64-apple-ios  # Simulator

cargo build --release --target aarch64-apple-ios --lib
```

## Example: Complete Integration

See `docs/RUST_CORE_FLUTTER_INTEGRATION.md` for a complete integration example.

## Architecture Benefits

This simplified approach provides:

- ✅ **57% reduction** in FFI complexity
- ✅ **Zero race conditions** (state in Dart only)
- ✅ **Easier debugging** (single language for state)
- ✅ **Better performance** (no cross-language state sync)
- ✅ **Cleaner code** (clear boundaries)

## Support

For questions or issues, see:
- `docs/RUST_CORE_FLUTTER_INTEGRATION.md` - Full implementation guide
- `docs/IMPLEMENTATION_PLAN.md` - Development plan
- GitHub Issues: https://github.com/Gameaday/ia-get-cli/issues
