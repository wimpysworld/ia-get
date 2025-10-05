/// Download statistics model for tracking overall download performance
/// 
/// Provides a type-safe interface for download statistics data that was
/// previously returned as `Map<String, dynamic>`.
class DownloadStatistics {
  final int activeDownloads;
  final int completedDownloads;
  final int queuedDownloads;
  final int totalFiles;
  final int totalBytes;
  final int activeBytesDownloaded;
  final double averageSpeed; // bytes per second
  final int sessionDurationSeconds;

  const DownloadStatistics({
    required this.activeDownloads,
    required this.completedDownloads,
    required this.queuedDownloads,
    required this.totalFiles,
    required this.totalBytes,
    required this.activeBytesDownloaded,
    required this.averageSpeed,
    required this.sessionDurationSeconds,
  });

  /// Total number of downloads across all states
  int get totalDownloads =>
      activeDownloads + completedDownloads + queuedDownloads;

  /// Session duration as a Duration object
  Duration get sessionDuration => Duration(seconds: sessionDurationSeconds);

  /// Check if there are any active downloads
  bool get hasActiveDownloads => activeDownloads > 0;

  /// Check if there are any queued downloads
  bool get hasQueuedDownloads => queuedDownloads > 0;

  /// Format average speed as a human-readable string
  String formatAverageSpeed() {
    if (averageSpeed <= 0) return '-';
    
    if (averageSpeed < 1024) {
      return '${averageSpeed.toStringAsFixed(0)} B/s';
    } else if (averageSpeed < 1024 * 1024) {
      return '${(averageSpeed / 1024).toStringAsFixed(1)} KB/s';
    } else if (averageSpeed < 1024 * 1024 * 1024) {
      return '${(averageSpeed / (1024 * 1024)).toStringAsFixed(1)} MB/s';
    } else {
      return '${(averageSpeed / (1024 * 1024 * 1024)).toStringAsFixed(2)} GB/s';
    }
  }

  /// Convert to JSON for serialization
  Map<String, dynamic> toJson() {
    return {
      'activeDownloads': activeDownloads,
      'completedDownloads': completedDownloads,
      'queuedDownloads': queuedDownloads,
      'totalFiles': totalFiles,
      'totalBytes': totalBytes,
      'activeBytesDownloaded': activeBytesDownloaded,
      'averageSpeed': averageSpeed,
      'sessionDuration': sessionDurationSeconds,
    };
  }

  /// Create from JSON
  factory DownloadStatistics.fromJson(Map<String, dynamic> json) {
    return DownloadStatistics(
      activeDownloads: json['activeDownloads'] as int? ?? 0,
      completedDownloads: json['completedDownloads'] as int? ?? 0,
      queuedDownloads: json['queuedDownloads'] as int? ?? 0,
      totalFiles: json['totalFiles'] as int? ?? 0,
      totalBytes: json['totalBytes'] as int? ?? 0,
      activeBytesDownloaded: json['activeBytesDownloaded'] as int? ?? 0,
      averageSpeed: (json['averageSpeed'] as num?)?.toDouble() ?? 0.0,
      sessionDurationSeconds: json['sessionDuration'] as int? ?? 0,
    );
  }

  /// Create a copy with updated fields
  DownloadStatistics copyWith({
    int? activeDownloads,
    int? completedDownloads,
    int? queuedDownloads,
    int? totalFiles,
    int? totalBytes,
    int? activeBytesDownloaded,
    double? averageSpeed,
    int? sessionDurationSeconds,
  }) {
    return DownloadStatistics(
      activeDownloads: activeDownloads ?? this.activeDownloads,
      completedDownloads: completedDownloads ?? this.completedDownloads,
      queuedDownloads: queuedDownloads ?? this.queuedDownloads,
      totalFiles: totalFiles ?? this.totalFiles,
      totalBytes: totalBytes ?? this.totalBytes,
      activeBytesDownloaded:
          activeBytesDownloaded ?? this.activeBytesDownloaded,
      averageSpeed: averageSpeed ?? this.averageSpeed,
      sessionDurationSeconds:
          sessionDurationSeconds ?? this.sessionDurationSeconds,
    );
  }

  @override
  String toString() {
    return 'DownloadStatistics('
        'active: $activeDownloads, '
        'completed: $completedDownloads, '
        'queued: $queuedDownloads, '
        'speed: ${formatAverageSpeed()}'
        ')';
  }

  @override
  bool operator ==(Object other) {
    if (identical(this, other)) return true;

    return other is DownloadStatistics &&
        other.activeDownloads == activeDownloads &&
        other.completedDownloads == completedDownloads &&
        other.queuedDownloads == queuedDownloads &&
        other.totalFiles == totalFiles &&
        other.totalBytes == totalBytes &&
        other.activeBytesDownloaded == activeBytesDownloaded &&
        other.averageSpeed == averageSpeed &&
        other.sessionDurationSeconds == sessionDurationSeconds;
  }

  @override
  int get hashCode {
    return Object.hash(
      activeDownloads,
      completedDownloads,
      queuedDownloads,
      totalFiles,
      totalBytes,
      activeBytesDownloaded,
      averageSpeed,
      sessionDurationSeconds,
    );
  }
}
