import 'dart:ffi';
import 'dart:isolate';
import 'dart:convert';
import 'package:ffi/ffi.dart';
import 'package:flutter/foundation.dart';
import '../models/archive_metadata.dart';
import '../models/download_progress.dart';

// Callback function types
typedef ProgressCallbackNative = Void Function(Double, Pointer<Utf8>, IntPtr);
typedef CompletionCallbackNative = Void Function(Bool, Pointer<Utf8>, IntPtr);

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
  
  static final _iaGetIsRequestInProgress = dylib.lookupFunction<
      Bool Function(Pointer<Utf8>),
      bool Function(Pointer<Utf8>)>('ia_get_is_request_in_progress');
  
  static final _iaGetGetPerformanceMetrics = dylib.lookupFunction<
      Pointer<Utf8> Function(),
      Pointer<Utf8> Function()>('ia_get_get_performance_metrics');
  
  static final _iaGetResetPerformanceMetrics = dylib.lookupFunction<
      Int32 Function(),
      int Function()>('ia_get_reset_performance_metrics');
  
  static final _iaGetHealthCheck = dylib.lookupFunction<
      Int32 Function(),
      int Function()>('ia_get_health_check');
  
  static final _iaGetClearStaleCache = dylib.lookupFunction<
      Int32 Function(),
      int Function()>('ia_get_clear_stale_cache');
  
  static final _iaGetGetCircuitBreakerStatus = dylib.lookupFunction<
      Int32 Function(),
      int Function()>('ia_get_get_circuit_breaker_status');
  
  static final _iaGetResetCircuitBreaker = dylib.lookupFunction<
      Int32 Function(),
      int Function()>('ia_get_reset_circuit_breaker');
  
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
    if (identifier.isEmpty) {
      if (kDebugMode) {
        print('getMetadataJson: empty identifier');
      }
      return null;
    }
    
    final identifierPtr = identifier.toNativeUtf8();
    try {
      final resultPtr = _iaGetGetMetadataJson(identifierPtr);
      if (resultPtr == nullptr) {
        if (kDebugMode) {
          print('getMetadataJson: null result for $identifier');
        }
        return null;
      }
      
      try {
        final result = resultPtr.toDartString();
        _iaGetFreeString(resultPtr);
        return result;
      } catch (e) {
        if (kDebugMode) {
          print('getMetadataJson: error converting to string: $e');
        }
        // Try to free the string even if conversion failed
        try {
          _iaGetFreeString(resultPtr);
        } catch (_) {}
        return null;
      }
    } catch (e) {
      if (kDebugMode) {
        print('getMetadataJson: exception: $e');
      }
      return null;
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
    if (metadataJson.isEmpty) {
      if (kDebugMode) {
        print('filterFiles: empty metadata JSON');
      }
      return null;
    }
    
    final metadataPtr = metadataJson.toNativeUtf8();
    final includePtr = includeFormats?.toNativeUtf8() ?? nullptr;
    final excludePtr = excludeFormats?.toNativeUtf8() ?? nullptr;
    final maxSizePtr = maxSize?.toNativeUtf8() ?? nullptr;
    
    try {
      final resultPtr = _iaGetFilterFiles(metadataPtr, includePtr, excludePtr, maxSizePtr);
      if (resultPtr == nullptr) {
        if (kDebugMode) {
          print('filterFiles: null result');
        }
        return null;
      }
      
      try {
        final result = resultPtr.toDartString();
        _iaGetFreeString(resultPtr);
        return result;
      } catch (e) {
        if (kDebugMode) {
          print('filterFiles: error converting to string: $e');
        }
        // Try to free the string even if conversion failed
        try {
          _iaGetFreeString(resultPtr);
        } catch (_) {}
        return null;
      }
    } catch (e) {
      if (kDebugMode) {
        print('filterFiles: exception: $e');
      }
      return null;
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
  
  /// Check if a request is already in progress
  static bool isRequestInProgress(String identifier) {
    if (identifier.isEmpty) return false;
    
    final identifierPtr = identifier.toNativeUtf8();
    try {
      return _iaGetIsRequestInProgress(identifierPtr);
    } finally {
      malloc.free(identifierPtr);
    }
  }
  
  /// Get performance metrics as JSON
  static String? getPerformanceMetrics() {
    try {
      final resultPtr = _iaGetGetPerformanceMetrics();
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
      if (kDebugMode) {
        print('getPerformanceMetrics: exception: $e');
      }
      return null;
    }
  }
  
  /// Reset performance metrics
  static int resetPerformanceMetrics() {
    return _iaGetResetPerformanceMetrics();
  }
  
  /// Perform health check (returns 0 for healthy, higher values indicate issues)
  static int healthCheck() {
    return _iaGetHealthCheck();
  }
  
  /// Clear stale cache entries
  static int clearStaleCache() {
    return _iaGetClearStaleCache();
  }
  
  /// Get circuit breaker status (0=Closed, 1=HalfOpen, 2=Open, -1=Error)
  static int getCircuitBreakerStatus() {
    return _iaGetGetCircuitBreakerStatus();
  }
  
  /// Reset circuit breaker
  static int resetCircuitBreaker() {
    return _iaGetResetCircuitBreaker();
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
        if (kDebugMode) {
          print('FFI initialization failed with code: $result');
        }
      } else {
        if (kDebugMode) {
          print('FFI initialized successfully');
        }
      }
    } catch (e, stackTrace) {
      _error = 'FFI initialization error: $e';
      if (kDebugMode) {
        print('FFI initialization exception: $e\n$stackTrace');
      }
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
    
    // Validate identifier - trim and check for empty/whitespace
    final trimmedIdentifier = identifier.trim();
    if (trimmedIdentifier.isEmpty) {
      _error = 'Invalid identifier: cannot be empty';
      notifyListeners();
      return;
    }
    
    // Additional validation - check for invalid characters
    if (trimmedIdentifier.contains(RegExp(r'[^\w\-\.]'))) {
      _error = 'Invalid identifier: contains invalid characters';
      notifyListeners();
      return;
    }
    
    // Check if request is already in progress
    if (IaGetFFI.isRequestInProgress(trimmedIdentifier)) {
      _error = 'Request already in progress for this identifier';
      if (kDebugMode) {
        print('Duplicate request detected for: $trimmedIdentifier');
      }
      notifyListeners();
      return;
    }
    
    // Check circuit breaker status
    final circuitBreakerStatus = IaGetFFI.getCircuitBreakerStatus();
    if (circuitBreakerStatus == 2) { // Open state
      _error = 'Service temporarily unavailable (circuit breaker open). Please try again later.';
      if (kDebugMode) {
        print('Circuit breaker is open, rejecting request');
      }
      notifyListeners();
      return;
    } else if (circuitBreakerStatus == 1) { // HalfOpen state
      if (kDebugMode) {
        print('Circuit breaker is in half-open state, proceeding cautiously');
      }
    }
    
    // Check health before proceeding
    final health = IaGetFFI.healthCheck();
    if (health > 30) {
      _error = 'System health degraded. Please try again later.';
      if (kDebugMode) {
        print('Health check failed with score: $health');
      }
      notifyListeners();
      return;
    }
    
    _isLoading = true;
    _error = null;
    _currentMetadata = null;
    _filteredFiles = [];
    notifyListeners();
    
    int maxRetries = 3;
    int retryCount = 0;
    
    while (retryCount < maxRetries) {
      try {
        // Create progress and completion callbacks
        final progressCallback = Pointer.fromFunction<ProgressCallbackNative>(_progressCallback);
        final completionCallback = Pointer.fromFunction<CompletionCallbackNative>(_completionCallback);
        
        if (kDebugMode) {
          print('Starting metadata fetch for: $trimmedIdentifier (attempt ${retryCount + 1}/$maxRetries)');
        }
        
        // Start metadata fetch
        final requestId = IaGetFFI.fetchMetadata(
          trimmedIdentifier,
          progressCallback,
          completionCallback,
          trimmedIdentifier.hashCode, // Use identifier hash as user data
        );
        
        if (requestId <= 0) {
          throw Exception('Failed to start metadata fetch (request ID: $requestId)');
        }
        
        if (kDebugMode) {
          print('Metadata fetch started with request ID: $requestId');
        }
        
        // Wait for completion using proper async handling with timeout
        await _waitForMetadataCompletion(trimmedIdentifier, timeout: const Duration(seconds: 30));
        
        // Get the cached metadata
        final metadataJson = IaGetFFI.getMetadataJson(trimmedIdentifier);
        if (metadataJson == null || metadataJson.isEmpty) {
          throw Exception('No metadata available for identifier: $trimmedIdentifier');
        }
        
        if (kDebugMode) {
          print('Retrieved metadata JSON (${metadataJson.length} bytes)');
        }
        
        // Parse JSON with error handling
        try {
          final metadataMap = jsonDecode(metadataJson) as Map<String, dynamic>;
          _currentMetadata = ArchiveMetadata.fromJson(metadataMap);
          _filteredFiles = _currentMetadata!.files;
          
          if (kDebugMode) {
            print('Successfully parsed metadata: ${_currentMetadata!.identifier}');
            print('Files found: ${_filteredFiles.length}');
          }
          
          // Success - break out of retry loop
          break;
        } catch (e) {
          throw Exception('Failed to parse metadata JSON: $e');
        }
        
      } catch (e, stackTrace) {
        retryCount++;
        
        if (retryCount >= maxRetries) {
          _error = 'Failed to fetch metadata after $maxRetries attempts: $e';
          if (kDebugMode) {
            print('Metadata fetch error: $e\n$stackTrace');
          }
          break;
        } else {
          if (kDebugMode) {
            print('Metadata fetch attempt ${retryCount} failed: $e. Retrying...');
          }
          // Exponential backoff: wait before retrying
          await Future.delayed(Duration(seconds: retryCount * 2));
        }
      }
    }
    
    _isLoading = false;
    notifyListeners();
  }
  
  /// Filter files based on criteria
  void filterFiles({
    List<String>? includeFormats,
    List<String>? excludeFormats,
    String? maxSize,
  }) {
    if (_currentMetadata == null) {
      _error = 'No metadata available to filter';
      notifyListeners();
      return;
    }
    
    try {
      final metadataJson = jsonEncode(_currentMetadata!.toJson());
      final includeFormatsStr = includeFormats?.join(',');
      final excludeFormatsStr = excludeFormats?.join(',');
      
      if (kDebugMode) {
        print('Filtering files - include: $includeFormatsStr, exclude: $excludeFormatsStr, maxSize: $maxSize');
      }
      
      final filteredJson = IaGetFFI.filterFiles(
        metadataJson,
        includeFormatsStr,
        excludeFormatsStr,
        maxSize,
      );
      
      if (filteredJson != null && filteredJson.isNotEmpty) {
        try {
          final filteredList = jsonDecode(filteredJson) as List<dynamic>;
          _filteredFiles = filteredList
              .map((json) => ArchiveFile.fromJson(json as Map<String, dynamic>))
              .toList();
          _error = null; // Clear any previous errors
          
          if (kDebugMode) {
            print('Filtered to ${_filteredFiles.length} files');
          }
        } catch (e) {
          _error = 'Failed to parse filtered results: $e';
          if (kDebugMode) {
            print('Filter parsing error: $e');
          }
        }
      } else {
        // No results or null - treat as no matches
        _filteredFiles = [];
        if (kDebugMode) {
          print('No files matched the filter criteria');
        }
      }
    } catch (e) {
      _error = 'Failed to filter files: $e';
      if (kDebugMode) {
        print('Filter error: $e');
      }
    }
    
    notifyListeners();
  }
  
  /// Calculate total size of selected files
  int calculateTotalSize(List<ArchiveFile> selectedFiles) {
    if (selectedFiles.isEmpty) {
      return 0;
    }
    
    try {
      final filesJson = jsonEncode(selectedFiles.map((f) => f.toJson()).toList());
      final size = IaGetFFI.calculateTotalSize(filesJson);
      
      if (kDebugMode) {
        print('Calculated total size: $size bytes for ${selectedFiles.length} files');
      }
      
      return size;
    } catch (e) {
      if (kDebugMode) {
        print('Failed to calculate total size: $e');
      }
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
  
  /// Wait for metadata fetch completion with timeout
  Future<void> _waitForMetadataCompletion(String identifier, {required Duration timeout}) async {
    const checkInterval = Duration(milliseconds: 500);
    final endTime = DateTime.now().add(timeout);
    int attempts = 0;
    final maxAttempts = timeout.inMilliseconds ~/ checkInterval.inMilliseconds;
    
    if (kDebugMode) {
      print('Waiting for metadata completion (timeout: ${timeout.inSeconds}s)...');
    }
    
    while (DateTime.now().isBefore(endTime)) {
      attempts++;
      
      // Add a delay before checking (not after) for better timing
      if (attempts > 1) {
        await Future.delayed(checkInterval);
      }
      
      // Check if metadata is available
      try {
        final metadataJson = IaGetFFI.getMetadataJson(identifier);
        if (metadataJson != null && metadataJson.isNotEmpty) {
          if (kDebugMode) {
            print('Metadata available after $attempts attempts (${attempts * checkInterval.inMilliseconds}ms)');
          }
          return; // Metadata is ready
        }
      } catch (e) {
        if (kDebugMode) {
          print('Error checking metadata availability (attempt $attempts): $e');
        }
        // Continue waiting but log the error
      }
      
      if (kDebugMode && attempts % 4 == 0) {
        print('Still waiting... ($attempts/${maxAttempts} attempts)');
      }
      
      // Add small initial delay to give the fetch operation time to start
      if (attempts == 1) {
        await Future.delayed(checkInterval);
      }
    }
    
    throw Exception('Metadata fetch timeout after ${timeout.inSeconds} seconds (${attempts} attempts)');
  }
  
  /// Get performance metrics
  Future<Map<String, dynamic>?> getPerformanceMetrics() async {
    try {
      final metricsJson = IaGetFFI.getPerformanceMetrics();
      if (metricsJson == null) return null;
      
      return jsonDecode(metricsJson) as Map<String, dynamic>;
    } catch (e) {
      if (kDebugMode) {
        print('Failed to get performance metrics: $e');
      }
      return null;
    }
  }
  
  /// Reset performance metrics
  void resetPerformanceMetrics() {
    IaGetFFI.resetPerformanceMetrics();
  }
  
  /// Check system health
  /// Returns health score (0 = healthy, higher = more issues)
  int checkHealth() {
    return IaGetFFI.healthCheck();
  }
  
  /// Clear stale cache entries
  void clearStaleCache() {
    IaGetFFI.clearStaleCache();
  }
  
  /// Get circuit breaker status
  /// Returns: 0=Closed (healthy), 1=HalfOpen (recovering), 2=Open (failing), -1=Error
  int getCircuitBreakerStatus() {
    return IaGetFFI.getCircuitBreakerStatus();
  }
  
  /// Reset circuit breaker (use with caution)
  void resetCircuitBreaker() {
    IaGetFFI.resetCircuitBreaker();
    if (kDebugMode) {
      print('Circuit breaker has been reset');
    }
  }
  
  /// Perform routine maintenance
  Future<void> performMaintenance() async {
    if (kDebugMode) {
      print('Performing routine maintenance...');
    }
    
    // Clear stale cache
    clearStaleCache();
    
    // Check health
    final health = checkHealth();
    if (kDebugMode) {
      print('Health check score: $health');
    }
    
    // Get metrics for monitoring
    final metrics = await getPerformanceMetrics();
    if (kDebugMode && metrics != null) {
      print('Performance metrics: $metrics');
    }
    
    // Reset circuit breaker if it's been open too long and health is good
    final circuitBreakerStatus = getCircuitBreakerStatus();
    if (circuitBreakerStatus == 2 && health < 20) {
      if (kDebugMode) {
        print('Circuit breaker open but health is good, resetting');
      }
      resetCircuitBreaker();
    }
  }
}