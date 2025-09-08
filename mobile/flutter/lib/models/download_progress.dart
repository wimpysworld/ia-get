/// Download progress information
class DownloadProgress {
  final int sessionId;
  final double overallProgress;
  final String? currentFile;
  final double currentFileProgress;
  final int downloadSpeed; // bytes per second
  final int etaSeconds;
  final int completedFiles;
  final int totalFiles;
  final int downloadedBytes;
  final int totalBytes;
  
  DownloadProgress({
    required this.sessionId,
    required this.overallProgress,
    this.currentFile,
    required this.currentFileProgress,
    required this.downloadSpeed,
    required this.etaSeconds,
    required this.completedFiles,
    required this.totalFiles,
    required this.downloadedBytes,
    required this.totalBytes,
  });
  
  String get speedFormatted {
    const units = ['B/s', 'KB/s', 'MB/s', 'GB/s'];
    double speed = downloadSpeed.toDouble();
    int unitIndex = 0;
    
    while (speed >= 1024 && unitIndex < units.length - 1) {
      speed /= 1024;
      unitIndex++;
    }
    
    return '${speed.toStringAsFixed(speed >= 100 ? 0 : 1)} ${units[unitIndex]}';
  }
  
  String get etaFormatted {
    if (etaSeconds <= 0) return 'Unknown';
    
    int hours = etaSeconds ~/ 3600;
    int minutes = (etaSeconds % 3600) ~/ 60;
    int seconds = etaSeconds % 60;
    
    if (hours > 0) {
      return '${hours}h ${minutes}m';
    } else if (minutes > 0) {
      return '${minutes}m ${seconds}s';
    } else {
      return '${seconds}s';
    }
  }
  
  String get downloadedFormatted {
    const units = ['B', 'KB', 'MB', 'GB', 'TB'];
    double bytes = downloadedBytes.toDouble();
    int unitIndex = 0;
    
    while (bytes >= 1024 && unitIndex < units.length - 1) {
      bytes /= 1024;
      unitIndex++;
    }
    
    return '${bytes.toStringAsFixed(bytes >= 100 ? 0 : 1)} ${units[unitIndex]}';
  }
  
  String get totalSizeFormatted {
    const units = ['B', 'KB', 'MB', 'GB', 'TB'];
    double bytes = totalBytes.toDouble();
    int unitIndex = 0;
    
    while (bytes >= 1024 && unitIndex < units.length - 1) {
      bytes /= 1024;
      unitIndex++;
    }
    
    return '${bytes.toStringAsFixed(bytes >= 100 ? 0 : 1)} ${units[unitIndex]}';
  }
}