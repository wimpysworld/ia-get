/// Optimized Download State Management Provider
///
/// Takes full advantage of the simplified FFI architecture:
/// - Zero race conditions (all state in Dart)
/// - Concurrent downloads with intelligent queuing
/// - Real-time progress tracking
/// - Automatic retry logic
/// - Clean cancellation
/// - Download statistics and history

import 'dart:async';
import 'dart:collection';
import 'package:flutter/foundation.dart';
import '../models/archive_metadata.dart';
import '../models/download_progress.dart';
import '../services/ia_get_simple_service.dart';

/// Download state for a single file
enum FileDownloadState {
  queued,
  downloading,
  completed,
  failed,
  cancelled,
}

/// Optimized download information with performance metrics
class DownloadInfo {
  final String url;
  final String localPath;
  final FileDownloadState state;
  final double progress;
  final String? error;
  final DateTime startTime;
  final DateTime? endTime;
  final int? totalBytes;
  final int? downloadedBytes;

  DownloadInfo({
    required this.url,
    required this.localPath,
    required this.state,
    this.progress = 0.0,
    this.error,
    required this.startTime,
    this.endTime,
    this.totalBytes,
    this.downloadedBytes,
  });

  DownloadInfo copyWith({
    FileDownloadState? state,
    double? progress,
    String? error,
    DateTime? endTime,
    int? totalBytes,
    int? downloadedBytes,
  }) {
    return DownloadInfo(
      url: url,
      localPath: localPath,
      state: state ?? this.state,
      progress: progress ?? this.progress,
      error: error ?? this.error,
      startTime: startTime,
      endTime: endTime ?? this.endTime,
      totalBytes: totalBytes ?? this.totalBytes,
      downloadedBytes: downloadedBytes ?? this.downloadedBytes,
    );
  }

  Duration get duration => (endTime ?? DateTime.now()).difference(startTime);

  double get downloadSpeedMBps {
    if (downloadedBytes == null) return 0.0;
    final seconds = duration.inMilliseconds / 1000.0;
    if (seconds == 0) return 0.0;
    return (downloadedBytes! / 1024 / 1024) / seconds;
  }

  String get statusText {
    switch (state) {
      case FileDownloadState.queued:
        return 'Queued';
      case FileDownloadState.downloading:
        return '${(progress * 100).toStringAsFixed(1)}% - ${downloadSpeedMBps.toStringAsFixed(2)} MB/s';
      case FileDownloadState.completed:
        return 'Completed';
      case FileDownloadState.failed:
        return 'Failed: ${error ?? "Unknown error"}';
      case FileDownloadState.cancelled:
        return 'Cancelled';
    }
  }
}

/// Optimized Download Provider
///
/// Key optimizations enabled by simplified FFI:
/// 1. Concurrent downloads without Rust state conflicts
/// 2. Efficient queue management
/// 3. Real-time progress without polling
/// 4. Simple cancellation via Dart state
/// 5. Automatic retry with exponential backoff
class DownloadProviderOptimized extends ChangeNotifier {
  final IaGetSimpleService _service = IaGetSimpleService();

  // ALL STATE IN DART (single source of truth)
  final Map<String, DownloadInfo> _downloads = {};
  final Map<String, ArchiveMetadata> _metadataCache = {};
  final List<String> _history = [];

  // Concurrent download management
  int maxConcurrentDownloads = 3;
  final Queue<String> _downloadQueue = Queue();
  int _activeDownloads = 0;

  // Auto-retry configuration
  bool autoRetry = true;
  int maxRetries = 3;
  final Map<String, int> _retryCount = {};

  // Public getters
  UnmodifiableMapView<String, DownloadInfo> get downloads =>
      UnmodifiableMapView(_downloads);
  UnmodifiableListView<String> get history => UnmodifiableListView(_history);
  UnmodifiableMapView<String, ArchiveMetadata> get metadataCache =>
      UnmodifiableMapView(_metadataCache);

  int get activeDownloadCount => _activeDownloads;
  int get queuedDownloadCount => _downloadQueue.length;

  List<DownloadInfo> get activeDownloads => _downloads.values
      .where((d) => d.state == FileDownloadState.downloading)
      .toList();

  List<DownloadInfo> get completedDownloads => _downloads.values
      .where((d) => d.state == FileDownloadState.completed)
      .toList();

  List<DownloadInfo> get failedDownloads => _downloads.values
      .where((d) => d.state == FileDownloadState.failed)
      .toList();

  /// Fetch metadata with caching (optimized for speed)
  Future<ArchiveMetadata> fetchMetadata(String identifier) async {
    // Return cached if available
    if (_metadataCache.containsKey(identifier)) {
      return _metadataCache[identifier]!;
    }

    // Fetch from Rust (stateless, runs in isolate)
    final metadata = await _service.fetchMetadata(identifier);

    // Cache for future use
    _metadataCache[identifier] = metadata;
    notifyListeners();

    return metadata;
  }

  /// Start download (intelligently queued)
  Future<void> startDownload({
    required String url,
    required String localPath,
    String? expectedHash,
    String? hashType,
    bool autoDecompress = false,
  }) async {
    // Skip if already exists
    if (_downloads.containsKey(url)) {
      if (kDebugMode) {
        print('Download already tracked: $url');
      }
      return;
    }

    // Initialize download info
    _downloads[url] = DownloadInfo(
      url: url,
      localPath: localPath,
      state: FileDownloadState.queued,
      startTime: DateTime.now(),
    );
    _retryCount[url] = 0;
    notifyListeners();

    // Add to queue
    _downloadQueue.add(url);
    _processQueue();
  }

  /// Process download queue (respects concurrency limit)
  Future<void> _processQueue() async {
    while (_downloadQueue.isNotEmpty &&
        _activeDownloads < maxConcurrentDownloads) {
      final url = _downloadQueue.removeFirst();

      // Skip if cancelled while queued
      if (_downloads[url]?.state == FileDownloadState.cancelled) {
        continue;
      }

      _activeDownloads++;
      _executeDownload(url);
    }
  }

  /// Execute download (runs in isolate, non-blocking)
  Future<void> _executeDownload(String url) async {
    try {
      // Update to downloading state
      _downloads[url] = _downloads[url]!.copyWith(
        state: FileDownloadState.downloading,
      );
      notifyListeners();

      // Download file (stateless Rust call via isolate)
      final result = await _service.downloadFile(
        url,
        _downloads[url]!.localPath,
        (downloaded, total) {
          // Real-time progress update (no polling!)
          if (_downloads[url]?.state == FileDownloadState.downloading) {
            _downloads[url] = _downloads[url]!.copyWith(
              progress: downloaded / total,
              downloadedBytes: downloaded,
              totalBytes: total,
            );
            notifyListeners();
          }
        },
      );

      if (result.success) {
        // Success!
        _downloads[url] = _downloads[url]!.copyWith(
          state: FileDownloadState.completed,
          progress: 1.0,
          endTime: DateTime.now(),
        );
        _history.add(url);
        if (kDebugMode) {
          print('Download completed: $url');
        }
      } else {
        // Failed - retry if enabled
        await _handleDownloadFailure(url, result.error ?? 'Unknown error');
      }
    } catch (e) {
      await _handleDownloadFailure(url, e.toString());
    } finally {
      _activeDownloads--;
      notifyListeners();
      _processQueue(); // Start next download
    }
  }

  /// Handle download failure with retry logic
  Future<void> _handleDownloadFailure(String url, String error) async {
    final retries = _retryCount[url] ?? 0;

    if (autoRetry && retries < maxRetries) {
      // Retry with exponential backoff
      _retryCount[url] = retries + 1;
      final delaySeconds = (1 << retries); // 1, 2, 4 seconds
      
      if (kDebugMode) {
        print('Download failed, retrying in $delaySeconds seconds: $url');
      }

      await Future.delayed(Duration(seconds: delaySeconds));

      // Re-queue for retry
      _downloads[url] = _downloads[url]!.copyWith(
        state: FileDownloadState.queued,
        error: 'Retrying... (attempt ${retries + 1}/$maxRetries)',
      );
      _downloadQueue.add(url);
      notifyListeners();
    } else {
      // Max retries reached or auto-retry disabled
      _downloads[url] = _downloads[url]!.copyWith(
        state: FileDownloadState.failed,
        error: error,
        endTime: DateTime.now(),
      );
      if (kDebugMode) {
        print('Download failed permanently: $url - $error');
      }
    }
  }

  /// Cancel download (clean cancellation via Dart state)
  void cancelDownload(String url) {
    if (!_downloads.containsKey(url)) return;

    final download = _downloads[url]!;

    if (download.state == FileDownloadState.downloading) {
      _downloads[url] = download.copyWith(
        state: FileDownloadState.cancelled,
        endTime: DateTime.now(),
      );
      _activeDownloads--;
      notifyListeners();
      _processQueue();
    } else if (download.state == FileDownloadState.queued) {
      _downloadQueue.remove(url);
      _downloads[url] = download.copyWith(
        state: FileDownloadState.cancelled,
        endTime: DateTime.now(),
      );
      notifyListeners();
    }

    if (kDebugMode) {
      print('Download cancelled: $url');
    }
  }

  /// Retry failed download
  Future<void> retryDownload(String url) async {
    if (!_downloads.containsKey(url)) return;

    final download = _downloads[url]!;
    if (download.state != FileDownloadState.failed) return;

    // Reset retry counter and restart
    _retryCount[url] = 0;
    _downloads.remove(url);
    
    await startDownload(
      url: download.url,
      localPath: download.localPath,
    );
  }

  /// Batch cancel all downloads
  void cancelAll() {
    final activeUrls = activeDownloads.map((d) => d.url).toList();
    for (final url in activeUrls) {
      cancelDownload(url);
    }
  }

  /// Clear completed downloads
  void clearCompleted() {
    _downloads.removeWhere((_, d) => d.state == FileDownloadState.completed);
    notifyListeners();
  }

  /// Clear all downloads
  void clearAll() {
    _downloads.clear();
    _downloadQueue.clear();
    _activeDownloads = 0;
    _retryCount.clear();
    notifyListeners();
  }

  /// Get comprehensive statistics
  Map<String, dynamic> getStatistics() {
    final completed = completedDownloads;
    final failed = failedDownloads;
    final totalBytes = completed.fold<int>(
      0,
      (sum, d) => sum + (d.downloadedBytes ?? 0),
    );
    final totalDuration = completed.fold<Duration>(
      Duration.zero,
      (sum, d) => sum + d.duration,
    );
    final speeds = completed
        .where((d) => d.downloadSpeedMBps > 0)
        .map((d) => d.downloadSpeedMBps)
        .toList();

    return {
      'total': _downloads.length,
      'completed': completed.length,
      'failed': failed.length,
      'active': activeDownloadCount,
      'queued': queuedDownloadCount,
      'totalBytes': totalBytes,
      'totalMB': (totalBytes / 1024 / 1024).toStringAsFixed(2),
      'averageSpeedMBps': speeds.isEmpty
          ? 0.0
          : (speeds.reduce((a, b) => a + b) / speeds.length)
              .toStringAsFixed(2),
      'totalDuration': totalDuration.toString(),
      'successRate': _downloads.isEmpty
          ? 0.0
          : (completed.length / _downloads.length * 100).toStringAsFixed(1),
    };
  }

  /// Export download history as JSON
  Map<String, dynamic> exportHistory() {
    return {
      'downloads': _downloads.values.map((d) => {
        'url': d.url,
        'localPath': d.localPath,
        'state': d.state.toString(),
        'progress': d.progress,
        'startTime': d.startTime.toIso8601String(),
        'endTime': d.endTime?.toIso8601String(),
        'totalBytes': d.totalBytes,
        'downloadedBytes': d.downloadedBytes,
        'speedMBps': d.downloadSpeedMBps,
      }).toList(),
      'statistics': getStatistics(),
    };
  }
}
