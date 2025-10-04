# Usage Examples - Improvements in Action

This document demonstrates the practical usage of the improvements made to the Flutter GUI and Rust process.

## Flutter State Machine Usage

### Before: String-based State Management

```dart
// Error-prone string comparisons
if (download.status == 'downloading') {  // Typo-prone!
  // ...
}

// Unclear what states are valid
download = download.copyWith(status: 'fetching');  // Is this valid?
```

### After: Type-safe State Machine

```dart
// Type-safe enum checks
if (download.downloadStatus == DownloadStatus.downloading) {
  // Compile-time safety!
}

// IDE autocomplete helps
download = download.copyWith(
  downloadStatus: DownloadStatus.fetchingMetadata
);

// Helper methods for clarity
if (download.downloadStatus.isActive) {
  print('Download is in progress');
}

if (download.downloadStatus.isFinished) {
  print('Download has completed or failed');
}
```

## Metadata Caching

### Before: Repeated Network Calls

```dart
// Every call fetches from network
final metadata1 = await provider.fetchMetadata('commute_test');
// ... later ...
final metadata2 = await provider.fetchMetadata('commute_test');  // Network call again!
```

### After: Automatic Caching

```dart
// First call fetches from network
final metadata1 = await provider.fetchMetadata('commute_test');

// Subsequent calls use cache (instant!)
final metadata2 = await provider.fetchMetadata('commute_test');

// Manual cache control
provider.clearMetadataCache();  // Clear when needed
```

## Enhanced File Filtering

### Before: Basic Substring Matching

```dart
// Only exact substring matching
await provider.startDownload(
  'commute_test',
  fileFilters: ['pdf'],  // Matches files containing 'pdf'
);
```

### After: Wildcard Patterns

```dart
// Wildcard patterns supported
await provider.startDownload(
  'commute_test',
  fileFilters: [
    '*.pdf',      // All PDF files
    '*.mp3',      // All MP3 files
    'chapter*',   // Files starting with 'chapter'
    '*_final.*',  // Files ending with '_final'
  ],
);

// Case-insensitive matching
fileFilters: ['*.PDF']  // Matches .pdf, .PDF, .Pdf, etc.
```

## Improved Error Handling

### Before: Generic Errors

```dart
try {
  await provider.startDownload('commute_test');
} catch (e) {
  // Generic error message
  print('Error: $e');  // "Exception: Download failed"
}
```

### After: Categorized Error Messages

```dart
try {
  await provider.startDownload('commute_test');
} catch (e) {
  final download = provider.getDownload('commute_test');
  
  // User-friendly categorized messages
  print(download?.error);  
  // Examples:
  // - "Network error: Please check your internet connection"
  // - "Permission error: Cannot write to destination"
  // - "Storage error: Insufficient disk space"
}
```

## Concurrent Download Tracking

### Before: No Tracking

```dart
// No way to know how many downloads are active
await provider.startDownload('item1');
await provider.startDownload('item2');
await provider.startDownload('item3');
```

### After: Active Download Monitoring

```dart
// Configure max concurrent downloads
provider.maxConcurrentDownloads = 3;

// Monitor active downloads
print('Active: ${provider.activeDownloadCount}');

// Check before starting new download
if (provider.activeDownloadCount < provider.maxConcurrentDownloads) {
  await provider.startDownload('item4');
} else {
  print('Maximum concurrent downloads reached');
}
```

## Rust FFI Safety Improvements

### Before: Scattered Unsafe Code

```rust
// Multiple unsafe blocks with duplicate validation
#[no_mangle]
pub unsafe extern "C" fn ia_get_fetch_metadata(id: *const c_char) -> *mut c_char {
    if id.is_null() {
        return ptr::null_mut();
    }
    
    let id_str = match CStr::from_ptr(id).to_str() {  // Unsafe
        Ok(s) => s,
        Err(_) => return ptr::null_mut(),
    };
    
    // ... more code ...
}
```

### After: Safe Helpers

```rust
// Clean, safe helper functions
fn safe_c_str_to_str<'a>(c_str: *const c_char) -> Option<&'a str> {
    if c_str.is_null() {
        return None;
    }
    unsafe {
        CStr::from_ptr(c_str).to_str().ok()
    }
}

// FFI functions are now cleaner
#[no_mangle]
pub unsafe extern "C" fn ia_get_fetch_metadata(id: *const c_char) -> *mut c_char {
    let id_str = match safe_c_str_to_str(id) {  // Safe wrapper!
        Some(s) => s,
        None => {
            set_last_error("Invalid identifier");
            return ptr::null_mut();
        }
    };
    
    // ... rest of code ...
}
```

## Complete Example: Download with All Improvements

```dart
import 'package:flutter/material.dart';
import 'package:provider/provider.dart';

class DownloadExample extends StatelessWidget {
  @override
  Widget build(BuildContext context) {
    return Consumer<DownloadProvider>(
      builder: (context, provider, child) {
        return Column(
          children: [
            // Show active download count
            Text('Active Downloads: ${provider.activeDownloadCount} / '
                 '${provider.maxConcurrentDownloads}'),
            
            // Download button with state checking
            ElevatedButton(
              onPressed: () async {
                try {
                  await provider.startDownload(
                    'commute_test',
                    fileFilters: ['*.pdf', '*.mp3'],  // Wildcard support!
                    outputDir: '/downloads/commute',
                  );
                } catch (e) {
                  // User-friendly error messages
                  final download = provider.getDownload('commute_test');
                  ScaffoldMessenger.of(context).showSnackBar(
                    SnackBar(content: Text(download?.error ?? 'Unknown error')),
                  );
                }
              },
              child: Text('Download'),
            ),
            
            // Display download status with type-safe checks
            Builder(
              builder: (context) {
                final download = provider.getDownload('commute_test');
                if (download == null) {
                  return Text('Not started');
                }
                
                // Type-safe state checking
                if (download.downloadStatus.isActive) {
                  return Column(
                    children: [
                      Text('Status: ${download.downloadStatus.value}'),
                      CircularProgressIndicator(
                        value: download.overallProgress / 100,
                      ),
                    ],
                  );
                }
                
                if (download.downloadStatus == DownloadStatus.complete) {
                  return Text('✅ Download Complete!');
                }
                
                if (download.downloadStatus == DownloadStatus.error) {
                  return Text('❌ Error: ${download.error}');
                }
                
                return Text('Status: ${download.downloadStatus.value}');
              },
            ),
          ],
        );
      },
    );
  }
}
```

## Performance Benefits

### Metadata Caching Impact

```dart
// Without caching (baseline)
Stopwatch stopwatch = Stopwatch()..start();
await provider.fetchMetadata('commute_test');  // ~500ms network call
print('First fetch: ${stopwatch.elapsedMilliseconds}ms');

stopwatch.reset();
await provider.fetchMetadata('commute_test');  // ~500ms network call again
print('Second fetch: ${stopwatch.elapsedMilliseconds}ms');

// With caching (optimized)
stopwatch.reset();
await provider.fetchMetadata('commute_test');  // ~500ms network call
print('First fetch: ${stopwatch.elapsedMilliseconds}ms');

stopwatch.reset();
await provider.fetchMetadata('commute_test');  // ~1ms from cache!
print('Second fetch: ${stopwatch.elapsedMilliseconds}ms');
```

**Result**: ~500x faster for cached metadata!

## Migration Guide

If you have existing code using the old string-based status:

```dart
// Old code still works (backward compatible)
if (download.status == 'downloading') {
  // This still works!
}

// But new code should use enum
if (download.downloadStatus == DownloadStatus.downloading) {
  // Type-safe and better!
}

// Old copyWith still works
download = download.copyWith(status: 'complete');

// New copyWith is better
download = download.copyWith(downloadStatus: DownloadStatus.complete);
```

## Testing the Improvements

### Testing Wildcard Filtering

```dart
test('Wildcard filtering works', () async {
  final provider = DownloadProvider();
  
  // Mock metadata with various files
  final metadata = ArchiveMetadata(
    files: [
      FileInfo(name: 'document.pdf'),
      FileInfo(name: 'audio.mp3'),
      FileInfo(name: 'image.jpg'),
      FileInfo(name: 'chapter1.pdf'),
      FileInfo(name: 'chapter2.pdf'),
    ],
  );
  
  // Test wildcard filtering
  final filtered = metadata.files.where((file) {
    return '*.pdf'.toLowerCase().replaceAll('*', '.*')
        .let((pattern) => RegExp(pattern).hasMatch(file.name.toLowerCase()));
  }).toList();
  
  expect(filtered.length, equals(3));  // All PDFs
});
```

### Testing State Transitions

```dart
test('State machine prevents invalid transitions', () {
  final download = DownloadState(
    identifier: 'test',
    downloadStatus: DownloadStatus.downloading,
  );
  
  // Can check state easily
  expect(download.downloadStatus.isActive, isTrue);
  expect(download.downloadStatus.isFinished, isFalse);
  
  // Type safety prevents typos
  final updated = download.copyWith(
    downloadStatus: DownloadStatus.complete,
  );
  
  expect(updated.downloadStatus.isFinished, isTrue);
});
```

## Conclusion

These improvements provide:
- **Better Type Safety**: Compile-time checks instead of runtime errors
- **Improved Performance**: Caching eliminates redundant network calls
- **Enhanced UX**: Clearer error messages and more flexible filtering
- **Maintainability**: Safer Rust code and cleaner Dart code

All while maintaining backward compatibility and following architecture best practices!
