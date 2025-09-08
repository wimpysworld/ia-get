import 'dart:ffi';
import 'dart:isolate';
import 'dart:convert';
import 'package:ffi/ffi.dart';
import 'package:flutter/foundation.dart';
import '../models/archive_metadata.dart';
import '../models/download_progress.dart';

/// FFI bindings for ia-get native library
class IaGetFFI {
  static DynamicLibrary? _dylib;
  
  static DynamicLibrary get dylib {
    if (_dylib != null) return _dylib!;
    
    if (defaultTargetPlatform == TargetPlatform.android) {
      _dylib = DynamicLibrary.open('libia_get_mobile.so');
    } else if (defaultTargetPlatform == TargetPlatform.iOS) {
      _dylib = DynamicLibrary.process();
    } else {
      throw UnsupportedError('Platform not supported');
    }
    
    return _dylib!;
  }
  
  // FFI function signatures
  static final _iaGetInit = dylib.lookupFunction<
      Int32 Function(),
      int Function()>('ia_get_init');
      
  static final _iaGetFetchMetadata = dylib.lookupFunction<
      Int32 Function(Pointer<Utf8>, Pointer<NativeFunction<ProgressCallbackNative>>, 
                    Pointer<NativeFunction<CompletionCallbackNative>>, IntPtr),
      int Function(Pointer<Utf8>, Pointer<NativeFunction<ProgressCallbackNative>>, 
                  Pointer<NativeFunction<CompletionCallbackNative>>, int)>('ia_get_fetch_metadata');
                  
  static final _iaGetFilterFiles = dylib.lookupFunction<
      Pointer<Utf8> Function(Pointer<Utf8>, Pointer<Utf8>, Pointer<Utf8>, Pointer<Utf8>),
      Pointer<Utf8> Function(Pointer<Utf8>, Pointer<Utf8>, Pointer<Utf8>, Pointer<Utf8>)>('ia_get_filter_files');
      
  static final _iaGetFreeString = dylib.lookupFunction<
      Void Function(Pointer<Utf8>),
      void Function(Pointer<Utf8>)>('ia_get_free_string');
      
  static final _iaGetGetMetadataJson = dylib.lookupFunction<
      Pointer<Utf8> Function(Pointer<Utf8>),
      Pointer<Utf8> Function(Pointer<Utf8>)>('ia_get_get_metadata_json');
      
  static final _iaGetCalculateTotalSize = dylib.lookupFunction<
      Uint64 Function(Pointer<Utf8>),
      int Function(Pointer<Utf8>)>('ia_get_calculate_total_size');
  
  // Callback function types
  typedef ProgressCallbackNative = Void Function(Double, Pointer<Utf8>, IntPtr);
  typedef CompletionCallbackNative = Void Function(Bool, Pointer<Utf8>, IntPtr);
  
  /// Initialize the FFI library
  static int init() {
    return _iaGetInit();
  }
  
  /// Fetch archive metadata
  static int fetchMetadata(
    String identifier,
    Pointer<NativeFunction<ProgressCallbackNative>> progressCallback,
    Pointer<NativeFunction<CompletionCallbackNative>> completionCallback,
    int userData,
  ) {
    final identifierPtr = identifier.toNativeUtf8();
    try {
      return _iaGetFetchMetadata(identifierPtr, progressCallback, completionCallback, userData);
    } finally {
      malloc.free(identifierPtr);
    }
  }
  
  /// Get cached metadata as JSON
  static String? getMetadataJson(String identifier) {
    final identifierPtr = identifier.toNativeUtf8();
    try {
      final resultPtr = _iaGetGetMetadataJson(identifierPtr);
      if (resultPtr == nullptr) return null;
      
      final result = resultPtr.toDartString();
      _iaGetFreeString(resultPtr);
      return result;
    } finally {
      malloc.free(identifierPtr);
    }
  }
  
  /// Filter files based on criteria
  static String? filterFiles(
    String metadataJson,
    String? includeFormats,
    String? excludeFormats,
    String? maxSize,
  ) {
    final metadataPtr = metadataJson.toNativeUtf8();
    final includePtr = includeFormats?.toNativeUtf8() ?? nullptr;
    final excludePtr = excludeFormats?.toNativeUtf8() ?? nullptr;
    final maxSizePtr = maxSize?.toNativeUtf8() ?? nullptr;
    
    try {
      final resultPtr = _iaGetFilterFiles(metadataPtr, includePtr, excludePtr, maxSizePtr);
      if (resultPtr == nullptr) return null;
      
      final result = resultPtr.toDartString();
      _iaGetFreeString(resultPtr);
      return result;
    } finally {
      malloc.free(metadataPtr);
      if (includePtr != nullptr) malloc.free(includePtr);
      if (excludePtr != nullptr) malloc.free(excludePtr);
      if (maxSizePtr != nullptr) malloc.free(maxSizePtr);
    }
  }
  
  /// Calculate total size of selected files
  static int calculateTotalSize(String filesJson) {
    final filesPtr = filesJson.toNativeUtf8();
    try {
      return _iaGetCalculateTotalSize(filesPtr);
    } finally {
      malloc.free(filesPtr);
    }
  }
}

/// Service class for managing ia-get operations
class IaGetService extends ChangeNotifier {
  bool _isInitialized = false;
  bool _isLoading = false;
  String? _error;
  ArchiveMetadata? _currentMetadata;
  List<ArchiveFile> _filteredFiles = [];
  
  bool get isInitialized => _isInitialized;
  bool get isLoading => _isLoading;
  String? get error => _error;
  ArchiveMetadata? get currentMetadata => _currentMetadata;
  List<ArchiveFile> get filteredFiles => _filteredFiles;
  
  /// Initialize the service
  Future<void> initialize() async {
    try {
      final result = IaGetFFI.init();
      _isInitialized = result == 0;
      if (!_isInitialized) {
        _error = 'Failed to initialize FFI library';
      }
    } catch (e) {
      _error = 'FFI initialization error: $e';
    }
    notifyListeners();
  }
  
  /// Fetch metadata for an archive
  Future<void> fetchMetadata(String identifier) async {
    if (!_isInitialized) {
      _error = 'Service not initialized';
      notifyListeners();
      return;
    }
    
    _isLoading = true;
    _error = null;
    notifyListeners();
    
    try {
      // Create progress and completion callbacks
      final progressCallback = Pointer.fromFunction<IaGetFFI.ProgressCallbackNative>(_progressCallback);
      final completionCallback = Pointer.fromFunction<IaGetFFI.CompletionCallbackNative>(_completionCallback);
      
      // Start metadata fetch
      final requestId = IaGetFFI.fetchMetadata(
        identifier,
        progressCallback,
        completionCallback,
        identifier.hashCode, // Use identifier hash as user data
      );
      
      if (requestId <= 0) {
        throw Exception('Failed to start metadata fetch');
      }
      
      // Wait for completion (in real implementation, use proper async handling)
      await Future.delayed(const Duration(seconds: 2));
      
      // Get the cached metadata
      final metadataJson = IaGetFFI.getMetadataJson(identifier);
      if (metadataJson != null) {
        final metadataMap = jsonDecode(metadataJson) as Map<String, dynamic>;
        _currentMetadata = ArchiveMetadata.fromJson(metadataMap);
        _filteredFiles = _currentMetadata!.files;
      } else {
        throw Exception('Failed to retrieve metadata');
      }
      
    } catch (e) {
      _error = 'Failed to fetch metadata: $e';
    } finally {
      _isLoading = false;
      notifyListeners();
    }
  }
  
  /// Filter files based on criteria
  void filterFiles({
    List<String>? includeFormats,
    List<String>? excludeFormats,
    String? maxSize,
  }) {
    if (_currentMetadata == null) return;
    
    try {
      final metadataJson = jsonEncode(_currentMetadata!.toJson());
      final includeFormatsStr = includeFormats?.join(',');
      final excludeFormatsStr = excludeFormats?.join(',');
      
      final filteredJson = IaGetFFI.filterFiles(
        metadataJson,
        includeFormatsStr,
        excludeFormatsStr,
        maxSize,
      );
      
      if (filteredJson != null) {
        final filteredList = jsonDecode(filteredJson) as List<dynamic>;
        _filteredFiles = filteredList
            .map((json) => ArchiveFile.fromJson(json as Map<String, dynamic>))
            .toList();
      }
    } catch (e) {
      _error = 'Failed to filter files: $e';
    }
    
    notifyListeners();
  }
  
  /// Calculate total size of selected files
  int calculateTotalSize(List<ArchiveFile> selectedFiles) {
    try {
      final filesJson = jsonEncode(selectedFiles.map((f) => f.toJson()).toList());
      return IaGetFFI.calculateTotalSize(filesJson);
    } catch (e) {
      return 0;
    }
  }
  
  // Callback functions (static methods for FFI)
  static void _progressCallback(double progress, Pointer<Utf8> message, int userData) {
    // In a real implementation, use SendPort to communicate with the main isolate
    if (kDebugMode) {
      final msg = message != nullptr ? message.toDartString() : '';
      print('Progress: ${(progress * 100).toStringAsFixed(1)}% - $msg');
    }
  }
  
  static void _completionCallback(bool success, Pointer<Utf8> errorMessage, int userData) {
    // In a real implementation, use SendPort to communicate with the main isolate
    if (kDebugMode) {
      if (success) {
        print('Operation completed successfully');
      } else {
        final error = errorMessage != nullptr ? errorMessage.toDartString() : 'Unknown error';
        print('Operation failed: $error');
      }
    }
  }
}