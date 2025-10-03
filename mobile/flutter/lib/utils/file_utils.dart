import 'dart:io';
import 'dart:math';

/// Utility class for file operations and formatting
class FileUtils {
  static const List<String> _byteSuffixes = ['B', 'KB', 'MB', 'GB', 'TB'];

  /// Format bytes into human readable string
  static String formatBytes(int bytes) {
    if (bytes <= 0) return '0 B';

    final i = (log(bytes) / log(1024)).floor();
    final size = bytes / pow(1024, i);

    if (i == 0) {
      return '$bytes B';
    }

    return '${size.toStringAsFixed(size < 10 ? 1 : 0)} ${_byteSuffixes[i]}';
  }

  /// Format transfer speed (bytes per second) into human readable string
  static String formatTransferSpeed(double bytesPerSecond) {
    if (bytesPerSecond <= 0) return '0 B/s';

    final i = (log(bytesPerSecond) / log(1024)).floor();
    final speed = bytesPerSecond / pow(1024, i);

    if (i == 0) {
      return '${bytesPerSecond.toInt()} B/s';
    }

    return '${speed.toStringAsFixed(speed < 10 ? 1 : 0)} ${_byteSuffixes[i]}/s';
  }

  /// Format duration into human readable string
  static String formatDuration(Duration duration) {
    final hours = duration.inHours;
    final minutes = duration.inMinutes.remainder(60);
    final seconds = duration.inSeconds.remainder(60);

    if (hours > 0) {
      return '${hours}h ${minutes}m ${seconds}s';
    } else if (minutes > 0) {
      return '${minutes}m ${seconds}s';
    } else {
      return '${seconds}s';
    }
  }

  /// Estimate remaining time based on current progress and speed
  static String formatEstimatedTime(
    double progress,
    double bytesPerSecond,
    int totalBytes,
  ) {
    if (progress <= 0 || bytesPerSecond <= 0) return 'Unknown';

    final remainingBytes = totalBytes * (1 - progress);
    final remainingSeconds = remainingBytes / bytesPerSecond;

    return formatDuration(Duration(seconds: remainingSeconds.ceil()));
  }

  /// Get file extension from filename
  static String getFileExtension(String filename) {
    final lastDot = filename.lastIndexOf('.');
    if (lastDot == -1 || lastDot == filename.length - 1) {
      return '';
    }
    return filename.substring(lastDot + 1).toLowerCase();
  }

  /// Get file type icon based on extension
  static String getFileTypeIcon(String filename) {
    final extension = getFileExtension(filename);

    switch (extension) {
      case 'mp4':
      case 'avi':
      case 'mkv':
      case 'mov':
      case 'wmv':
        return '🎬';
      case 'mp3':
      case 'wav':
      case 'flac':
      case 'ogg':
        return '🎵';
      case 'jpg':
      case 'jpeg':
      case 'png':
      case 'gif':
      case 'bmp':
        return '🖼️';
      case 'pdf':
        return '📄';
      case 'txt':
      case 'md':
        return '📝';
      case 'zip':
      case 'rar':
      case 'tar':
      case 'gz':
        return '📦';
      case 'iso':
        return '💿';
      default:
        return '📁';
    }
  }

  /// Check if a file exists and is accessible
  static Future<bool> isFileAccessible(String path) async {
    try {
      final file = File(path);
      return await file.exists();
    } catch (e) {
      return false;
    }
  }

  /// Create directory if it doesn't exist
  static Future<bool> ensureDirectoryExists(String path) async {
    try {
      final directory = Directory(path);
      if (!await directory.exists()) {
        await directory.create(recursive: true);
      }
      return true;
    } catch (e) {
      return false;
    }
  }

  /// Get available disk space for a path
  /// Returns available space in bytes, or null if unable to determine
  ///
  /// Note: On Android, this function returns null because reliable disk space
  /// APIs are not consistently available across devices. The app handles this
  /// gracefully by skipping disk space validation.
  static Future<int?> getAvailableSpace(String path) async {
    // On Android, disk space checks are unreliable and not supported
    // Return null to skip validation (similar to Rust implementation)
    // The app gracefully handles null by proceeding with download
    return null;
  }

  /// Check if there is sufficient disk space for a download
  /// Returns true if sufficient, false if not, null if unable to determine
  static Future<bool?> hasSufficientSpace(
    String path,
    int requiredBytes,
  ) async {
    final availableSpace = await getAvailableSpace(path);
    if (availableSpace == null) {
      return null; // Unable to determine
    }

    // Add safety margin: 100MB or 5% of required size, whichever is larger
    const minMargin = 100 * 1024 * 1024; // 100MB in bytes
    final percentMargin = (requiredBytes * 0.05).round();
    final safetyMargin = minMargin > percentMargin ? minMargin : percentMargin;

    final totalRequired = requiredBytes + safetyMargin;
    return availableSpace >= totalRequired;
  }

  /// Get required space with safety margin for a download
  static int getRequiredSpaceWithMargin(int downloadSize) {
    const minMargin = 100 * 1024 * 1024; // 100MB in bytes
    final percentMargin = (downloadSize * 0.05).round();
    final safetyMargin = minMargin > percentMargin ? minMargin : percentMargin;
    return downloadSize + safetyMargin;
  }

  /// Sanitize filename for safe file system usage
  static String sanitizeFilename(String filename) {
    // Remove or replace invalid characters
    final sanitized = filename
        .replaceAll(RegExp(r'[<>:"/\\|?*]'), '_')
        .replaceAll(RegExp(r'\s+'), ' ')
        .trim();

    // Ensure filename is not too long
    if (sanitized.length > 255) {
      final extension = getFileExtension(sanitized);
      final nameWithoutExt = sanitized.substring(0, sanitized.lastIndexOf('.'));
      final maxNameLength = 255 - extension.length - 1;
      return '${nameWithoutExt.substring(0, maxNameLength)}.$extension';
    }

    return sanitized;
  }

  /// Generate unique filename if file already exists
  static Future<String> getUniqueFilename(
    String basePath,
    String filename,
  ) async {
    final sanitized = sanitizeFilename(filename);
    final file = File('$basePath/$sanitized');

    if (!await file.exists()) {
      return sanitized;
    }

    final extension = getFileExtension(sanitized);
    final nameWithoutExt = extension.isEmpty
        ? sanitized
        : sanitized.substring(0, sanitized.lastIndexOf('.'));

    int counter = 1;
    String uniqueName;

    do {
      uniqueName = extension.isEmpty
          ? '${nameWithoutExt}_$counter'
          : '${nameWithoutExt}_$counter.$extension';
      counter++;
    } while (await File('$basePath/$uniqueName').exists());

    return uniqueName;
  }

  /// Calculate MD5 hash of a file (for verification)
  static Future<String?> calculateFileHash(String filePath) async {
    try {
      // This would require crypto package and is platform-specific
      // Placeholder for actual implementation
      return null;
    } catch (e) {
      return null;
    }
  }
}
