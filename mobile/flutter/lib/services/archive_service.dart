import 'dart:convert';
import 'package:flutter/foundation.dart';
import 'package:http/http.dart' as http;
import '../models/archive_metadata.dart';
import '../models/search_result.dart';
import 'internet_archive_api.dart';
import '../core/constants/internet_archive_constants.dart';

/// Archive Service - Pure Dart/Flutter implementation
///
/// This service provides a clean interface for interacting with the Internet Archive,
/// now using a pure Dart implementation instead of FFI.
///
/// Benefits of pure Dart approach:
/// - No native library dependencies or build complexity
/// - Works on all Flutter platforms (Android, iOS, Web, Desktop)
/// - Easier to debug and maintain
/// - Better error messages and handling
/// - No race conditions from FFI boundaries

class ArchiveService extends ChangeNotifier {
  final InternetArchiveApi _api = InternetArchiveApi();

  // State
  bool _isInitialized = true; // No initialization needed for pure Dart
  bool _isLoading = false;
  String? _error;
  ArchiveMetadata? _currentMetadata;
  List<ArchiveFile> _filteredFiles = [];
  List<SearchResult> _suggestions = [];

  // File filtering state
  String? _includeFormats;
  String? _excludeFormats;
  String? _maxSize;
  String? _sourceTypes;

  // Getters
  bool get isInitialized => _isInitialized;
  bool get isLoading => _isLoading;
  String? get error => _error;
  ArchiveMetadata? get currentMetadata => _currentMetadata;
  List<ArchiveFile> get filteredFiles => _filteredFiles;
  bool get canCancel => _isLoading; // Simplified - no request tracking needed
  List<SearchResult> get suggestions => _suggestions;

  /// Initialize the service
  Future<void> initialize() async {
    _isInitialized = true;
    _error = null;
    notifyListeners();
  }

  /// Fetch metadata for an archive
  Future<ArchiveMetadata> fetchMetadata(String identifier) async {
    final trimmedIdentifier = identifier.trim();
    if (trimmedIdentifier.isEmpty) {
      _error = 'Invalid identifier: cannot be empty';
      notifyListeners();
      throw Exception(_error);
    }

    _isLoading = true;
    _error = null;
    _currentMetadata = null;
    _filteredFiles = [];
    _suggestions = [];
    notifyListeners();

    try {
      // Use pure Dart API to fetch metadata
      final metadata = await _api.fetchMetadata(trimmedIdentifier);
      
      _currentMetadata = metadata;
      _filteredFiles = metadata.files;
      
      // Apply current filters if any
      if (_includeFormats != null || _excludeFormats != null || 
          _maxSize != null || _sourceTypes != null) {
        await _applyFilters();
      }

      _error = null;
      _isLoading = false;
      notifyListeners();
      
      return metadata;
    } catch (e, stackTrace) {
      _error = 'Failed to fetch metadata: ${e.toString()}';
      _currentMetadata = null;
      _filteredFiles = [];
      _isLoading = false;
      
      if (kDebugMode) {
        print('Error fetching metadata: $e');
        print('Stack trace: $stackTrace');
      }
      
      notifyListeners();
      rethrow;
    }
  }

  /// Apply file filters
  Future<void> applyFilters({
    String? includeFormats,
    String? excludeFormats,
    String? maxSize,
    String? sourceTypes,
  }) async {
    _includeFormats = includeFormats;
    _excludeFormats = excludeFormats;
    _maxSize = maxSize;
    _sourceTypes = sourceTypes;

    await _applyFilters();
  }

  /// Apply file filters (with Flutter screen compatible signature)
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

    // Convert list parameters to comma-separated strings
    _includeFormats = includeFormats != null && includeFormats.isNotEmpty
        ? includeFormats.join(',')
        : null;
    _excludeFormats = excludeFormats != null && excludeFormats.isNotEmpty
        ? excludeFormats.join(',')
        : null;
    _maxSize = maxSize;

    // Build source types filter
    final sourceTypes = <String>[];
    if (includeOriginal) sourceTypes.add('original');
    if (includeDerivative) sourceTypes.add('derivative');
    if (includeMetadata) sourceTypes.add('metadata');
    _sourceTypes = sourceTypes.isNotEmpty ? sourceTypes.join(',') : null;

    // Apply filters synchronously (no await needed)
    _applyFiltersSync();
  }

  /// Internal method to apply filters
  Future<void> _applyFilters() async {
    if (_currentMetadata == null) return;

    try {
      // Start with all files
      var files = _currentMetadata!.files;

      // Apply include formats filter
      if (_includeFormats != null && _includeFormats!.isNotEmpty) {
        final formats = _includeFormats!.split(',').map((f) => f.trim().toLowerCase()).toList();
        files = files.where((file) {
          final ext = file.name.split('.').last.toLowerCase();
          return formats.contains(ext);
        }).toList();
      }

      // Apply exclude formats filter
      if (_excludeFormats != null && _excludeFormats!.isNotEmpty) {
        final formats = _excludeFormats!.split(',').map((f) => f.trim().toLowerCase()).toList();
        files = files.where((file) {
          final ext = file.name.split('.').last.toLowerCase();
          return !formats.contains(ext);
        }).toList();
      }

      // Apply max size filter
      if (_maxSize != null && _maxSize!.isNotEmpty) {
        final maxBytes = _parseSize(_maxSize!);
        if (maxBytes > 0) {
          files = files.where((file) {
            return (file.size ?? 0) <= maxBytes;
          }).toList();
        }
      }

      // Apply source types filter (original vs derivative)
      if (_sourceTypes != null && _sourceTypes!.isNotEmpty) {
        final types = _sourceTypes!.split(',').map((t) => t.trim().toLowerCase()).toList();
        files = files.where((file) {
          final source = file.source?.toLowerCase() ?? '';
          return types.any((type) => source.contains(type));
        }).toList();
      }

      _filteredFiles = files;
      notifyListeners();
    } catch (e) {
      if (kDebugMode) {
        print('Error applying filters: $e');
      }
      // On error, show all files
      _filteredFiles = _currentMetadata!.files;
      notifyListeners();
    }
  }

  /// Internal method to apply filters synchronously
  void _applyFiltersSync() {
    if (_currentMetadata == null) return;

    try {
      // Start with all files
      var files = _currentMetadata!.files;

      // Apply include formats filter
      if (_includeFormats != null && _includeFormats!.isNotEmpty) {
        final formats = _includeFormats!.split(',').map((f) => f.trim().toLowerCase()).toList();
        files = files.where((file) {
          final ext = file.name.split('.').last.toLowerCase();
          final format = file.format?.toLowerCase() ?? '';
          return formats.contains(ext) || formats.contains(format);
        }).toList();
      }

      // Apply exclude formats filter
      if (_excludeFormats != null && _excludeFormats!.isNotEmpty) {
        final formats = _excludeFormats!.split(',').map((f) => f.trim().toLowerCase()).toList();
        files = files.where((file) {
          final ext = file.name.split('.').last.toLowerCase();
          final format = file.format?.toLowerCase() ?? '';
          return !formats.contains(ext) && !formats.contains(format);
        }).toList();
      }

      // Apply max size filter
      if (_maxSize != null && _maxSize!.isNotEmpty) {
        final maxBytes = _parseSize(_maxSize!);
        if (maxBytes > 0) {
          files = files.where((file) {
            return (file.size ?? 0) <= maxBytes;
          }).toList();
        }
      }

      // Apply source types filter (original vs derivative)
      if (_sourceTypes != null && _sourceTypes!.isNotEmpty) {
        final types = _sourceTypes!.split(',').map((t) => t.trim().toLowerCase()).toList();
        files = files.where((file) {
          final source = file.source?.toLowerCase() ?? '';
          // If no source types are specified, include all
          if (types.isEmpty) return true;
          // Check if file source matches any of the allowed types
          return types.any((type) => source.contains(type) || (source.isEmpty && type == 'original'));
        }).toList();
      }

      _filteredFiles = files;
      _error = null;
      notifyListeners();
    } catch (e) {
      if (kDebugMode) {
        print('Error applying filters: $e');
      }
      // On error, show all files
      _filteredFiles = _currentMetadata!.files;
      _error = 'Error applying filters: ${e.toString()}';
      notifyListeners();
    }
  }

  /// Parse size string (e.g., "10MB", "1GB") to bytes
  int _parseSize(String sizeStr) {
    final regex = RegExp(r'(\d+(?:\.\d+)?)\s*([KMGT]?B?)', caseSensitive: false);
    final match = regex.firstMatch(sizeStr.trim());
    
    if (match == null) return 0;
    
    final value = double.tryParse(match.group(1) ?? '0') ?? 0;
    final unit = (match.group(2) ?? '').toUpperCase();
    
    switch (unit) {
      case 'KB':
      case 'K':
        return (value * 1024).toInt();
      case 'MB':
      case 'M':
        return (value * 1024 * 1024).toInt();
      case 'GB':
      case 'G':
        return (value * 1024 * 1024 * 1024).toInt();
      case 'TB':
      case 'T':
        return (value * 1024 * 1024 * 1024 * 1024).toInt();
      default:
        return value.toInt();
    }
  }

  /// Clear current metadata
  void clearMetadata() {
    _currentMetadata = null;
    _filteredFiles = [];
    _error = null;
    _suggestions = [];
    _includeFormats = null;
    _excludeFormats = null;
    _maxSize = null;
    _sourceTypes = null;
    notifyListeners();
  }

  /// Clear filters
  void clearFilters() {
    _includeFormats = null;
    _excludeFormats = null;
    _maxSize = null;
    _sourceTypes = null;
    
    if (_currentMetadata != null) {
      _filteredFiles = _currentMetadata!.files;
      notifyListeners();
    }
  }

  /// Calculate total size of selected files
  int calculateTotalSize(List<ArchiveFile> files) {
    return files.fold<int>(0, (sum, file) => sum + (file.size ?? 0));
  }

  /// Search for archives using Internet Archive API
  Future<void> searchArchives(String query) async {
    if (query.isEmpty) {
      _suggestions = [];
      notifyListeners();
      return;
    }

    _isLoading = true;
    notifyListeners();

    try {
      // Use standardized search URL builder
      final searchUrl = IAUtils.buildSearchUrl(
        query: query,
        rows: IASearchParams.defaultRows,
        fields: IASearchParams.defaultFields,
      );
      
      final response = await http.get(Uri.parse(searchUrl));
      
      if (response.statusCode == 200) {
        final jsonData = json.decode(response.body);
        final docs = jsonData['response']?['docs'] as List<dynamic>? ?? [];
        
        _suggestions = docs
            .map((doc) => SearchResult.fromJson(doc as Map<String, dynamic>))
            .toList();
      } else {
        if (kDebugMode) {
          print('Search API returned status ${response.statusCode}');
        }
        _suggestions = [];
      }
      
      notifyListeners();
    } catch (e) {
      if (kDebugMode) {
        print('Error searching archives: $e');
      }
      _suggestions = [];
      notifyListeners();
    } finally {
      _isLoading = false;
      notifyListeners();
    }
  }

  /// Notify listeners of file selection changes (for UI updates)
  void notifyFileSelectionChanged() {
    notifyListeners();
  }

  /// Cancel current operation (no-op for simplified FFI but kept for compatibility)
  void cancelOperation() {
    _isLoading = false;
    notifyListeners();
  }

  /// Get available formats from current metadata
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
      final parts = file.name.split('.');
      if (parts.length > 1) {
        formats.add(parts.last.toLowerCase());
      }
    }
    return formats;
  }

  /// Download a file from the given URL to the specified output path
  /// 
  /// [url] - The URL to download from
  /// [outputPath] - The local file path where the file will be saved
  /// [onProgress] - Optional callback for download progress updates (downloaded bytes, total bytes)
  Future<void> downloadFile(
    String url,
    String outputPath, {
    Function(int downloaded, int total)? onProgress,
  }) async {
    try {
      await _api.downloadFile(url, outputPath, onProgress: onProgress);
    } catch (e) {
      if (kDebugMode) {
        print('Error downloading file: $e');
      }
      rethrow;
    }
  }

  /// Validate file checksum
  /// 
  /// [filePath] - Path to the file to validate
  /// [expectedHash] - The expected hash value
  /// [hashType] - Type of hash (md5, sha1, sha256, etc.)
  /// 
  /// Returns true if the checksum matches, false otherwise
  Future<bool> validateChecksum(
    String filePath,
    String expectedHash, {
    String hashType = 'md5',
  }) async {
    try {
      return await _api.validateChecksum(filePath, expectedHash, hashType);
    } catch (e) {
      if (kDebugMode) {
        print('Error validating checksum: $e');
      }
      rethrow;
    }
  }

  /// Decompress/extract an archive file
  /// 
  /// [archivePath] - Path to the archive file
  /// [outputDir] - Directory where files will be extracted
  /// 
  /// Returns a list of extracted file paths
  Future<List<String>> decompressFile(
    String archivePath,
    String outputDir,
  ) async {
    try {
      return await _api.decompressFile(archivePath, outputDir);
    } catch (e) {
      if (kDebugMode) {
        print('Error decompressing file: $e');
      }
      rethrow;
    }
  }

  @override
  void dispose() {
    _api.dispose();
    _currentMetadata = null;
    _filteredFiles = [];
    _suggestions = [];
    super.dispose();
  }
}
