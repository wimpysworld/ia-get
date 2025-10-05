import 'dart:convert';
import 'package:flutter/foundation.dart';
import 'package:http/http.dart' as http;
import '../models/archive_metadata.dart';
import '../models/search_result.dart';
import 'internet_archive_api.dart';

/// Pure Dart/Flutter Archive Service
///
/// This service replaces the FFI-based implementation with a pure Dart/Flutter
/// solution that directly calls the Internet Archive API.
///
/// Benefits over FFI approach:
/// - No native library dependencies
/// - Easier to debug and maintain
/// - Better error messages
/// - Platform-independent (works on all Flutter platforms)
/// - No build complexity with native code
class ArchiveServiceDart extends ChangeNotifier {
  final InternetArchiveApi _api = InternetArchiveApi();

  // State
  bool _isInitialized = true; // No initialization needed
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

  // Download tracking
  final Map<String, CancellationToken> _activeDownloads = {};

  // Getters
  bool get isInitialized => _isInitialized;
  bool get isLoading => _isLoading;
  String? get error => _error;
  ArchiveMetadata? get currentMetadata => _currentMetadata;
  List<ArchiveFile> get filteredFiles => _filteredFiles;
  bool get canCancel => _isLoading;
  List<SearchResult> get suggestions => _suggestions;

  /// Initialize the service
  Future<void> initialize() async {
    _isInitialized = true;
    _error = null;
    notifyListeners();
  }

  /// Fetch metadata for an archive
  Future<void> fetchMetadata(String identifier) async {
    final trimmedIdentifier = identifier.trim();
    if (trimmedIdentifier.isEmpty) {
      _error = 'Invalid identifier: cannot be empty';
      notifyListeners();
      return;
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
    } catch (e, stackTrace) {
      _error = 'Failed to fetch metadata: ${e.toString()}';
      _currentMetadata = null;
      _filteredFiles = [];
      
      if (kDebugMode) {
        print('Error fetching metadata: $e');
        print('Stack trace: $stackTrace');
      }
    } finally {
      _isLoading = false;
      notifyListeners();
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

  /// Internal method to apply filters
  Future<void> _applyFilters() async {
    if (_currentMetadata == null) return;

    try {
      _filteredFiles = _currentMetadata!.files.where((file) {
        // Apply include formats filter
        if (_includeFormats != null && _includeFormats!.isNotEmpty) {
          final formats = _includeFormats!
              .split(',')
              .map((f) => f.trim().toLowerCase())
              .toList();
          final fileFormat = file.format?.toLowerCase() ?? '';
          if (!formats.contains(fileFormat)) {
            return false;
          }
        }

        // Apply exclude formats filter
        if (_excludeFormats != null && _excludeFormats!.isNotEmpty) {
          final formats = _excludeFormats!
              .split(',')
              .map((f) => f.trim().toLowerCase())
              .toList();
          final fileFormat = file.format?.toLowerCase() ?? '';
          if (formats.contains(fileFormat)) {
            return false;
          }
        }

        // Apply max size filter
        if (_maxSize != null && _maxSize!.isNotEmpty) {
          final maxSizeBytes = _parseSize(_maxSize!);
          if (maxSizeBytes != null && file.size != null) {
            if (file.size! > maxSizeBytes) {
              return false;
            }
          }
        }

        // Apply source type filter
        if (_sourceTypes != null && _sourceTypes!.isNotEmpty) {
          final sources = _sourceTypes!
              .split(',')
              .map((s) => s.trim().toLowerCase())
              .toList();
          if (!sources.contains(file.source.toLowerCase())) {
            return false;
          }
        }

        return true;
      }).toList();

      notifyListeners();
    } catch (e) {
      if (kDebugMode) {
        print('Error applying filters: $e');
      }
    }
  }

  /// Parse size string (e.g., "10MB", "1.5GB") to bytes
  int? _parseSize(String sizeStr) {
    final pattern = RegExp(r'^([\d.]+)\s*(B|KB|MB|GB|TB)?$', caseSensitive: false);
    final match = pattern.firstMatch(sizeStr.trim());
    
    if (match == null) return null;

    final value = double.tryParse(match.group(1)!);
    if (value == null) return null;

    final unit = (match.group(2) ?? 'B').toUpperCase();
    
    switch (unit) {
      case 'B':
        return value.toInt();
      case 'KB':
        return (value * 1024).toInt();
      case 'MB':
        return (value * 1024 * 1024).toInt();
      case 'GB':
        return (value * 1024 * 1024 * 1024).toInt();
      case 'TB':
        return (value * 1024 * 1024 * 1024 * 1024).toInt();
      default:
        return null;
    }
  }

  /// Download a file
  Future<String> downloadFile(
    String url,
    String outputPath, {
    void Function(int downloaded, int total)? onProgress,
  }) async {
    final token = CancellationToken();
    _activeDownloads[url] = token;

    try {
      final result = await _api.downloadFile(
        url,
        outputPath,
        onProgress: onProgress,
        cancellationToken: token,
      );
      return result;
    } finally {
      _activeDownloads.remove(url);
    }
  }

  /// Cancel a download
  void cancelDownload(String url) {
    _activeDownloads[url]?.cancel();
  }

  /// Validate file checksum
  Future<bool> validateChecksum(
    String filePath,
    String expectedHash,
    String hashType,
  ) async {
    return await _api.validateChecksum(filePath, expectedHash, hashType);
  }

  /// Search Internet Archive (basic implementation using search API)
  Future<void> searchArchive(String query) async {
    if (query.trim().isEmpty) {
      _suggestions = [];
      notifyListeners();
      return;
    }

    try {
      final searchUrl = 'https://archive.org/advancedsearch.php'
          '?q=${Uri.encodeComponent(query)}'
          '&fl[]=identifier,title,description,mediatype'
          '&rows=20'
          '&page=1'
          '&output=json';

      final response = await http.get(Uri.parse(searchUrl));

      if (response.statusCode == 200) {
        final data = json.decode(response.body);
        final docs = data['response']['docs'] as List;

        _suggestions = docs.map((doc) {
          return SearchResult(
            identifier: doc['identifier'] ?? '',
            title: doc['title'] ?? '',
            description: doc['description'] ?? '',
            mediaType: doc['mediatype'] ?? '',
          );
        }).toList();

        notifyListeners();
      }
    } catch (e) {
      if (kDebugMode) {
        print('Error searching archive: $e');
      }
    }
  }

  /// Clear metadata cache
  void clearMetadataCache() {
    _currentMetadata = null;
    _filteredFiles = [];
    notifyListeners();
  }

  /// Get API statistics
  Map<String, dynamic> getApiStats() {
    return _api.getStats();
  }

  @override
  void dispose() {
    // Cancel any active downloads
    for (var token in _activeDownloads.values) {
      token.cancel();
    }
    _activeDownloads.clear();
    
    _api.dispose();
    super.dispose();
  }
}
