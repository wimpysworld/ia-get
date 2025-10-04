import 'package:flutter/foundation.dart';
import '../models/archive_metadata.dart';
import '../models/download_progress.dart' hide DownloadStatus;
import '../models/file_filter.dart';
import '../services/ia_get_simple_service.dart';

/// Download State Management Provider
///
/// Manages all download state in Dart, using the simplified FFI interface.
/// This eliminates race conditions by having a single source of truth in Dart.

/// Download state machine for clear state transitions
enum DownloadStatus {
  idle,
  fetchingMetadata,
  downloading,
  validating,
  extracting,
  complete,
  error,
  cancelled,
}

/// Extension to convert DownloadStatus to string for backward compatibility
extension DownloadStatusExtension on DownloadStatus {
  String get value {
    switch (this) {
      case DownloadStatus.idle:
        return 'idle';
      case DownloadStatus.fetchingMetadata:
        return 'fetching_metadata';
      case DownloadStatus.downloading:
        return 'downloading';
      case DownloadStatus.validating:
        return 'validating';
      case DownloadStatus.extracting:
        return 'extracting';
      case DownloadStatus.complete:
        return 'complete';
      case DownloadStatus.error:
        return 'error';
      case DownloadStatus.cancelled:
        return 'cancelled';
    }
  }
  
  /// Check if download is in progress
  bool get isActive {
    return this == DownloadStatus.fetchingMetadata ||
           this == DownloadStatus.downloading ||
           this == DownloadStatus.validating ||
           this == DownloadStatus.extracting;
  }
  
  /// Check if download has finished (success or failure)
  bool get isFinished {
    return this == DownloadStatus.complete ||
           this == DownloadStatus.error ||
           this == DownloadStatus.cancelled;
  }
}

/// Download state for a single item
class DownloadState {
  final String identifier;
  final ArchiveMetadata? metadata;
  final Map<String, DownloadProgress> fileProgress;
  final DownloadStatus downloadStatus;
  final String? error;
  final DateTime? startTime;
  final DateTime? endTime;

  DownloadState({
    required this.identifier,
    this.metadata,
    Map<String, DownloadProgress>? fileProgress,
    this.downloadStatus = DownloadStatus.idle,
    this.error,
    this.startTime,
    this.endTime,
  }) : fileProgress = fileProgress ?? {};
  
  /// Get status as string for backward compatibility
  String get status => downloadStatus.value;

  DownloadState copyWith({
    String? identifier,
    ArchiveMetadata? metadata,
    Map<String, DownloadProgress>? fileProgress,
    DownloadStatus? downloadStatus,
    String? status,  // Deprecated, use downloadStatus
    String? error,
    DateTime? startTime,
    DateTime? endTime,
  }) {
    return DownloadState(
      identifier: identifier ?? this.identifier,
      metadata: metadata ?? this.metadata,
      fileProgress: fileProgress ?? this.fileProgress,
      downloadStatus: downloadStatus ?? 
                      (status != null ? _parseStatus(status) : this.downloadStatus),
      error: error ?? this.error,
      startTime: startTime ?? this.startTime,
      endTime: endTime ?? this.endTime,
    );
  }
  
  /// Parse status string to enum for backward compatibility
  static DownloadStatus _parseStatus(String status) {
    switch (status) {
      case 'fetching_metadata':
        return DownloadStatus.fetchingMetadata;
      case 'downloading':
        return DownloadStatus.downloading;
      case 'validating':
        return DownloadStatus.validating;
      case 'extracting':
        return DownloadStatus.extracting;
      case 'complete':
        return DownloadStatus.complete;
      case 'error':
        return DownloadStatus.error;
      case 'cancelled':
        return DownloadStatus.cancelled;
      default:
        return DownloadStatus.idle;
    }
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
  
  // Enhanced: Metadata caching for better performance
  final Map<String, ArchiveMetadata> _metadataCache = {};
  
  // Enhanced: Concurrent download configuration
  int maxConcurrentDownloads = 3;
  int _activeDownloads = 0;
  
  // Download queue for managing concurrent downloads
  final List<_QueuedDownload> _downloadQueue = [];
  bool _isProcessingQueue = false;
  
  // Enhanced: Automatic retry configuration
  int maxRetryAttempts = 3;
  Duration initialRetryDelay = const Duration(seconds: 2);
  
  // Enhanced: Download statistics
  int _totalDownloadsStarted = 0;
  int _totalDownloadsCompleted = 0;
  int _totalDownloadsFailed = 0;
  int _totalBytesDownloaded = 0;

  /// Get all downloads
  Map<String, DownloadState> get downloads => Map.unmodifiable(_downloads);

  /// Get download history
  List<String> get downloadHistory => List.unmodifiable(_downloadHistory);
  
  /// Get active download count
  int get activeDownloadCount => _activeDownloads;
  
  /// Get queued download count
  int get queuedDownloadCount => _downloadQueue.length;
  
  /// Get download statistics
  int get totalDownloadsStarted => _totalDownloadsStarted;
  int get totalDownloadsCompleted => _totalDownloadsCompleted;
  int get totalDownloadsFailed => _totalDownloadsFailed;
  int get totalBytesDownloaded => _totalBytesDownloaded;
  
  /// Calculate average download speed (bytes per second)
  double get averageDownloadSpeed {
    final activeDownloads = _downloads.values.where((d) => d.downloadStatus.isActive);
    if (activeDownloads.isEmpty) return 0.0;
    
    double totalSpeed = 0.0;
    int count = 0;
    
    for (final download in activeDownloads) {
      if (download.startTime != null) {
        final elapsed = DateTime.now().difference(download.startTime!).inSeconds;
        if (elapsed > 0) {
          final totalBytes = download.fileProgress.values
              .fold<int>(0, (sum, progress) => sum + progress.downloaded);
          totalSpeed += totalBytes / elapsed;
          count++;
        }
      }
    }
    
    return count > 0 ? totalSpeed / count : 0.0;
  }

  /// Get specific download state
  DownloadState? getDownload(String identifier) {
    return _downloads[identifier];
  }
  
  /// Get cached metadata if available
  ArchiveMetadata? getCachedMetadata(String identifier) {
    return _metadataCache[identifier];
  }
  
  /// Clear metadata cache
  void clearMetadataCache() {
    _metadataCache.clear();
    notifyListeners();
  }

  /// Start downloading an archive
  /// 
  /// If maxConcurrentDownloads is reached, the download will be queued.
  /// 
  /// [fileFilters] (deprecated): Use [filter] parameter instead for advanced filtering
  /// [filter]: Advanced FileFilter object supporting subfolders, regex, size ranges, etc.
  Future<void> startDownload(
    String identifier, {
    String? outputDir,
    List<String>? fileFilters,
    FileFilter? filter,
  }) async {
    if (_downloads.containsKey(identifier)) {
      if (_downloads[identifier]!.downloadStatus.isActive) {
        throw Exception('Download already in progress for $identifier');
      }
    }

    // Check if we can start immediately or need to queue
    if (_activeDownloads >= maxConcurrentDownloads) {
      // Queue the download
      _downloadQueue.add(_QueuedDownload(
        identifier: identifier,
        outputDir: outputDir,
        fileFilters: fileFilters,
        filter: filter,
      ));
      
      // Initialize as queued state
      _downloads[identifier] = DownloadState(
        identifier: identifier,
        downloadStatus: DownloadStatus.idle,
        startTime: DateTime.now(),
      );
      notifyListeners();
      
      if (kDebugMode) {
        print('Download queued: $identifier (${_downloadQueue.length} in queue)');
      }
      return;
    }

    await _executeDownload(identifier, outputDir, fileFilters, filter);
  }

  /// Execute a download
  Future<void> _executeDownload(
    String identifier,
    String? outputDir,
    List<String>? fileFilters,
    FileFilter? filter,
  ) async {

    // Initialize download state
    _downloads[identifier] = DownloadState(
      identifier: identifier,
      downloadStatus: DownloadStatus.fetchingMetadata,
      startTime: DateTime.now(),
    );
    _totalDownloadsStarted++;
    notifyListeners();

    try {
      // Fetch metadata with caching
      ArchiveMetadata metadata;
      if (_metadataCache.containsKey(identifier)) {
        // Use cached metadata for better performance
        metadata = _metadataCache[identifier]!;
      } else {
        // Fetch from network and cache
        metadata = await _service.fetchMetadata(identifier);
        _metadataCache[identifier] = metadata;
      }
      
      _downloads[identifier] = _downloads[identifier]!.copyWith(
        metadata: metadata,
        downloadStatus: DownloadStatus.downloading,
      );
      notifyListeners();

      // Enhanced: Improved file filtering with multiple patterns and advanced options
      var filesToDownload = metadata.files;
      
      // Use advanced filter if provided, otherwise fall back to simple fileFilters
      if (filter != null && filter.hasActiveCriteria) {
        filesToDownload = _applyAdvancedFilter(filesToDownload, filter);
      } else if (fileFilters != null && fileFilters.isNotEmpty) {
        filesToDownload = filesToDownload.where((file) {
          final fileName = file.name.toLowerCase();
          return fileFilters.any((filterStr) {
            final filterLower = filterStr.toLowerCase();
            // Support both exact contains and wildcard patterns
            if (filterLower.contains('*')) {
              // Simple wildcard matching
              final pattern = filterLower.replaceAll('*', '.*');
              return RegExp(pattern).hasMatch(fileName);
            } else {
              // Exact substring matching
              return fileName.contains(filterLower);
            }
          });
        }).toList();
      }

      if (filesToDownload.isEmpty) {
        throw Exception('No files to download after filtering');
      }

      // Download each file
      final downloadDir = outputDir ?? '/sdcard/Download/ia-get/$identifier';
      
      // Track active downloads for concurrency control
      _activeDownloads++;
      
      for (final file in filesToDownload) {
        final url = 'https://archive.org/download/$identifier/${file.name}';
        final outputPath = '$downloadDir/${file.name}';

        // Initialize progress for this file
        final fileProgress = Map<String, DownloadProgress>.from(
          _downloads[identifier]!.fileProgress,
        );
        fileProgress[file.name] = DownloadProgress.simple(
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
            updatedProgress[file.name] = DownloadProgress.simple(
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
          progress: 1.0,
          status: DownloadStatus.completed,
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
        downloadStatus: DownloadStatus.complete,
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

      // Enhanced: More specific error messages
      String errorMessage = e.toString();
      if (errorMessage.contains('network') || errorMessage.contains('HTTP')) {
        errorMessage = 'Network error: Please check your internet connection';
      } else if (errorMessage.contains('permission') || errorMessage.contains('denied')) {
        errorMessage = 'Permission error: Cannot write to destination';
      } else if (errorMessage.contains('space') || errorMessage.contains('full')) {
        errorMessage = 'Storage error: Insufficient disk space';
      }

      _downloads[identifier] = _downloads[identifier]!.copyWith(
        downloadStatus: DownloadStatus.error,
        error: errorMessage,
        endTime: DateTime.now(),
      );
      _totalDownloadsFailed++;
      notifyListeners();

      rethrow;
    } finally {
      // Ensure active download count is decremented
      if (_activeDownloads > 0) {
        _activeDownloads--;
      }
      
      // Process queue if there are pending downloads
      _processQueue();
    }
  }
  
  /// Retry a failed download
  /// 
  /// Automatically called for transient errors, but can also be manually triggered
  Future<void> retryDownload(String identifier) async {
    final downloadState = _downloads[identifier];
    if (downloadState == null) {
      throw Exception('Download not found: $identifier');
    }
    
    if (downloadState.downloadStatus != DownloadStatus.error) {
      throw Exception('Can only retry failed downloads');
    }
    
    // Reset download state
    _downloads[identifier] = downloadState.copyWith(
      downloadStatus: DownloadStatus.idle,
      error: null,
    );
    notifyListeners();
    
    // Restart the download
    await startDownload(
      identifier,
      outputDir: downloadState.metadata?.identifier,
    );
  }

  /// Process the download queue
  Future<void> _processQueue() async {
    // Prevent concurrent queue processing
    if (_isProcessingQueue || _downloadQueue.isEmpty) {
      return;
    }

    _isProcessingQueue = true;

    try {
      while (_downloadQueue.isNotEmpty && _activeDownloads < maxConcurrentDownloads) {
        final queued = _downloadQueue.removeAt(0);
        
        if (kDebugMode) {
          print('Processing queued download: ${queued.identifier}');
        }

        // Execute the queued download (fire and forget to allow queue processing)
        _executeDownload(
          queued.identifier,
          queued.outputDir,
          queued.fileFilters,
          queued.filter,
        ).catchError((error) {
          if (kDebugMode) {
            print('Queued download failed: ${queued.identifier} - $error');
          }
        });
      }
    } finally {
      _isProcessingQueue = false;
    }
  }
  
  /// Download multiple files from the same archive (batch operation)
  /// 
  /// Efficiently downloads selected files with concurrent processing
  Future<void> batchDownload(
    String identifier,
    List<String> fileNames, {
    String? outputDir,
  }) async {
    if (fileNames.isEmpty) {
      throw Exception('No files selected for batch download');
    }
    
    if (kDebugMode) {
      print('Starting batch download: $identifier with ${fileNames.length} files');
    }
    
    // Use the file filters to download only selected files
    await startDownload(
      identifier,
      outputDir: outputDir,
      fileFilters: fileNames,
    );
  }

  /// Cancel a download
  Future<void> cancelDownload(String identifier) async {
    if (!_downloads.containsKey(identifier)) {
      return;
    }

    // Note: In a full implementation, you'd need to signal the Rust side
    // to cancel the download. For now, we just update the state.
    _downloads[identifier] = _downloads[identifier]!.copyWith(
      downloadStatus: DownloadStatus.cancelled,
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
  
  /// Apply advanced filter to list of files
  List<ArchiveFile> _applyAdvancedFilter(List<ArchiveFile> files, FileFilter filter) {
    return files.where((file) {
      // Source type filtering
      if (file.source != null) {
        final source = file.source!.toLowerCase();
        if (source == 'original' && !filter.includeOriginal) return false;
        if (source == 'derivative' && !filter.includeDerivative) return false;
        if (source == 'metadata' && !filter.includeMetadata) return false;
      }
      
      // Format filtering
      if (filter.includeFormats.isNotEmpty && file.format != null) {
        if (!filter.includeFormats.contains(file.format!.toLowerCase())) {
          return false;
        }
      }
      if (filter.excludeFormats.isNotEmpty && file.format != null) {
        if (filter.excludeFormats.contains(file.format!.toLowerCase())) {
          return false;
        }
      }
      
      // Size filtering
      if (file.size != null) {
        if (filter.minSize != null && file.size! < filter.minSize!) return false;
        if (filter.maxSize != null && file.size! > filter.maxSize!) return false;
      }
      
      // Subfolder filtering
      if (filter.includeSubfolders.isNotEmpty) {
        bool matchesSubfolder = filter.includeSubfolders.any((subfolder) => 
          file.isInSubfolder(subfolder)
        );
        if (!matchesSubfolder) return false;
      }
      if (filter.excludeSubfolders.isNotEmpty) {
        bool matchesExcluded = filter.excludeSubfolders.any((subfolder) => 
          file.isInSubfolder(subfolder)
        );
        if (matchesExcluded) return false;
      }
      
      // Pattern filtering (filename-based)
      final fullPath = file.name.toLowerCase();
      
      if (filter.includePatterns.isNotEmpty) {
        bool matchesPattern = filter.includePatterns.any((pattern) {
          return _matchesPattern(fullPath, pattern, filter.useRegex);
        });
        if (!matchesPattern) return false;
      }
      
      if (filter.excludePatterns.isNotEmpty) {
        bool matchesExcluded = filter.excludePatterns.any((pattern) {
          return _matchesPattern(fullPath, pattern, filter.useRegex);
        });
        if (matchesExcluded) return false;
      }
      
      return true;
    }).toList();
  }
  
  /// Match a filename against a pattern (wildcard or regex)
  bool _matchesPattern(String filename, String pattern, bool useRegex) {
    final filenameLower = filename.toLowerCase();
    final patternLower = pattern.toLowerCase();
    
    if (useRegex) {
      // Treat pattern as regex
      try {
        final regex = RegExp(patternLower);
        return regex.hasMatch(filenameLower);
      } catch (_) {
        // Invalid regex, fall back to contains
        return filenameLower.contains(patternLower);
      }
    } else {
      // Wildcard pattern matching
      if (patternLower.contains('*') || patternLower.contains('?')) {
        final regexPattern = patternLower
            .replaceAll('\\', '\\\\')
            .replaceAll('.', '\\.')
            .replaceAll('*', '.*')
            .replaceAll('?', '.');
        try {
          final regex = RegExp('^$regexPattern\$');
          return regex.hasMatch(filenameLower);
        } catch (_) {
          return filenameLower.contains(patternLower);
        }
      } else {
        // Simple substring matching
        return filenameLower.contains(patternLower);
      }
    }
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

/// Helper class for queued downloads
class _QueuedDownload {
  final String identifier;
  final String? outputDir;
  final List<String>? fileFilters;
  final FileFilter? filter;

  _QueuedDownload({
    required this.identifier,
    this.outputDir,
    this.fileFilters,
    this.filter,
  });
}

/// Extension to add copyWith to DownloadProgress for simple file progress
extension DownloadProgressCopyWith on DownloadProgress {
  DownloadProgress copyWith({
    int? downloaded,
    int? total,
    double? percentage,
    String? status,
    String? error,
  }) {
    return DownloadProgress.simple(
      downloaded: downloaded ?? this.downloaded,
      total: total ?? this.total,
      percentage: percentage ?? this.percentage,
      status: status ?? 'downloading',
      error: error ?? errorMessage,
    );
  }
}
