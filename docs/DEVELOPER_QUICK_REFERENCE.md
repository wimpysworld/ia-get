# Developer Quick Reference

## New Simplified Architecture (v0.8.0+)

This guide helps developers quickly understand the new simplified FFI architecture and how to work with it.

## Architecture Overview

```
┌─────────────────────────────────────┐
│         Flutter/Dart Layer          │
│  (All State Management)             │
├─────────────────────────────────────┤
│  • ArchiveService                   │
│  • DownloadProvider                 │
│  • UI Components                    │
│  • Business Logic                   │
└──────────────┬──────────────────────┘
               │
               │ 6 Simple FFI Functions
               │ (Stateless)
               ▼
┌─────────────────────────────────────┐
│         Rust Core Layer             │
│  (Pure Computation)                 │
├─────────────────────────────────────┤
│  • Stateless Modules:               │
│    - Metadata fetching              │
│    - File downloads                 │
│    - Checksum validation            │
│    - Archive extraction             │
└─────────────────────────────────────┘
```

## The 6 FFI Functions

### 1. Fetch Metadata
```rust
// Rust
pub unsafe extern "C" fn ia_get_fetch_metadata(
    identifier: *const c_char
) -> *mut c_char
```

```dart
// Dart
Future<ArchiveMetadata> fetchMetadata(String identifier) async {
  final result = _ffi.ia_get_fetch_metadata(identifier.toNativeUtf8());
  // Parse JSON and return
}
```

**Use Case:** Get archive information from Internet Archive

### 2. Download File
```rust
// Rust
pub unsafe extern "C" fn ia_get_download_file(
    url: *const c_char,
    output_path: *const c_char,
    progress_callback: ProgressCallback,
    user_data: *mut c_void,
) -> IaGetResult
```

```dart
// Dart
Future<void> downloadFile(
  String url,
  String outputPath,
  ProgressCallback onProgress,
) async {
  // Call FFI in isolate to avoid blocking UI
}
```

**Use Case:** Download files with progress tracking

### 3. Decompress File
```rust
// Rust
pub unsafe extern "C" fn ia_get_decompress_file(
    archive_path: *const c_char,
    output_dir: *const c_char,
) -> *mut c_char
```

```dart
// Dart
Future<List<String>> decompressFile(
  String archivePath,
  String outputDir,
) async {
  final result = _ffi.ia_get_decompress_file(...);
  // Returns JSON array of extracted files
}
```

**Use Case:** Extract archives (zip, tar.gz, etc.)

### 4. Validate Checksum
```rust
// Rust
pub unsafe extern "C" fn ia_get_validate_checksum(
    file_path: *const c_char,
    expected_hash: *const c_char,
    hash_type: *const c_char,
) -> c_int
```

```dart
// Dart
Future<bool> validateChecksum(
  String filePath,
  String expectedHash,
  String hashType, // "md5", "sha1", "sha256"
) async {
  final result = _ffi.ia_get_validate_checksum(...);
  return result == 1;
}
```

**Use Case:** Verify file integrity after download

### 5. Get Last Error
```rust
// Rust
pub unsafe extern "C" fn ia_get_last_error() -> *const c_char
```

```dart
// Dart
String? getLastError() {
  final errorPtr = _ffi.ia_get_last_error();
  if (errorPtr == nullptr) return null;
  return errorPtr.toDartString();
}
```

**Use Case:** Get detailed error information

### 6. Free String
```rust
// Rust
pub unsafe extern "C" fn ia_get_free_string(s: *mut c_char)
```

```dart
// Dart
void _freeString(Pointer<Utf8> ptr) {
  _ffi.ia_get_free_string(ptr.cast());
}
```

**Use Case:** Clean up memory allocated by Rust

## Key Principles

### 1. All State in Dart
```dart
// ✅ GOOD: State in Dart Provider
class DownloadProvider extends ChangeNotifier {
  final Map<String, DownloadState> _downloads = {};
  
  Future<void> startDownload(String identifier) async {
    _downloads[identifier] = DownloadState(status: 'downloading');
    notifyListeners();
    
    // Call stateless Rust function
    await _service.downloadFile(url, path, onProgress);
    
    _downloads[identifier] = DownloadState(status: 'complete');
    notifyListeners();
  }
}
```

```rust
// ❌ BAD: Don't add state to Rust
// static mut DOWNLOADS: HashMap<String, DownloadState> = ...;
```

### 2. Use Isolates for Blocking Calls
```dart
// ✅ GOOD: Run in isolate
Future<ArchiveMetadata> fetchMetadata(String id) async {
  return await Isolate.run(() {
    // This won't block UI thread
    return _callRustFFI(id);
  });
}
```

```dart
// ❌ BAD: Blocking UI thread
Future<ArchiveMetadata> fetchMetadata(String id) async {
  return _callRustFFI(id); // UI will freeze!
}
```

### 3. Error Handling
```dart
// ✅ GOOD: Check for errors
try {
  final result = await _service.downloadFile(url, path);
  // Success
} catch (e) {
  final error = _service.getLastError();
  print('Download failed: $error');
}
```

## Common Patterns

### Pattern 1: Fetch and Display Metadata
```dart
class ArchiveService extends ChangeNotifier {
  ArchiveMetadata? _metadata;
  
  Future<void> fetchMetadata(String identifier) async {
    _isLoading = true;
    notifyListeners();
    
    try {
      _metadata = await _ffi.fetchMetadata(identifier);
      _error = null;
    } catch (e) {
      _error = e.toString();
      _metadata = null;
    } finally {
      _isLoading = false;
      notifyListeners();
    }
  }
}
```

### Pattern 2: Download with Progress
```dart
class DownloadProvider extends ChangeNotifier {
  Future<void> startDownload(String url, String path) async {
    await _service.downloadFile(
      url,
      path,
      onProgress: (downloaded, total) {
        _progress[url] = downloaded / total;
        notifyListeners();
      },
    );
  }
}
```

### Pattern 3: Validate and Extract
```dart
Future<void> processArchive(String archivePath) async {
  // Validate
  final isValid = await _service.validateChecksum(
    archivePath,
    expectedHash,
    'sha256',
  );
  
  if (!isValid) {
    throw Exception('Checksum validation failed');
  }
  
  // Extract
  final extractedFiles = await _service.decompressFile(
    archivePath,
    outputDir,
  );
  
  print('Extracted ${extractedFiles.length} files');
}
```

## Migration from Old FFI

### Old Way (14+ Functions, Stateful)
```dart
// Initialize
await iaGetService.initialize();

// Fetch metadata (state stored in Rust)
await iaGetService.fetchMetadata(identifier);

// Get metadata (from Rust state)
final metadata = iaGetService.currentMetadata;

// Issues:
// - State split between Rust and Dart
// - Race conditions possible
// - Complex lifecycle management
```

### New Way (6 Functions, Stateless)
```dart
// No initialization needed

// Fetch metadata (pure function)
final metadata = await archiveService.fetchMetadata(identifier);

// State stays in Dart
// - No race conditions
// - Simple lifecycle
// - Easy testing
```

## Performance Tips

### 1. Cache Metadata
```dart
class ArchiveService {
  final Map<String, ArchiveMetadata> _cache = {};
  
  Future<ArchiveMetadata> fetchMetadata(String id) async {
    if (_cache.containsKey(id)) {
      return _cache[id]!;
    }
    
    final metadata = await _ffi.fetchMetadata(id);
    _cache[id] = metadata;
    return metadata;
  }
}
```

### 2. Batch Operations
```dart
// ✅ GOOD: Download concurrently
Future<void> downloadFiles(List<FileInfo> files) async {
  await Future.wait(
    files.map((file) => _service.downloadFile(file.url, file.path)),
  );
}
```

### 3. Progress Throttling
```dart
DateTime? _lastUpdate;

void onProgress(int downloaded, int total) {
  final now = DateTime.now();
  if (_lastUpdate != null && 
      now.difference(_lastUpdate!) < Duration(milliseconds: 100)) {
    return; // Throttle updates
  }
  
  _lastUpdate = now;
  _updateProgress(downloaded / total);
}
```

## Testing

### Unit Testing
```dart
test('fetchMetadata returns valid data', () async {
  final service = ArchiveService();
  final metadata = await service.fetchMetadata('test_archive');
  
  expect(metadata.identifier, 'test_archive');
  expect(metadata.files, isNotEmpty);
});
```

### Integration Testing
```dart
testWidgets('download flow completes', (tester) async {
  final provider = DownloadProvider();
  
  await provider.startDownload('archive_id');
  
  expect(provider.downloads['archive_id']?.status, 'complete');
});
```

## Debugging

### Enable Debug Logging
```dart
if (kDebugMode) {
  print('Calling FFI: fetchMetadata($identifier)');
}

final result = await _ffi.fetchMetadata(identifier);

if (kDebugMode) {
  print('FFI returned: ${result.files.length} files');
}
```

### Check for Memory Leaks
```dart
// Always free strings returned by Rust
final jsonPtr = _ffi.ia_get_fetch_metadata(id.toNativeUtf8());
try {
  final json = jsonPtr.toDartString();
  // Use json...
} finally {
  _ffi.ia_get_free_string(jsonPtr); // Important!
}
```

## Common Pitfalls

### ❌ Don't: Store Rust Pointers
```dart
// BAD: Storing raw pointer
class MyService {
  Pointer<Utf8>? _cachedPointer; // Memory leak!
}
```

### ✅ Do: Convert and Free Immediately
```dart
// GOOD: Convert to Dart immediately
class MyService {
  String? _cachedString; // Safe
  
  void fetch() {
    final ptr = _ffi.getSomething();
    _cachedString = ptr.toDartString();
    _ffi.freeString(ptr); // Clean up
  }
}
```

### ❌ Don't: Block UI Thread
```dart
// BAD: Synchronous FFI call on UI thread
Widget build(BuildContext context) {
  final data = _ffi.fetchMetadata('id'); // Freezes UI!
  return Text(data);
}
```

### ✅ Do: Use FutureBuilder
```dart
// GOOD: Async with FutureBuilder
Widget build(BuildContext context) {
  return FutureBuilder(
    future: _service.fetchMetadata('id'),
    builder: (context, snapshot) {
      if (snapshot.hasData) {
        return Text(snapshot.data!.title);
      }
      return CircularProgressIndicator();
    },
  );
}
```

## Resources

- **Architecture Analysis:** `docs/ARCHITECTURE_ANALYSIS.md`
- **Migration Guide:** `docs/FLUTTER_MIGRATION_COMPLETE.md`
- **Next Steps:** `docs/NEXT_STEPS.md`
- **FFI Summary:** `docs/FFI_COMPLETION_SUMMARY.md`
- **C Header:** `include/ia_get_simple.h`

## Quick Commands

```bash
# Build Rust library
cargo build --release

# Run Rust tests
cargo test --lib

# Build Android APK
cd mobile/flutter
flutter build apk

# Run Flutter app
flutter run
```

---

**Version:** ia-get v1.6.0+  
**Last Updated:** 2024  
**Status:** Production Ready
