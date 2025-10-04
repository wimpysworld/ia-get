import 'dart:async';
import 'dart:convert';
import 'dart:ffi' as ffi;
import 'dart:isolate';
import 'package:ffi/ffi.dart';
import 'package:flutter/foundation.dart';
import '../models/archive_metadata.dart';
import '../models/download_progress.dart';

/// Simplified FFI Service for ia-get
///
/// This service uses the new simplified FFI interface with just 6 functions,
/// moving all state management to Dart. This eliminates race conditions and
/// dramatically simplifies the FFI boundary.
///
/// Architecture:
/// - Rust: Stateless computation engine (6 FFI functions)
/// - Dart: All state management, progress tracking, session handling
/// - Isolates: Used for all blocking Rust calls

// ============================================================================
// FFI Type Definitions
// ============================================================================

/// Result type for FFI operations
enum IaGetResult {
  success(0),
  error(-1);

  final int value;
  const IaGetResult(this.value);

  static IaGetResult fromInt(int value) {
    return value == 0 ? IaGetResult.success : IaGetResult.error;
  }
}

/// Progress callback function type
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

// ============================================================================
// FFI Function Signatures
// ============================================================================

/// Native function signatures
typedef FetchMetadataNative = ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8> identifier);
typedef FetchMetadataDart = ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8> identifier);

typedef DownloadFileNative = ffi.Int32 Function(
  ffi.Pointer<Utf8> url,
  ffi.Pointer<Utf8> outputPath,
  ffi.Pointer<ffi.NativeFunction<ProgressCallbackNative>> callback,
  ffi.Pointer<ffi.Void> userData,
);
typedef DownloadFileDart = int Function(
  ffi.Pointer<Utf8> url,
  ffi.Pointer<Utf8> outputPath,
  ffi.Pointer<ffi.NativeFunction<ProgressCallbackNative>> callback,
  ffi.Pointer<ffi.Void> userData,
);

typedef DecompressFileNative = ffi.Pointer<Utf8> Function(
  ffi.Pointer<Utf8> archivePath,
  ffi.Pointer<Utf8> outputDir,
);
typedef DecompressFileDart = ffi.Pointer<Utf8> Function(
  ffi.Pointer<Utf8> archivePath,
  ffi.Pointer<Utf8> outputDir,
);

typedef ValidateChecksumNative = ffi.Int32 Function(
  ffi.Pointer<Utf8> filePath,
  ffi.Pointer<Utf8> expectedHash,
  ffi.Pointer<Utf8> hashType,
);
typedef ValidateChecksumDart = int Function(
  ffi.Pointer<Utf8> filePath,
  ffi.Pointer<Utf8> expectedHash,
  ffi.Pointer<Utf8> hashType,
);

typedef GetLastErrorNative = ffi.Pointer<Utf8> Function();
typedef GetLastErrorDart = ffi.Pointer<Utf8> Function();

typedef FreeStringNative = ffi.Void Function(ffi.Pointer<Utf8> s);
typedef FreeStringDart = void Function(ffi.Pointer<Utf8> s);

// ============================================================================
// FFI Bindings
// ============================================================================

class IaGetSimpleFFI {
  static ffi.DynamicLibrary? _dylib;
  static String? _loadError;

  static ffi.DynamicLibrary get dylib {
    if (_dylib != null) return _dylib!;

    try {
      if (defaultTargetPlatform == TargetPlatform.android) {
        _dylib = ffi.DynamicLibrary.process();
        if (kDebugMode) {
          print('Simplified FFI: Successfully accessed library on Android');
        }
      } else if (defaultTargetPlatform == TargetPlatform.iOS) {
        _dylib = ffi.DynamicLibrary.process();
      } else {
        throw UnsupportedError('Platform not supported: $defaultTargetPlatform');
      }
    } catch (e, stackTrace) {
      _loadError = e.toString();
      if (kDebugMode) {
        print('Simplified FFI: Failed to load library: $e');
        print('Stack trace: $stackTrace');
      }
      rethrow;
    }

    return _dylib!;
  }

  static String? get loadError => _loadError;

  // Lazy-loaded FFI functions
  static FetchMetadataDart? _fetchMetadata;
  static FetchMetadataDart get fetchMetadata {
    _fetchMetadata ??= dylib.lookupFunction<FetchMetadataNative, FetchMetadataDart>(
      'ia_get_fetch_metadata',
    );
    return _fetchMetadata!;
  }

  static DownloadFileDart? _downloadFile;
  static DownloadFileDart get downloadFile {
    _downloadFile ??= dylib.lookupFunction<DownloadFileNative, DownloadFileDart>(
      'ia_get_download_file',
    );
    return _downloadFile!;
  }

  static DecompressFileDart? _decompressFile;
  static DecompressFileDart get decompressFile {
    _decompressFile ??= dylib.lookupFunction<DecompressFileNative, DecompressFileDart>(
      'ia_get_decompress_file',
    );
    return _decompressFile!;
  }

  static ValidateChecksumDart? _validateChecksum;
  static ValidateChecksumDart get validateChecksum {
    _validateChecksum ??= dylib.lookupFunction<ValidateChecksumNative, ValidateChecksumDart>(
      'ia_get_validate_checksum',
    );
    return _validateChecksum!;
  }

  static GetLastErrorDart? _getLastError;
  static GetLastErrorDart get getLastError {
    _getLastError ??= dylib.lookupFunction<GetLastErrorNative, GetLastErrorDart>(
      'ia_get_last_error',
    );
    return _getLastError!;
  }

  static FreeStringDart? _freeString;
  static FreeStringDart get freeString {
    _freeString ??= dylib.lookupFunction<FreeStringNative, FreeStringDart>(
      'ia_get_free_string',
    );
    return _freeString!;
  }
}

// ============================================================================
// Isolate Message Types
// ============================================================================

class _IsolateMessage {
  final SendPort sendPort;
  final dynamic data;

  _IsolateMessage(this.sendPort, this.data);
}

class _FetchMetadataRequest {
  final String identifier;

  _FetchMetadataRequest(this.identifier);
}

class _DownloadFileRequest {
  final String url;
  final String outputPath;
  final SendPort? progressPort;

  _DownloadFileRequest(this.url, this.outputPath, this.progressPort);
}

class _DecompressFileRequest {
  final String archivePath;
  final String outputDir;

  _DecompressFileRequest(this.archivePath, this.outputDir);
}

class _ValidateChecksumRequest {
  final String filePath;
  final String expectedHash;
  final String hashType;

  _ValidateChecksumRequest(this.filePath, this.expectedHash, this.hashType);
}

// ============================================================================
// Isolate Entry Points
// ============================================================================

void _fetchMetadataIsolate(_IsolateMessage message) {
  try {
    final request = message.data as _FetchMetadataRequest;
    final identifierPtr = request.identifier.toNativeUtf8();

    try {
      final resultPtr = IaGetSimpleFFI.fetchMetadata(identifierPtr.cast());

      if (resultPtr == ffi.nullptr) {
        final errorPtr = IaGetSimpleFFI.getLastError();
        final error = errorPtr.cast<Utf8>().toDartString();
        message.sendPort.send({'success': false, 'error': error});
        return;
      }

      final jsonString = resultPtr.cast<Utf8>().toDartString();
      IaGetSimpleFFI.freeString(resultPtr.cast());

      message.sendPort.send({'success': true, 'data': jsonString});
    } finally {
      malloc.free(identifierPtr);
    }
  } catch (e, stackTrace) {
    message.sendPort.send({
      'success': false,
      'error': 'Failed to fetch metadata: $e\n$stackTrace'
    });
  }
}

void _downloadFileIsolate(_IsolateMessage message) {
  try {
    final request = message.data as _DownloadFileRequest;
    final urlPtr = request.url.toNativeUtf8();
    final outputPathPtr = request.outputPath.toNativeUtf8();

    try {
      // Create progress callback if progress port provided
      ffi.Pointer<ffi.NativeFunction<ProgressCallbackNative>> callbackPtr =
          ffi.nullptr;
      ffi.Pointer<ffi.Void> userDataPtr = ffi.nullptr;

      if (request.progressPort != null) {
        // Store progress port in a global variable for callback access
        // Note: In production, you'd want a more robust solution
        callbackPtr = ffi.Pointer.fromFunction<ProgressCallbackNative>(
          _downloadProgressCallback,
        );
        // userDataPtr would need to be properly allocated and managed
      }

      final result = IaGetSimpleFFI.downloadFile(
        urlPtr.cast(),
        outputPathPtr.cast(),
        callbackPtr,
        userDataPtr,
      );

      if (result != 0) {
        final errorPtr = IaGetSimpleFFI.getLastError();
        final error = errorPtr.cast<Utf8>().toDartString();
        message.sendPort.send({'success': false, 'error': error});
        return;
      }

      message.sendPort.send({'success': true});
    } finally {
      malloc.free(urlPtr);
      malloc.free(outputPathPtr);
    }
  } catch (e, stackTrace) {
    message.sendPort.send({
      'success': false,
      'error': 'Failed to download file: $e\n$stackTrace'
    });
  }
}

void _downloadProgressCallback(
  int downloaded,
  int total,
  ffi.Pointer<ffi.Void> userData,
) {
  // This would be called from Rust during download
  // In a full implementation, you'd need to pass the progress port through userData
  if (kDebugMode) {
    print('Download progress: $downloaded / $total bytes');
  }
}

void _decompressFileIsolate(_IsolateMessage message) {
  try {
    final request = message.data as _DecompressFileRequest;
    final archivePathPtr = request.archivePath.toNativeUtf8();
    final outputDirPtr = request.outputDir.toNativeUtf8();

    try {
      final resultPtr = IaGetSimpleFFI.decompressFile(
        archivePathPtr.cast(),
        outputDirPtr.cast(),
      );

      if (resultPtr == ffi.nullptr) {
        final errorPtr = IaGetSimpleFFI.getLastError();
        final error = errorPtr.cast<Utf8>().toDartString();
        message.sendPort.send({'success': false, 'error': error});
        return;
      }

      final jsonString = resultPtr.cast<Utf8>().toDartString();
      IaGetSimpleFFI.freeString(resultPtr.cast());

      final files = (jsonDecode(jsonString) as List).cast<String>();
      message.sendPort.send({'success': true, 'files': files});
    } finally {
      malloc.free(archivePathPtr);
      malloc.free(outputDirPtr);
    }
  } catch (e, stackTrace) {
    message.sendPort.send({
      'success': false,
      'error': 'Failed to decompress file: $e\n$stackTrace'
    });
  }
}

void _validateChecksumIsolate(_IsolateMessage message) {
  try {
    final request = message.data as _ValidateChecksumRequest;
    final filePathPtr = request.filePath.toNativeUtf8();
    final expectedHashPtr = request.expectedHash.toNativeUtf8();
    final hashTypePtr = request.hashType.toNativeUtf8();

    try {
      final result = IaGetSimpleFFI.validateChecksum(
        filePathPtr.cast(),
        expectedHashPtr.cast(),
        hashTypePtr.cast(),
      );

      if (result == -1) {
        final errorPtr = IaGetSimpleFFI.getLastError();
        final error = errorPtr.cast<Utf8>().toDartString();
        message.sendPort.send({'success': false, 'error': error});
        return;
      }

      message.sendPort.send({'success': true, 'valid': result == 1});
    } finally {
      malloc.free(filePathPtr);
      malloc.free(expectedHashPtr);
      malloc.free(hashTypePtr);
    }
  } catch (e, stackTrace) {
    message.sendPort.send({
      'success': false,
      'error': 'Failed to validate checksum: $e\n$stackTrace'
    });
  }
}

// ============================================================================
// Simplified Service with Dart State Management
// ============================================================================

/// Simplified ia-get service using the new 6-function FFI interface
///
/// All state is managed in Dart, eliminating race conditions and
/// dramatically simplifying the architecture.
class IaGetSimpleService {
  // State management - all in Dart!
  final Map<String, ArchiveMetadata> _metadataCache = {};
  final Map<String, DownloadProgress> _downloadProgress = {};
  final Set<String> _activeDownloads = {};

  /// Fetch metadata for an archive
  Future<ArchiveMetadata> fetchMetadata(String identifier) async {
    // Check cache first
    if (_metadataCache.containsKey(identifier)) {
      return _metadataCache[identifier]!;
    }

    // Create receive port for isolate response
    final receivePort = ReceivePort();

    // Spawn isolate to call blocking FFI
    await Isolate.spawn(
      _fetchMetadataIsolate,
      _IsolateMessage(
        receivePort.sendPort,
        _FetchMetadataRequest(identifier),
      ),
    );

    // Wait for response
    final response = await receivePort.first as Map<String, dynamic>;
    receivePort.close();

    if (!response['success']) {
      throw Exception(response['error']);
    }

    // Parse JSON response
    final jsonData = jsonDecode(response['data']);
    final metadata = ArchiveMetadata.fromJson(jsonData);

    // Cache result
    _metadataCache[identifier] = metadata;

    return metadata;
  }

  /// Download a file with progress tracking
  Future<void> downloadFile(
    String url,
    String outputPath, {
    void Function(int downloaded, int total)? onProgress,
  }) async {
    if (_activeDownloads.contains(url)) {
      throw Exception('Download already in progress for $url');
    }

    _activeDownloads.add(url);

    try {
      // Initialize progress tracking
      _downloadProgress[url] = DownloadProgress.simple(
        downloaded: 0,
        total: 0,
        percentage: 0.0,
        status: 'starting',
      );

      // Create receive port for isolate response
      final receivePort = ReceivePort();

      // Create progress port if callback provided
      ReceivePort? progressPort;
      if (onProgress != null) {
        progressPort = ReceivePort();
        progressPort.listen((data) {
          if (data is Map && data.containsKey('downloaded')) {
            final downloaded = data['downloaded'] as int;
            final total = data['total'] as int;
            onProgress(downloaded, total);

            // Update state
            _downloadProgress[url] = DownloadProgress.simple(
              downloaded: downloaded,
              total: total,
              percentage: total > 0 ? (downloaded / total) * 100 : 0.0,
              status: 'downloading',
            );
          }
        });
      }

      // Spawn isolate to call blocking FFI
      await Isolate.spawn(
        _downloadFileIsolate,
        _IsolateMessage(
          receivePort.sendPort,
          _DownloadFileRequest(url, outputPath, progressPort?.sendPort),
        ),
      );

      // Wait for response
      final response = await receivePort.first as Map<String, dynamic>;
      receivePort.close();
      progressPort?.close();

      if (!response['success']) {
        _downloadProgress[url] = DownloadProgress.simple(
          downloaded: 0,
          total: 0,
          percentage: 0.0,
          status: 'error',
          error: response['error'],
        );
        throw Exception(response['error']);
      }

      // Mark as complete
      final progress = _downloadProgress[url]!;
      _downloadProgress[url] = DownloadProgress.simple(
        downloaded: progress.downloaded,
        total: progress.total,
        percentage: 100.0,
        status: 'complete',
      );
    } finally {
      _activeDownloads.remove(url);
    }
  }

  /// Decompress an archive file
  Future<List<String>> decompressFile(
    String archivePath,
    String outputDir,
  ) async {
    // Create receive port for isolate response
    final receivePort = ReceivePort();

    // Spawn isolate to call blocking FFI
    await Isolate.spawn(
      _decompressFileIsolate,
      _IsolateMessage(
        receivePort.sendPort,
        _DecompressFileRequest(archivePath, outputDir),
      ),
    );

    // Wait for response
    final response = await receivePort.first as Map<String, dynamic>;
    receivePort.close();

    if (!response['success']) {
      throw Exception(response['error']);
    }

    return (response['files'] as List).cast<String>();
  }

  /// Validate file checksum
  Future<bool> validateChecksum(
    String filePath,
    String expectedHash, {
    String hashType = 'md5',
  }) async {
    // Create receive port for isolate response
    final receivePort = ReceivePort();

    // Spawn isolate to call blocking FFI
    await Isolate.spawn(
      _validateChecksumIsolate,
      _IsolateMessage(
        receivePort.sendPort,
        _ValidateChecksumRequest(filePath, expectedHash, hashType),
      ),
    );

    // Wait for response
    final response = await receivePort.first as Map<String, dynamic>;
    receivePort.close();

    if (!response['success']) {
      throw Exception(response['error']);
    }

    return response['valid'] as bool;
  }

  /// Get current download progress for a URL
  DownloadProgress? getDownloadProgress(String url) {
    return _downloadProgress[url];
  }

  /// Check if download is active
  bool isDownloadActive(String url) {
    return _activeDownloads.contains(url);
  }

  /// Clear metadata cache
  void clearMetadataCache() {
    _metadataCache.clear();
  }

  /// Clear download history
  void clearDownloadHistory() {
    _downloadProgress.clear();
  }
}
