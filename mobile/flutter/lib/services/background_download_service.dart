import 'dart:async';
import 'dart:isolate';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import '../models/download_progress.dart';
import '../models/archive_metadata.dart';
import 'ia_get_service.dart';

/// Service for managing background downloads with Android WorkManager integration
class BackgroundDownloadService extends ChangeNotifier {
  static const _platform = MethodChannel('com.internetarchive.helper/background_download');
  
  final Map<String, DownloadProgress> _activeDownloads = {};
  Timer? _statusUpdateTimer;
  bool _isInitialized = false;

  Map<String, DownloadProgress> get activeDownloads => Map.unmodifiable(_activeDownloads);
  bool get hasActiveDownloads => _activeDownloads.isNotEmpty;
  int get activeDownloadCount => _activeDownloads.length;

  /// Initialize the background download service
  Future<void> initialize() async {
    if (_isInitialized) return;
    
    try {
      // Setup method channel communication with Android
      _platform.setMethodCallHandler(_handleMethodCall);
      
      // Initialize native background service
      await _platform.invokeMethod('initialize');
      
      // Start periodic status updates
      _statusUpdateTimer = Timer.periodic(
        const Duration(seconds: 2),
        (_) => _updateDownloadStatuses(),
      );
      
      _isInitialized = true;
    } catch (e) {
      debugPrint('Failed to initialize background download service: $e');
    }
  }

  /// Handle method calls from Android native code
  Future<void> _handleMethodCall(MethodCall call) async {
    switch (call.method) {
      case 'onDownloadProgress':
        _handleDownloadProgress(call.arguments);
        break;
      case 'onDownloadComplete':
        _handleDownloadComplete(call.arguments);
        break;
      case 'onDownloadError':
        _handleDownloadError(call.arguments);
        break;
    }
  }

  /// Start a background download
  Future<String?> startBackgroundDownload({
    required String identifier,
    required List<String> selectedFiles,
    required String downloadPath,
    String? includeFormats,
    String? excludeFormats,
    String? maxSize,
  }) async {
    try {
      final downloadId = await _platform.invokeMethod('startDownload', {
        'identifier': identifier,
        'selectedFiles': selectedFiles,
        'downloadPath': downloadPath,
        'includeFormats': includeFormats,
        'excludeFormats': excludeFormats,
        'maxSize': maxSize,
      });

      if (downloadId != null) {
        _activeDownloads[downloadId] = DownloadProgress(
          downloadId: downloadId,
          identifier: identifier,
          totalFiles: selectedFiles.length,
          status: DownloadStatus.queued,
        );
        notifyListeners();
      }

      return downloadId;
    } catch (e) {
      debugPrint('Failed to start background download: $e');
      return null;
    }
  }

  /// Cancel a background download
  Future<bool> cancelDownload(String downloadId) async {
    try {
      final success = await _platform.invokeMethod('cancelDownload', {
        'downloadId': downloadId,
      });

      if (success == true) {
        _activeDownloads.remove(downloadId);
        notifyListeners();
      }

      return success == true;
    } catch (e) {
      debugPrint('Failed to cancel download: $e');
      return false;
    }
  }

  /// Pause a background download
  Future<bool> pauseDownload(String downloadId) async {
    try {
      final success = await _platform.invokeMethod('pauseDownload', {
        'downloadId': downloadId,
      });

      if (success == true && _activeDownloads.containsKey(downloadId)) {
        _activeDownloads[downloadId] = _activeDownloads[downloadId]!.copyWith(
          status: DownloadStatus.paused,
        );
        notifyListeners();
      }

      return success == true;
    } catch (e) {
      debugPrint('Failed to pause download: $e');
      return false;
    }
  }

  /// Resume a paused download
  Future<bool> resumeDownload(String downloadId) async {
    try {
      final success = await _platform.invokeMethod('resumeDownload', {
        'downloadId': downloadId,
      });

      if (success == true && _activeDownloads.containsKey(downloadId)) {
        _activeDownloads[downloadId] = _activeDownloads[downloadId]!.copyWith(
          status: DownloadStatus.downloading,
        );
        notifyListeners();
      }

      return success == true;
    } catch (e) {
      debugPrint('Failed to resume download: $e');
      return false;
    }
  }

  /// Update download statuses from native side
  Future<void> _updateDownloadStatuses() async {
    if (_activeDownloads.isEmpty) return;

    try {
      final statuses = await _platform.invokeMethod('getDownloadStatuses');
      if (statuses is Map) {
        for (final entry in statuses.entries) {
          final downloadId = entry.key as String;
          final statusData = entry.value as Map;
          
          if (_activeDownloads.containsKey(downloadId)) {
            _activeDownloads[downloadId] = _parseProgressUpdate(downloadId, statusData);
          }
        }
        notifyListeners();
      }
    } catch (e) {
      debugPrint('Failed to update download statuses: $e');
    }
  }

  /// Handle download progress update from native
  void _handleDownloadProgress(Map<dynamic, dynamic> data) {
    final downloadId = data['downloadId'] as String?;
    if (downloadId == null) return;

    _activeDownloads[downloadId] = _parseProgressUpdate(downloadId, data);
    notifyListeners();
  }

  /// Handle download completion from native
  void _handleDownloadComplete(Map<dynamic, dynamic> data) {
    final downloadId = data['downloadId'] as String?;
    if (downloadId == null) return;

    if (_activeDownloads.containsKey(downloadId)) {
      _activeDownloads[downloadId] = _activeDownloads[downloadId]!.copyWith(
        status: DownloadStatus.completed,
        progress: 1.0,
        completedFiles: _activeDownloads[downloadId]!.totalFiles,
      );
      notifyListeners();
      
      // Remove completed download after a delay
      Timer(const Duration(seconds: 5), () {
        _activeDownloads.remove(downloadId);
        notifyListeners();
      });
    }
  }

  /// Handle download error from native
  void _handleDownloadError(Map<dynamic, dynamic> data) {
    final downloadId = data['downloadId'] as String?;
    final errorMessage = data['error'] as String?;
    if (downloadId == null) return;

    if (_activeDownloads.containsKey(downloadId)) {
      _activeDownloads[downloadId] = _activeDownloads[downloadId]!.copyWith(
        status: DownloadStatus.error,
        errorMessage: errorMessage,
      );
      notifyListeners();
    }
  }

  /// Parse progress update from native data
  DownloadProgress _parseProgressUpdate(String downloadId, Map<dynamic, dynamic> data) {
    final existing = _activeDownloads[downloadId];
    if (existing == null) {
      return DownloadProgress(
        downloadId: downloadId,
        identifier: data['identifier'] as String? ?? '',
        totalFiles: data['totalFiles'] as int? ?? 0,
        status: DownloadStatus.queued,
      );
    }

    return existing.copyWith(
      progress: (data['progress'] as num?)?.toDouble(),
      completedFiles: data['completedFiles'] as int?,
      currentFile: data['currentFile'] as String?,
      downloadedBytes: data['downloadedBytes'] as int?,
      totalBytes: data['totalBytes'] as int?,
      transferSpeed: data['transferSpeed'] as double?,
      status: _parseDownloadStatus(data['status'] as String?),
    );
  }

  /// Parse download status from string
  DownloadStatus _parseDownloadStatus(String? status) {
    switch (status?.toLowerCase()) {
      case 'queued':
        return DownloadStatus.queued;
      case 'downloading':
        return DownloadStatus.downloading;
      case 'paused':
        return DownloadStatus.paused;
      case 'completed':
        return DownloadStatus.completed;
      case 'error':
        return DownloadStatus.error;
      case 'cancelled':
        return DownloadStatus.cancelled;
      default:
        return DownloadStatus.queued;
    }
  }

  @override
  void dispose() {
    _statusUpdateTimer?.cancel();
    _activeDownloads.clear();
    super.dispose();
  }
}