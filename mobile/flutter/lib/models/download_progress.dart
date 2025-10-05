/// Status of a download
enum DownloadStatus { queued, downloading, paused, completed, error, cancelled }

/// Download progress information
class DownloadProgress {
  final String downloadId;
  final String identifier;
  final int sessionId;
  final double? progress; // 0.0 to 1.0, null for indeterminate
  final String? currentFile;
  final double? currentFileProgress;
  final double? transferSpeed; // bytes per second
  final int? etaSeconds;
  final int? completedFiles;
  final int totalFiles;
  final int? downloadedBytes;
  final int? totalBytes;
  final DownloadStatus status;
  final String? errorMessage;
  final DateTime startTime;
  final int retryCount; // Track number of retry attempts

  DownloadProgress({
    required this.downloadId,
    required this.identifier,
    this.sessionId = 0,
    this.progress,
    this.currentFile,
    this.currentFileProgress,
    this.transferSpeed,
    this.etaSeconds,
    this.completedFiles,
    required this.totalFiles,
    this.downloadedBytes,
    this.totalBytes,
    required this.status,
    this.errorMessage,
    DateTime? startTime,
    this.retryCount = 0,
  }) : startTime = startTime ?? DateTime.now();

  // Legacy compatibility getters for field access
  int get downloaded => downloadedBytes ?? 0;
  int get total => totalBytes ?? 0;
  double get percentage => progress != null ? progress! * 100 : 0.0;

  /// Legacy factory constructor for simple file progress tracking
  /// Used by download_provider.dart and archive_service.dart
  factory DownloadProgress.simple({
    required int downloaded,
    required int total,
    required double percentage,
    required String status,
    String? error,
  }) {
    // Parse status string to enum
    DownloadStatus statusEnum;
    switch (status) {
      case 'queued':
        statusEnum = DownloadStatus.queued;
        break;
      case 'downloading':
        statusEnum = DownloadStatus.downloading;
        break;
      case 'paused':
        statusEnum = DownloadStatus.paused;
        break;
      case 'completed':
      case 'complete':
        statusEnum = DownloadStatus.completed;
        break;
      case 'error':
        statusEnum = DownloadStatus.error;
        break;
      case 'cancelled':
        statusEnum = DownloadStatus.cancelled;
        break;
      default:
        statusEnum = DownloadStatus.queued;
    }

    return DownloadProgress(
      downloadId: 'file-${DateTime.now().millisecondsSinceEpoch}',
      identifier: '',
      downloadedBytes: downloaded,
      totalBytes: total,
      progress: total > 0 ? downloaded / total : 0.0,
      totalFiles: 1,
      status: statusEnum,
      errorMessage: error,
    );
  }

  /// Create a copy with updated fields
  DownloadProgress copyWith({
    String? downloadId,
    String? identifier,
    int? sessionId,
    double? progress,
    String? currentFile,
    double? currentFileProgress,
    double? transferSpeed,
    int? etaSeconds,
    int? completedFiles,
    int? totalFiles,
    int? downloadedBytes,
    int? totalBytes,
    DownloadStatus? status,
    String? errorMessage,
    DateTime? startTime,
    int? retryCount,
  }) {
    return DownloadProgress(
      downloadId: downloadId ?? this.downloadId,
      identifier: identifier ?? this.identifier,
      sessionId: sessionId ?? this.sessionId,
      progress: progress ?? this.progress,
      currentFile: currentFile ?? this.currentFile,
      currentFileProgress: currentFileProgress ?? this.currentFileProgress,
      transferSpeed: transferSpeed ?? this.transferSpeed,
      etaSeconds: etaSeconds ?? this.etaSeconds,
      completedFiles: completedFiles ?? this.completedFiles,
      totalFiles: totalFiles ?? this.totalFiles,
      downloadedBytes: downloadedBytes ?? this.downloadedBytes,
      totalBytes: totalBytes ?? this.totalBytes,
      status: status ?? this.status,
      errorMessage: errorMessage ?? this.errorMessage,
      startTime: startTime ?? this.startTime,
      retryCount: retryCount ?? this.retryCount,
    );
  }

  // Legacy compatibility getters
  double get overallProgress => progress ?? 0.0;
  int get downloadSpeed => (transferSpeed ?? 0).toInt();

  String get speedFormatted {
    if (transferSpeed == null) return '0 B/s';
    const units = ['B/s', 'KB/s', 'MB/s', 'GB/s'];
    double speed = transferSpeed!;
    int unitIndex = 0;

    while (speed >= 1024 && unitIndex < units.length - 1) {
      speed /= 1024;
      unitIndex++;
    }

    return '${speed.toStringAsFixed(speed >= 100 ? 0 : 1)} ${units[unitIndex]}';
  }

  String get etaFormatted {
    if (etaSeconds == null || etaSeconds! <= 0) return 'Unknown';

    int hours = etaSeconds! ~/ 3600;
    int minutes = (etaSeconds! % 3600) ~/ 60;
    int seconds = etaSeconds! % 60;

    if (hours > 0) {
      return '${hours}h ${minutes}m';
    } else if (minutes > 0) {
      return '${minutes}m ${seconds}s';
    } else {
      return '${seconds}s';
    }
  }

  String get downloadedFormatted {
    if (downloadedBytes == null) return '0 B';
    const units = ['B', 'KB', 'MB', 'GB', 'TB'];
    double bytes = downloadedBytes!.toDouble();
    int unitIndex = 0;

    while (bytes >= 1024 && unitIndex < units.length - 1) {
      bytes /= 1024;
      unitIndex++;
    }

    return '${bytes.toStringAsFixed(bytes >= 100 ? 0 : 1)} ${units[unitIndex]}';
  }

  String get totalSizeFormatted {
    if (totalBytes == null) return 'Unknown';
    const units = ['B', 'KB', 'MB', 'GB', 'TB'];
    double bytes = totalBytes!.toDouble();
    int unitIndex = 0;

    while (bytes >= 1024 && unitIndex < units.length - 1) {
      bytes /= 1024;
      unitIndex++;
    }

    return '${bytes.toStringAsFixed(bytes >= 100 ? 0 : 1)} ${units[unitIndex]}';
  }

  /// Convert to JSON for serialization
  Map<String, dynamic> toJson() {
    return {
      'downloadId': downloadId,
      'identifier': identifier,
      'sessionId': sessionId,
      'progress': progress,
      'currentFile': currentFile,
      'currentFileProgress': currentFileProgress,
      'transferSpeed': transferSpeed,
      'etaSeconds': etaSeconds,
      'completedFiles': completedFiles,
      'totalFiles': totalFiles,
      'downloadedBytes': downloadedBytes,
      'totalBytes': totalBytes,
      'status': status.name,
      'errorMessage': errorMessage,
      'startTime': startTime.toIso8601String(),
      'retryCount': retryCount,
    };
  }

  /// Create from JSON
  factory DownloadProgress.fromJson(Map<String, dynamic> json) {
    // Parse status string to enum
    DownloadStatus statusEnum;
    final statusStr = json['status'] as String? ?? 'queued';
    switch (statusStr) {
      case 'queued':
        statusEnum = DownloadStatus.queued;
        break;
      case 'downloading':
        statusEnum = DownloadStatus.downloading;
        break;
      case 'paused':
        statusEnum = DownloadStatus.paused;
        break;
      case 'completed':
        statusEnum = DownloadStatus.completed;
        break;
      case 'error':
        statusEnum = DownloadStatus.error;
        break;
      case 'cancelled':
        statusEnum = DownloadStatus.cancelled;
        break;
      default:
        statusEnum = DownloadStatus.queued;
    }

    return DownloadProgress(
      downloadId: json['downloadId'] as String? ?? '',
      identifier: json['identifier'] as String? ?? '',
      sessionId: json['sessionId'] as int? ?? 0,
      progress: (json['progress'] as num?)?.toDouble(),
      currentFile: json['currentFile'] as String?,
      currentFileProgress: (json['currentFileProgress'] as num?)?.toDouble(),
      transferSpeed: (json['transferSpeed'] as num?)?.toDouble(),
      etaSeconds: json['etaSeconds'] as int?,
      completedFiles: json['completedFiles'] as int?,
      totalFiles: json['totalFiles'] as int? ?? 0,
      downloadedBytes: json['downloadedBytes'] as int?,
      totalBytes: json['totalBytes'] as int?,
      status: statusEnum,
      errorMessage: json['errorMessage'] as String?,
      startTime: json['startTime'] != null
          ? DateTime.parse(json['startTime'] as String)
          : null,
      retryCount: json['retryCount'] as int? ?? 0,
    );
  }
}
