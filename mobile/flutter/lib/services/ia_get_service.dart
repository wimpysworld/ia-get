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
  static String? _loadError;

  static DynamicLibrary get dylib {
    if (_dylib != null) return _dylib!;

    try {
      if (defaultTargetPlatform == TargetPlatform.android) {
        // On Android, the library is already loaded by MainActivity via System.loadLibrary()
        // We access it via DynamicLibrary.process() to use the already-loaded symbols
        if (kDebugMode) {
          print('FFI: Attempting to access native library via DynamicLibrary.process() on Android...');
        }
        _dylib = DynamicLibrary.process();
        if (kDebugMode) {
          print('FFI: Successfully accessed process library on Android');
        }
      } else if (defaultTargetPlatform == TargetPlatform.iOS) {
        _dylib = DynamicLibrary.process();
      } else {
        throw UnsupportedError('Platform not supported: $defaultTargetPlatform');
      }
    } catch (e, stackTrace) {
      _loadError = e.toString();
      if (kDebugMode) {
        print('FFI: Failed to load dynamic library: $e');
        print('FFI: Stack trace: $stackTrace');
      }
      rethrow;
    }

    return _dylib!;
  }

  static String? get loadError => _loadError;

  // FFI function signatures - made lazy to avoid exceptions during class initialization
  static int Function()? __iaGetInit;
  static int Function() get _iaGetInit {
    if (__iaGetInit != null) return __iaGetInit!;
    __iaGetInit = dylib.lookupFunction<Int32 Function(), int Function()>('ia_get_init');
    return __iaGetInit!;
  }

  static int Function(
    Pointer<Utf8>,
    Pointer<NativeFunction<ProgressCallbackNative>>,
    Pointer<NativeFunction<CompletionCallbackNative>>,
    int,
  )? __iaGetFetchMetadata;
  static int Function(
    Pointer<Utf8>,
    Pointer<NativeFunction<ProgressCallbackNative>>,
    Pointer<NativeFunction<CompletionCallbackNative>>,
    int,
  ) get _iaGetFetchMetadata {
    if (__iaGetFetchMetadata != null) return __iaGetFetchMetadata!;
    __iaGetFetchMetadata = dylib.lookupFunction<
        Int32 Function(
          Pointer<Utf8>,
          Pointer<NativeFunction<ProgressCallbackNative>>,
          Pointer<NativeFunction<CompletionCallbackNative>>,
          IntPtr,
        ),
        int Function(
          Pointer<Utf8>,
          Pointer<NativeFunction<ProgressCallbackNative>>,
          Pointer<NativeFunction<CompletionCallbackNative>>,
          int,
        )
      >('ia_get_fetch_metadata');
    return __iaGetFetchMetadata!;
  }

  static Pointer<Utf8> Function(
    Pointer<Utf8>,
    Pointer<Utf8>,
    Pointer<Utf8>,
    Pointer<Utf8>,
    Pointer<Utf8>,
  )? __iaGetFilterFiles;
  static Pointer<Utf8> Function(
    Pointer<Utf8>,
    Pointer<Utf8>,
    Pointer<Utf8>,
    Pointer<Utf8>,
    Pointer<Utf8>,
  ) get _iaGetFilterFiles {
    if (__iaGetFilterFiles != null) return __iaGetFilterFiles!;
    __iaGetFilterFiles = dylib.lookupFunction<
        Pointer<Utf8> Function(
          Pointer<Utf8>,
          Pointer<Utf8>,
          Pointer<Utf8>,
          Pointer<Utf8>,
          Pointer<Utf8>,
        ),
        Pointer<Utf8> Function(
          Pointer<Utf8>,
          Pointer<Utf8>,
          Pointer<Utf8>,
          Pointer<Utf8>,
          Pointer<Utf8>,
        )
      >('ia_get_filter_files');
    return __iaGetFilterFiles!;
  }

  static void Function(Pointer<Utf8>)? __iaGetFreeString;
  static void Function(Pointer<Utf8>) get _iaGetFreeString {
    if (__iaGetFreeString != null) return __iaGetFreeString!;
    __iaGetFreeString = dylib.lookupFunction<
        Void Function(Pointer<Utf8>),
        void Function(Pointer<Utf8>)
      >('ia_get_free_string');
    return __iaGetFreeString!;
  }

  static Pointer<Utf8> Function(Pointer<Utf8>)? __iaGetGetMetadataJson;
  static Pointer<Utf8> Function(Pointer<Utf8>) get _iaGetGetMetadataJson {
    if (__iaGetGetMetadataJson != null) return __iaGetGetMetadataJson!;
    __iaGetGetMetadataJson = dylib.lookupFunction<
        Pointer<Utf8> Function(Pointer<Utf8>),
        Pointer<Utf8> Function(Pointer<Utf8>)
      >('ia_get_get_metadata_json');
    return __iaGetGetMetadataJson!;
  }

  static int Function(Pointer<Utf8>)? __iaGetCalculateTotalSize;
  static int Function(Pointer<Utf8>) get _iaGetCalculateTotalSize {
    if (__iaGetCalculateTotalSize != null) return __iaGetCalculateTotalSize!;
    __iaGetCalculateTotalSize = dylib.lookupFunction<
        Uint64 Function(Pointer<Utf8>),
        int Function(Pointer<Utf8>)
      >('ia_get_calculate_total_size');
    return __iaGetCalculateTotalSize!;
  }

  static bool Function(Pointer<Utf8>)? __iaGetIsRequestInProgress;
  static bool Function(Pointer<Utf8>) get _iaGetIsRequestInProgress {
    if (__iaGetIsRequestInProgress != null) return __iaGetIsRequestInProgress!;
    __iaGetIsRequestInProgress = dylib.lookupFunction<
        Bool Function(Pointer<Utf8>),
        bool Function(Pointer<Utf8>)
      >('ia_get_is_request_in_progress');
    return __iaGetIsRequestInProgress!;
  }

  static Pointer<Utf8> Function()? __iaGetGetPerformanceMetrics;
  static Pointer<Utf8> Function() get _iaGetGetPerformanceMetrics {
    if (__iaGetGetPerformanceMetrics != null) return __iaGetGetPerformanceMetrics!;
    __iaGetGetPerformanceMetrics = dylib.lookupFunction<
        Pointer<Utf8> Function(), 
        Pointer<Utf8> Function()
      >('ia_get_get_performance_metrics');
    return __iaGetGetPerformanceMetrics!;
  }

  static int Function()? __iaGetResetPerformanceMetrics;
  static int Function() get _iaGetResetPerformanceMetrics {
    if (__iaGetResetPerformanceMetrics != null) return __iaGetResetPerformanceMetrics!;
    __iaGetResetPerformanceMetrics = dylib.lookupFunction<
        Int32 Function(), 
        int Function()
      >('ia_get_reset_performance_metrics');
    return __iaGetResetPerformanceMetrics!;
  }

  static int Function()? __iaGetHealthCheck;
  static int Function() get _iaGetHealthCheck {
    if (__iaGetHealthCheck != null) return __iaGetHealthCheck!;
    __iaGetHealthCheck = dylib.lookupFunction<
        Int32 Function(), 
        int Function()
      >('ia_get_health_check');
    return __iaGetHealthCheck!;
  }

  static int Function()? __iaGetClearStaleCache;
  static int Function() get _iaGetClearStaleCache {
    if (__iaGetClearStaleCache != null) return __iaGetClearStaleCache!;
    __iaGetClearStaleCache = dylib.lookupFunction<
        Int32 Function(), 
        int Function()
      >('ia_get_clear_stale_cache');
    return __iaGetClearStaleCache!;
  }

  static int Function()? __iaGetGetCircuitBreakerStatus;
  static int Function() get _iaGetGetCircuitBreakerStatus {
    if (__iaGetGetCircuitBreakerStatus != null) return __iaGetGetCircuitBreakerStatus!;
    __iaGetGetCircuitBreakerStatus = dylib.lookupFunction<
        Int32 Function(), 
        int Function()
      >('ia_get_get_circuit_breaker_status');
    return __iaGetGetCircuitBreakerStatus!;
  }

  static int Function()? __iaGetResetCircuitBreaker;
  static int Function() get _iaGetResetCircuitBreaker {
    if (__iaGetResetCircuitBreaker != null) return __iaGetResetCircuitBreaker!;
    __iaGetResetCircuitBreaker = dylib.lookupFunction<
        Int32 Function(), 
        int Function()
      >('ia_get_reset_circuit_breaker');
    return __iaGetResetCircuitBreaker!;
  }

  static int Function(int)? __iaGetCancelOperation;
  static int Function(int) get _iaGetCancelOperation {
    if (__iaGetCancelOperation != null) return __iaGetCancelOperation!;
    __iaGetCancelOperation = dylib.lookupFunction<
        Int32 Function(Int32), 
        int Function(int)
      >('ia_get_cancel_operation');
    return __iaGetCancelOperation!;
  }

  static Pointer<Utf8> Function(Pointer<Utf8>, int)? __iaGetSearchArchives;
  static Pointer<Utf8> Function(Pointer<Utf8>, int) get _iaGetSearchArchives {
    if (__iaGetSearchArchives != null) return __iaGetSearchArchives!;
    __iaGetSearchArchives = dylib.lookupFunction<
        Pointer<Utf8> Function(Pointer<Utf8>, Int32),
        Pointer<Utf8> Function(Pointer<Utf8>, int)
      >('ia_get_search_archives');
    return __iaGetSearchArchives!;
  }

  /// Initialize the FFI library
  /// Returns 0 on success, non-zero error code on failure
  static int init() {
    try {
      return _iaGetInit();
    } catch (e) {
      if (kDebugMode) {
        print('FFI: Failed to initialize - symbol lookup or call failed: $e');
      }
      _loadError = 'Init failed: ${e.toString()}';
      return -1;
    }
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
      return _iaGetFetchMetadata(
        identifierPtr,
        progressCallback,
        completionCallback,
        userData,
      );
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
    String? sourceTypes,
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
    final sourceTypesPtr = sourceTypes?.toNativeUtf8() ?? nullptr;

    try {
      final resultPtr = _iaGetFilterFiles(
        metadataPtr,
        includePtr,
        excludePtr,
        maxSizePtr,
        sourceTypesPtr,
      );
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
      if (sourceTypesPtr != nullptr) malloc.free(sourceTypesPtr);
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
        try {
          _iaGetFreeString(resultPtr);
        } catch (_) {}
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

  /// Cancel an ongoing operation
  static int cancelOperation(int operationId) {
    return _iaGetCancelOperation(operationId);
  }

  /// Search for archives
  static String? searchArchives(String query, {int maxResults = 10}) {
    if (query.isEmpty) {
      if (kDebugMode) print('searchArchives: empty query');
      return null;
    }

    final queryPtr = query.toNativeUtf8();
    try {
      final resultPtr = _iaGetSearchArchives(queryPtr, maxResults);
      if (resultPtr == nullptr) return null;

      try {
        final result = resultPtr.toDartString();
        _iaGetFreeString(resultPtr);
        return result;
      } catch (e) {
        try {
          _iaGetFreeString(resultPtr);
        } catch (_) {}
        if (kDebugMode) print('searchArchives: failed to convert result: $e');
        return null;
      }
    } catch (e) {
      if (kDebugMode) print('searchArchives: exception: $e');
      return null;
    } finally {
      malloc.free(queryPtr);
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
  int? _currentRequestId;
  List<Map<String, String>> _suggestions = []; // Store search suggestions

  bool get isInitialized => _isInitialized;
  bool get isLoading => _isLoading;
  String? get error => _error;
  ArchiveMetadata? get currentMetadata => _currentMetadata;
  List<ArchiveFile> get filteredFiles => _filteredFiles;
  bool get canCancel => _isLoading && _currentRequestId != null;
  List<Map<String, String>> get suggestions => _suggestions;

  /// Initialize the service
  Future<void> initialize() async {
    if (kDebugMode) {
      print('IaGetService: Starting initialization...');
    }
    
    try {
      // Try to initialize the FFI library
      if (kDebugMode) {
        print('IaGetService: Calling IaGetFFI.init()...');
      }
      
      final result = IaGetFFI.init();
      _isInitialized = result == 0;
      
      if (!_isInitialized) {
        _error = 'Failed to initialize FFI library (error code: $result)';
        if (kDebugMode) {
          print('FFI initialization failed with code: $result');
        }
      } else {
        if (kDebugMode) {
          print('FFI initialized successfully');
        }
      }
    } catch (e, stackTrace) {
      _error = 'FFI initialization error: ${e.toString()}';
      _isInitialized = false;
      if (kDebugMode) {
        print('FFI initialization exception: $e');
        print('Stack trace: $stackTrace');
      }
    } finally {
      // ALWAYS notify listeners, even if there was an exception
      // This ensures the UI updates to show the error state
      if (kDebugMode) {
        print('IaGetService: Notifying listeners (isInitialized=$_isInitialized, error=$_error)');
      }
      notifyListeners();
    }
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
    if (circuitBreakerStatus == 2) {
      // Open state
      _error =
          'Service temporarily unavailable (circuit breaker open). Please try again later.';
      if (kDebugMode) {
        print('Circuit breaker is open, rejecting request');
      }
      notifyListeners();
      return;
    } else if (circuitBreakerStatus == 1) {
      // HalfOpen state
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
    _suggestions = []; // Clear previous suggestions
    notifyListeners();

    int maxRetries = 3;
    int retryCount = 0;

    while (retryCount < maxRetries) {
      try {
        // Create progress and completion callbacks
        final progressCallback = Pointer.fromFunction<ProgressCallbackNative>(
          _progressCallback,
        );
        final completionCallback =
            Pointer.fromFunction<CompletionCallbackNative>(_completionCallback);

        if (kDebugMode) {
          print(
            'Starting metadata fetch for: $trimmedIdentifier (attempt ${retryCount + 1}/$maxRetries)',
          );
        }

        // Start metadata fetch
        final requestId = IaGetFFI.fetchMetadata(
          trimmedIdentifier,
          progressCallback,
          completionCallback,
          trimmedIdentifier.hashCode, // Use identifier hash as user data
        );

        // Store request ID for cancellation
        _currentRequestId = requestId;
        notifyListeners();

        if (requestId <= 0) {
          throw Exception(
            'Failed to start metadata fetch (request ID: $requestId)',
          );
        }

        if (kDebugMode) {
          print('Metadata fetch started with request ID: $requestId');
        }

        // Wait for completion using proper async handling with timeout
        await _waitForMetadataCompletion(
          trimmedIdentifier,
          timeout: const Duration(seconds: 30),
        );

        // Get the cached metadata
        final metadataJson = IaGetFFI.getMetadataJson(trimmedIdentifier);
        if (metadataJson == null || metadataJson.isEmpty) {
          throw Exception(
            'No metadata available for identifier: $trimmedIdentifier',
          );
        }

        if (kDebugMode) {
          print('Retrieved metadata JSON (${metadataJson.length} bytes)');
        }

        // Parse JSON with error handling
        try {
          final metadataMap = jsonDecode(metadataJson) as Map<String, dynamic>;
          _currentMetadata = ArchiveMetadata.fromJson(metadataMap);
          _filteredFiles = _currentMetadata!.files;
          _suggestions = []; // Clear suggestions on successful fetch

          if (kDebugMode) {
            print(
              'Successfully parsed metadata: ${_currentMetadata!.identifier}',
            );
            print('Files found: ${_filteredFiles.length}');
          }

          // Success - break out of retry loop
          break;
        } catch (e) {
          throw Exception('Failed to parse metadata JSON: $e');
        }
      } catch (e, stackTrace) {
        retryCount++;

        // Show suggestions after first failure (immediately)
        if (retryCount >= 1 && _suggestions.isEmpty) {
          if (kDebugMode) {
            print(
              'Metadata fetch failed after $retryCount attempts. Searching for similar archives...',
            );
          }

          // Check if the identifier contains uppercase letters - IA identifiers are typically lowercase
          final hasUpperCase =
              trimmedIdentifier != trimmedIdentifier.toLowerCase();
          final lowercaseIdentifier = trimmedIdentifier.toLowerCase();

          // Attempt to search for similar archives in background
          try {
            final searchResults = IaGetFFI.searchArchives(
              trimmedIdentifier,
              maxResults: 5,
            );
            if (searchResults != null && searchResults.isNotEmpty) {
              final searchData =
                  jsonDecode(searchResults) as Map<String, dynamic>;
              final docs = searchData['response']?['docs'] as List<dynamic>?;

              if (docs != null && docs.isNotEmpty) {
                // Store suggestions for display (but keep trying to fetch)
                _suggestions = docs.take(5).map((doc) {
                  final id = (doc['identifier'] ?? 'unknown').toString();
                  final title = (doc['title'] ?? id).toString();
                  return {'identifier': id, 'title': title};
                }).toList();

                // If identifier had uppercase, suggest lowercase version as first option
                if (hasUpperCase && lowercaseIdentifier != trimmedIdentifier) {
                  // Insert lowercase suggestion at the beginning
                  _suggestions.insert(0, {
                    'identifier': lowercaseIdentifier,
                    'title': 'Try lowercase: $lowercaseIdentifier',
                  });
                }

                // Show suggestions while continuing to try
                if (retryCount < maxRetries) {
                  _error = hasUpperCase
                      ? 'Archive identifiers are usually lowercase. See suggestions below while we continue searching.'
                      : 'Still searching... See suggestions below while we continue.';
                }

                // Notify to show suggestions immediately
                notifyListeners();
              } else if (hasUpperCase) {
                // No search results but has uppercase - suggest lowercase anyway
                _suggestions = [
                  {
                    'identifier': lowercaseIdentifier,
                    'title': 'Try lowercase: $lowercaseIdentifier',
                  },
                ];
                _error =
                    'Archive identifiers are usually lowercase. Try the suggestion below.';
                notifyListeners();
              }
            } else if (hasUpperCase) {
              // Search failed but has uppercase - suggest lowercase anyway
              _suggestions = [
                {
                  'identifier': lowercaseIdentifier,
                  'title': 'Try lowercase: $lowercaseIdentifier',
                },
              ];
              _error =
                  'Archive identifiers are usually lowercase. Try the suggestion below.';
              notifyListeners();
            }
          } catch (searchError) {
            if (kDebugMode) {
              print('Search for similar archives failed: $searchError');
            }
            // Even if search fails, suggest lowercase if applicable
            if (hasUpperCase) {
              _suggestions = [
                {
                  'identifier': lowercaseIdentifier,
                  'title': 'Try lowercase: $lowercaseIdentifier',
                },
              ];
              _error =
                  'Archive identifiers are usually lowercase. Try the suggestion below.';
              notifyListeners();
            }
          }
        }

        if (retryCount >= maxRetries) {
          // After all retries failed
          if (kDebugMode) {
            print('Metadata fetch failed after $maxRetries attempts.');
          }

          // Set final error message
          if (_suggestions.isNotEmpty) {
            _error =
                'Archive "$trimmedIdentifier" not found. See suggestions below.';
          } else {
            _error =
                'Archive "$trimmedIdentifier" not found. No similar archives found.';
          }

          if (kDebugMode) {
            print('Metadata fetch error: $e\n$stackTrace');
          }
          break;
        } else {
          if (kDebugMode) {
            print(
              'Metadata fetch attempt $retryCount failed: $e. Retrying...',
            );
          }
          // Exponential backoff: wait before retrying
          await Future.delayed(Duration(seconds: retryCount * 2));
        }
      }
    }

    _isLoading = false;
    _currentRequestId = null;
    notifyListeners();
  }

  /// Cancel the current metadata fetch operation
  void cancelOperation() {
    if (_currentRequestId != null) {
      if (kDebugMode) {
        print('Cancelling operation with request ID: $_currentRequestId');
      }

      final result = IaGetFFI.cancelOperation(_currentRequestId!);

      if (result == 0) {
        _isLoading = false;
        _currentRequestId = null;
        _error = 'Operation cancelled by user';

        if (kDebugMode) {
          print('Operation cancelled successfully');
        }
      } else {
        if (kDebugMode) {
          print('Failed to cancel operation (error code: $result)');
        }
      }

      notifyListeners();
    }
  }

  /// Filter files based on criteria
  void filterFiles({
    List<String>? includeFormats,
    List<String>? excludeFormats,
    String? maxSize,
    bool includeOriginal = true,
    bool includeDerivative = true,
    bool includeMetadata = true,
  }) {
    if (_currentMetadata == null) {
      _error = 'No metadata available to filter';
      notifyListeners();
      return;
    }

    final hasSourceFilter =
        !includeOriginal || !includeDerivative || !includeMetadata;

    // Check if any files in the archive actually have source field populated
    final hasSourceField = _currentMetadata!.files.any(
      (file) => file.source != null && file.source!.isNotEmpty,
    );

    // If no filters are active, show all files (default behavior)
    if ((includeFormats == null || includeFormats.isEmpty) &&
        (excludeFormats == null || excludeFormats.isEmpty) &&
        (maxSize == null || maxSize.isEmpty) &&
        !hasSourceFilter) {
      _filteredFiles = _currentMetadata!.files;
      _error = null;

      if (kDebugMode) {
        print('No filters active - showing all ${_filteredFiles.length} files');
      }

      notifyListeners();
      return;
    }

    try {
      final metadataJson = jsonEncode(_currentMetadata!.toJson());
      final includeFormatsStr = includeFormats?.join(',');
      final excludeFormatsStr = excludeFormats?.join(',');
      
      // Build source types string - only include selected types
      String? sourceTypesStr;
      if (hasSourceFilter) {
        List<String> selectedTypes = [];
        if (includeOriginal) selectedTypes.add('original');
        if (includeDerivative) selectedTypes.add('derivative');
        if (includeMetadata) selectedTypes.add('metadata');
        sourceTypesStr = selectedTypes.isEmpty ? null : selectedTypes.join(',');
      }

      if (kDebugMode) {
        print(
          'Filtering files - include: $includeFormatsStr, exclude: $excludeFormatsStr, maxSize: $maxSize, sourceTypes: $sourceTypesStr',
        );
        print(
          'Source filters - original: $includeOriginal, derivative: $includeDerivative, metadata: $includeMetadata',
        );
        print('Archive has source field: $hasSourceField');
      }

      final filteredJson = IaGetFFI.filterFiles(
        metadataJson,
        includeFormatsStr,
        excludeFormatsStr,
        maxSize,
        sourceTypesStr,
      );

      if (filteredJson != null && filteredJson.isNotEmpty) {
        try {
          final filteredList = jsonDecode(filteredJson) as List<dynamic>;
          final files = filteredList
              .map((json) => ArchiveFile.fromJson(json as Map<String, dynamic>))
              .toList();

          _filteredFiles = files;
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
      final filesJson = jsonEncode(
        selectedFiles.map((f) => f.toJson()).toList(),
      );
      final size = IaGetFFI.calculateTotalSize(filesJson);

      if (kDebugMode) {
        print(
          'Calculated total size: $size bytes for ${selectedFiles.length} files',
        );
      }

      return size;
    } catch (e) {
      if (kDebugMode) {
        print('Failed to calculate total size: $e');
      }
      return 0;
    }
  }

  /// Notify that file selection has changed
  /// This should be called when files are selected/deselected
  void notifyFileSelectionChanged() {
    notifyListeners();
  }

  /// Get all unique file formats/extensions present in the current archive
  /// Returns a cached set for performance
  Set<String> getAvailableFormats() {
    if (_currentMetadata == null) {
      return {};
    }

    final formats = <String>{};
    for (final file in _currentMetadata!.files) {
      // Add format if available
      if (file.format != null && file.format!.isNotEmpty) {
        formats.add(file.format!.toLowerCase());
      }

      // Also extract extension from filename
      final fileName = file.name.toLowerCase();
      final lastDot = fileName.lastIndexOf('.');
      if (lastDot != -1 && lastDot < fileName.length - 1) {
        final ext = fileName.substring(lastDot + 1);
        // Only add if it looks like a valid extension (no spaces, reasonable length)
        if (!ext.contains(' ') && ext.length <= 10 && ext.isNotEmpty) {
          formats.add(ext);
        }
      }
    }

    return formats;
  }

  /// Clear search suggestions
  void clearSuggestions() {
    _suggestions = [];
    notifyListeners();
  }

  /// Clear current metadata and return to search state
  void clearMetadata() {
    _currentMetadata = null;
    _filteredFiles = [];
    _error = null;
    _suggestions = [];
    notifyListeners();
  }

  // Callback functions (static methods for FFI)
  //
  // ⚠️ CRITICAL ANDROID SAFETY NOTE ⚠️
  // These callbacks are invoked from NATIVE RUST THREADS (std::thread::spawn in ffi.rs).
  // Calling Dart code from non-Dart threads causes CRASHES on Android.
  //
  // The current implementation uses a POLLING mechanism (_waitForMetadataCompletion)
  // which safely checks for results from the Dart thread. These callbacks are kept
  // as no-ops to satisfy the FFI interface but should NOT execute any Dart code.
  //
  // See ANDROID_CRASH_ROOT_CAUSE.md for detailed explanation.
  static void _progressCallback(
    double progress,
    Pointer<Utf8> message,
    int userData,
  ) {
    // NO-OP: Do not execute Dart code from native thread callbacks
    // Progress monitoring is handled by polling in _waitForMetadataCompletion
    // Any code here will crash the app on Android due to thread safety violations
  }

  static void _completionCallback(
    bool success,
    Pointer<Utf8> errorMessage,
    int userData,
  ) {
    // NO-OP: Do not execute Dart code from native thread callbacks
    // Completion detection is handled by polling in _waitForMetadataCompletion
    // Any code here will crash the app on Android due to thread safety violations
  }

  /// Wait for metadata fetch completion with timeout
  Future<void> _waitForMetadataCompletion(
    String identifier, {
    required Duration timeout,
  }) async {
    const checkInterval = Duration(milliseconds: 500);
    final endTime = DateTime.now().add(timeout);
    int attempts = 0;
    final maxAttempts = timeout.inMilliseconds ~/ checkInterval.inMilliseconds;

    if (kDebugMode) {
      print(
        'Waiting for metadata completion (timeout: ${timeout.inSeconds}s)...',
      );
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
            print(
              'Metadata available after $attempts attempts (${attempts * checkInterval.inMilliseconds}ms)',
            );
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
        print('Still waiting... ($attempts/$maxAttempts attempts)');
      }

      // Add small initial delay to give the fetch operation time to start
      if (attempts == 1) {
        await Future.delayed(checkInterval);
      }
    }

    throw Exception(
      'Metadata fetch timeout after ${timeout.inSeconds} seconds ($attempts attempts)',
    );
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

  /// Process download progress updates in a background isolate
  /// This prevents UI blocking when processing large numbers of file updates
  static Future<DownloadProgress> processProgressUpdateInIsolate(
    Map<String, dynamic> progressData,
    DownloadProgress? existingProgress,
  ) async {
    return await Isolate.run(
      () => _processProgressIsolate(progressData, existingProgress),
    );
  }

  /// Isolate function to process progress updates
  static DownloadProgress _processProgressIsolate(
    Map<String, dynamic> data,
    DownloadProgress? existing,
  ) {
    // Process progress data without blocking main thread
    final progress = (data['progress'] as num?)?.toDouble();
    final completedFiles = data['completedFiles'] as int?;
    final totalFiles = data['totalFiles'] as int? ?? existing?.totalFiles ?? 0;
    final downloadedBytes = data['downloadedBytes'] as int?;
    final totalBytes = data['totalBytes'] as int?;
    final transferSpeed = (data['transferSpeed'] as num?)?.toDouble();

    // Calculate ETA if we have speed and remaining bytes
    int? etaSeconds;
    if (transferSpeed != null &&
        transferSpeed > 0 &&
        downloadedBytes != null &&
        totalBytes != null) {
      final remainingBytes = totalBytes - downloadedBytes;
      etaSeconds = (remainingBytes / transferSpeed).ceil();
    }

    if (existing != null) {
      return existing.copyWith(
        progress: progress,
        completedFiles: completedFiles,
        totalFiles: totalFiles,
        downloadedBytes: downloadedBytes,
        totalBytes: totalBytes,
        transferSpeed: transferSpeed,
        etaSeconds: etaSeconds,
        currentFile: data['currentFile'] as String?,
      );
    }

    return DownloadProgress(
      downloadId: data['downloadId'] as String? ?? '',
      identifier: data['identifier'] as String? ?? '',
      progress: progress,
      completedFiles: completedFiles,
      totalFiles: totalFiles,
      downloadedBytes: downloadedBytes,
      totalBytes: totalBytes,
      transferSpeed: transferSpeed,
      etaSeconds: etaSeconds,
      currentFile: data['currentFile'] as String?,
      status: DownloadStatus.downloading,
    );
  }
}
