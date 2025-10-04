/// Download State Management Provider
///
/// Manages all download state in Dart, using the simplified FFI interface.
/// This eliminates race conditions by having a single source of truth in Dart.

import 'package:flutter/foundation.dart';
import '../models/archive_metadata.dart';
import '../models/download_progress.dart';
import '../services/ia_get_simple_service.dart';

/// Download state for a single item
class DownloadState {
  final String identifier;
  final ArchiveMetadata? metadata;
  final Map<String, DownloadProgress> fileProgress;
  final String status; // 'idle', 'fetching_metadata', 'downloading', 'complete', 'error'
  final String? error;
  final DateTime? startTime;
  final DateTime? endTime;

  DownloadState({
    required this.identifier,
    this.metadata,
    Map<String, DownloadProgress>? fileProgress,
    this.status = 'idle',
    this.error,
    this.startTime,
    this.endTime,
  }) : fileProgress = fileProgress ?? {};

  DownloadState copyWith({
    String? identifier,
    ArchiveMetadata? metadata,
    Map<String, DownloadProgress>? fileProgress,
    String? status,
    String? error,
    DateTime? startTime,
    DateTime? endTime,
  }) {
    return DownloadState(
      identifier: identifier ?? this.identifier,
      metadata: metadata ?? this.metadata,
      fileProgress: fileProgress ?? this.fileProgress,
      status: status ?? this.status,
      error: error ?? this.error,
      startTime: startTime ?? this.startTime,
      endTime: endTime ?? this.endTime,
    );
  }

  double get overallProgress {
    if (fileProgress.isEmpty) return 0.0;
    
    final totalPercentage = fileProgress.values
        .map((p) => p.percentage)
        .reduce((a, b) => a + b);
    
    return totalPercentage / fileProgress.length;
  }

  int get totalDownloaded {
    return fileProgress.values
        .map((p) => p.downloaded)
        .fold(0, (a, b) => a + b);
  }

  int get totalSize {
    return fileProgress.values
        .map((p) => p.total)
        .fold(0, (a, b) => a + b);
  }
}

/// Download Provider - manages all download state in Dart
///
/// This is the single source of truth for download state, eliminating
/// race conditions between Rust and Dart.
class DownloadProvider extends ChangeNotifier {
  final IaGetSimpleService _service = IaGetSimpleService();
  
  // State management - all in Dart!
  final Map<String, DownloadState> _downloads = {};
  final List<String> _downloadHistory = [];

  /// Get all downloads
  Map<String, DownloadState> get downloads => Map.unmodifiable(_downloads);

  /// Get download history
  List<String> get downloadHistory => List.unmodifiable(_downloadHistory);

  /// Get specific download state
  DownloadState? getDownload(String identifier) {
    return _downloads[identifier];
  }

  /// Start downloading an archive
  Future<void> startDownload(
    String identifier, {
    String? outputDir,
    List<String>? fileFilters,
  }) async {
    if (_downloads.containsKey(identifier)) {
      if (_downloads[identifier]!.status == 'downloading') {
        throw Exception('Download already in progress for $identifier');
      }
    }

    // Initialize download state
    _downloads[identifier] = DownloadState(
      identifier: identifier,
      status: 'fetching_metadata',
      startTime: DateTime.now(),
    );
    notifyListeners();

    try {
      // Fetch metadata
      final metadata = await _service.fetchMetadata(identifier);
      
      _downloads[identifier] = _downloads[identifier]!.copyWith(
        metadata: metadata,
        status: 'downloading',
      );
      notifyListeners();

      // Filter files if specified
      var filesToDownload = metadata.files;
      if (fileFilters != null && fileFilters.isNotEmpty) {
        filesToDownload = filesToDownload.where((file) {
          return fileFilters.any((filter) => file.name.contains(filter));
        }).toList();
      }

      if (filesToDownload.isEmpty) {
        throw Exception('No files to download after filtering');
      }

      // Download each file
      final downloadDir = outputDir ?? '/sdcard/Download/ia-get/$identifier';
      
      for (final file in filesToDownload) {
        final url = 'https://archive.org/download/$identifier/${file.name}';
        final outputPath = '$downloadDir/${file.name}';

        // Initialize progress for this file
        final fileProgress = Map<String, DownloadProgress>.from(
          _downloads[identifier]!.fileProgress,
        );
        fileProgress[file.name] = DownloadProgress(
          downloaded: 0,
          total: file.size ?? 0,
          percentage: 0.0,
          status: 'starting',
        );
        
        _downloads[identifier] = _downloads[identifier]!.copyWith(
          fileProgress: fileProgress,
        );
        notifyListeners();

        // Download with progress tracking
        await _service.downloadFile(
          url,
          outputPath,
          onProgress: (downloaded, total) {
            final updatedProgress = Map<String, DownloadProgress>.from(
              _downloads[identifier]!.fileProgress,
            );
            updatedProgress[file.name] = DownloadProgress(
              downloaded: downloaded,
              total: total,
              percentage: total > 0 ? (downloaded / total) * 100 : 0.0,
              status: 'downloading',
            );
            
            _downloads[identifier] = _downloads[identifier]!.copyWith(
              fileProgress: updatedProgress,
            );
            notifyListeners();
          },
        );

        // Mark file as complete
        final updatedProgress = Map<String, DownloadProgress>.from(
          _downloads[identifier]!.fileProgress,
        );
        updatedProgress[file.name] = updatedProgress[file.name]!.copyWith(
          percentage: 100.0,
          status: 'complete',
        );
        
        _downloads[identifier] = _downloads[identifier]!.copyWith(
          fileProgress: updatedProgress,
        );
        notifyListeners();

        // Validate checksum if available
        if (file.md5 != null && file.md5!.isNotEmpty) {
          try {
            final isValid = await _service.validateChecksum(
              outputPath,
              file.md5!,
              hashType: 'md5',
            );

            if (!isValid) {
              throw Exception('Checksum validation failed for ${file.name}');
            }

            if (kDebugMode) {
              print('Checksum validated for ${file.name}');
            }
          } catch (e) {
            if (kDebugMode) {
              print('Warning: Checksum validation failed: $e');
            }
          }
        }

        // Decompress if it's an archive
        if (_isArchive(file.name)) {
          try {
            final extractedFiles = await _service.decompressFile(
              outputPath,
              downloadDir,
            );
            
            if (kDebugMode) {
              print('Extracted ${extractedFiles.length} files from ${file.name}');
            }
          } catch (e) {
            if (kDebugMode) {
              print('Warning: Failed to decompress ${file.name}: $e');
            }
          }
        }
      }

      // Mark download as complete
      _downloads[identifier] = _downloads[identifier]!.copyWith(
        status: 'complete',
        endTime: DateTime.now(),
      );
      
      // Add to history
      if (!_downloadHistory.contains(identifier)) {
        _downloadHistory.insert(0, identifier);
        if (_downloadHistory.length > 100) {
          _downloadHistory.removeLast();
        }
      }
      
      notifyListeners();
    } catch (e, stackTrace) {
      if (kDebugMode) {
        print('Download failed: $e');
        print('Stack trace: $stackTrace');
      }

      _downloads[identifier] = _downloads[identifier]!.copyWith(
        status: 'error',
        error: e.toString(),
        endTime: DateTime.now(),
      );
      notifyListeners();

      rethrow;
    }
  }

  /// Cancel a download
  Future<void> cancelDownload(String identifier) async {
    if (!_downloads.containsKey(identifier)) {
      return;
    }

    // Note: In a full implementation, you'd need to signal the Rust side
    // to cancel the download. For now, we just update the state.
    _downloads[identifier] = _downloads[identifier]!.copyWith(
      status: 'cancelled',
      endTime: DateTime.now(),
    );
    notifyListeners();
  }

  /// Clear a download from state
  void clearDownload(String identifier) {
    _downloads.remove(identifier);
    notifyListeners();
  }

  /// Clear all completed downloads
  void clearCompletedDownloads() {
    _downloads.removeWhere((_, state) => state.status == 'complete');
    notifyListeners();
  }

  /// Clear download history
  void clearHistory() {
    _downloadHistory.clear();
    notifyListeners();
  }

  /// Check if file is an archive
  bool _isArchive(String filename) {
    final lowercaseName = filename.toLowerCase();
    return lowercaseName.endsWith('.zip') ||
        lowercaseName.endsWith('.tar') ||
        lowercaseName.endsWith('.tar.gz') ||
        lowercaseName.endsWith('.tgz') ||
        lowercaseName.endsWith('.tar.bz2') ||
        lowercaseName.endsWith('.tbz2') ||
        lowercaseName.endsWith('.tar.xz') ||
        lowercaseName.endsWith('.txz') ||
        lowercaseName.endsWith('.gz') ||
        lowercaseName.endsWith('.bz2') ||
        lowercaseName.endsWith('.xz');
  }
}

/// Extension to add copyWith to DownloadProgress
extension DownloadProgressCopyWith on DownloadProgress {
  DownloadProgress copyWith({
    int? downloaded,
    int? total,
    double? percentage,
    String? status,
    String? error,
  }) {
    return DownloadProgress(
      downloaded: downloaded ?? this.downloaded,
      total: total ?? this.total,
      percentage: percentage ?? this.percentage,
      status: status ?? this.status,
      error: error ?? this.error,
    );
  }
}
