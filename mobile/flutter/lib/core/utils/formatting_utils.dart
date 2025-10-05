/// Formatting utilities for displaying data in human-readable formats
library;

import 'dart:math';

/// Utility class for formatting various data types into human-readable strings
class FormattingUtils {
  static const List<String> _byteSuffixes = ['B', 'KB', 'MB', 'GB', 'TB', 'PB'];
  
  /// Format bytes into human-readable string
  /// 
  /// Examples:
  /// - 1024 → "1.0 KB"
  /// - 1536 → "1.5 KB"
  /// - 1048576 → "1 MB"
  /// - 0 → "0 B"
  static String formatBytes(int bytes) {
    if (bytes <= 0) return '0 B';
    if (bytes < 1024) return '$bytes B';

    final i = (log(bytes) / log(1024)).floor().clamp(0, _byteSuffixes.length - 1);
    final size = bytes / pow(1024, i);

    // Show 1 decimal place for values < 10, otherwise round to whole number
    return '${size.toStringAsFixed(size < 10 ? 1 : 0)} ${_byteSuffixes[i]}';
  }

  /// Format transfer speed (bytes per second) into human-readable string
  ///
  /// Examples:
  /// - 1024 → "1.0 KB/s"
  /// - 1048576 → "1 MB/s"
  static String formatSpeed(double bytesPerSecond) {
    if (bytesPerSecond <= 0) return '0 B/s';
    if (bytesPerSecond < 1024) return '${bytesPerSecond.toInt()} B/s';

    final i = (log(bytesPerSecond) / log(1024)).floor().clamp(0, _byteSuffixes.length - 1);
    final speed = bytesPerSecond / pow(1024, i);

    return '${speed.toStringAsFixed(speed < 10 ? 1 : 0)} ${_byteSuffixes[i]}/s';
  }

  /// Format duration into human-readable string
  ///
  /// Examples:
  /// - Duration(seconds: 45) → "45s"
  /// - Duration(minutes: 2, seconds: 30) → "2m 30s"
  /// - Duration(hours: 1, minutes: 15, seconds: 5) → "1h 15m 5s"
  /// - Duration(days: 1, hours: 2) → "1d 2h 0m"
  static String formatDuration(Duration duration) {
    if (duration.isNegative) return '0s';

    final days = duration.inDays;
    final hours = duration.inHours.remainder(24);
    final minutes = duration.inMinutes.remainder(60);
    final seconds = duration.inSeconds.remainder(60);

    if (days > 0) {
      return '${days}d ${hours}h ${minutes}m';
    } else if (hours > 0) {
      return '${hours}h ${minutes}m ${seconds}s';
    } else if (minutes > 0) {
      return '${minutes}m ${seconds}s';
    } else {
      return '${seconds}s';
    }
  }

  /// Format duration in a more compact form (for display in tight spaces)
  ///
  /// Examples:
  /// - Duration(seconds: 45) → "45s"
  /// - Duration(minutes: 2, seconds: 30) → "2:30"
  /// - Duration(hours: 1, minutes: 15, seconds: 5) → "1:15:05"
  static String formatDurationCompact(Duration duration) {
    if (duration.isNegative) return '0s';

    final hours = duration.inHours;
    final minutes = duration.inMinutes.remainder(60);
    final seconds = duration.inSeconds.remainder(60);

    if (hours > 0) {
      return '$hours:${minutes.toString().padLeft(2, '0')}:${seconds.toString().padLeft(2, '0')}';
    } else if (minutes > 0) {
      return '$minutes:${seconds.toString().padLeft(2, '0')}';
    } else {
      return '${seconds}s';
    }
  }

  /// Estimate remaining time based on current progress and speed
  ///
  /// Returns "Unknown" if calculation is not possible
  static String formatEstimatedTime(
    double progress,
    double bytesPerSecond,
    int totalBytes,
  ) {
    if (progress <= 0 || progress >= 1.0 || bytesPerSecond <= 0) {
      return 'Unknown';
    }

    final remainingBytes = totalBytes * (1 - progress);
    final remainingSeconds = remainingBytes / bytesPerSecond;

    if (remainingSeconds.isInfinite || remainingSeconds.isNaN) {
      return 'Unknown';
    }

    return formatDuration(Duration(seconds: remainingSeconds.ceil()));
  }

  /// Format a percentage value
  ///
  /// Examples:
  /// - 0.5 → "50%"
  /// - 0.657 → "65.7%"
  /// - 1.0 → "100%"
  static String formatPercentage(double value, {int decimals = 1}) {
    if (value.isNaN || value.isInfinite) return '0%';
    final percentage = (value * 100).clamp(0.0, 100.0);
    return '${percentage.toStringAsFixed(decimals)}%';
  }

  /// Format a count number with thousand separators
  ///
  /// Examples:
  /// - 1000 → "1,000"
  /// - 1234567 → "1,234,567"
  static String formatCount(int count) {
    return count.toString().replaceAllMapped(
          RegExp(r'(\d{1,3})(?=(\d{3})+(?!\d))'),
          (Match m) => '${m[1]},',
        );
  }

  /// Format a date/time in relative format (e.g., "2 hours ago")
  static String formatRelativeTime(DateTime dateTime) {
    final now = DateTime.now();
    final difference = now.difference(dateTime);

    if (difference.isNegative) {
      return 'in the future';
    }

    if (difference.inSeconds < 60) {
      return 'just now';
    } else if (difference.inMinutes < 60) {
      final minutes = difference.inMinutes;
      return '$minutes ${minutes == 1 ? 'minute' : 'minutes'} ago';
    } else if (difference.inHours < 24) {
      final hours = difference.inHours;
      return '$hours ${hours == 1 ? 'hour' : 'hours'} ago';
    } else if (difference.inDays < 7) {
      final days = difference.inDays;
      return '$days ${days == 1 ? 'day' : 'days'} ago';
    } else if (difference.inDays < 30) {
      final weeks = (difference.inDays / 7).floor();
      return '$weeks ${weeks == 1 ? 'week' : 'weeks'} ago';
    } else if (difference.inDays < 365) {
      final months = (difference.inDays / 30).floor();
      return '$months ${months == 1 ? 'month' : 'months'} ago';
    } else {
      final years = (difference.inDays / 365).floor();
      return '$years ${years == 1 ? 'year' : 'years'} ago';
    }
  }

  /// Format a date in a standard format
  ///
  /// Example: "Jan 15, 2024 3:45 PM"
  static String formatDate(DateTime dateTime) {
    const months = [
      'Jan',
      'Feb',
      'Mar',
      'Apr',
      'May',
      'Jun',
      'Jul',
      'Aug',
      'Sep',
      'Oct',
      'Nov',
      'Dec'
    ];

    final month = months[dateTime.month - 1];
    final day = dateTime.day;
    final year = dateTime.year;
    final hour = dateTime.hour > 12 ? dateTime.hour - 12 : (dateTime.hour == 0 ? 12 : dateTime.hour);
    final minute = dateTime.minute.toString().padLeft(2, '0');
    final period = dateTime.hour >= 12 ? 'PM' : 'AM';

    return '$month $day, $year $hour:$minute $period';
  }

  /// Format a date in a compact format
  ///
  /// Example: "01/15/24"
  static String formatDateCompact(DateTime dateTime) {
    final month = dateTime.month.toString().padLeft(2, '0');
    final day = dateTime.day.toString().padLeft(2, '0');
    final year = dateTime.year.toString().substring(2);

    return '$month/$day/$year';
  }

  /// Truncate a string with ellipsis if it exceeds max length
  static String truncate(String text, int maxLength, {String ellipsis = '...'}) {
    if (text.length <= maxLength) return text;
    return '${text.substring(0, maxLength - ellipsis.length)}$ellipsis';
  }

  /// Format a file size range
  ///
  /// Example: "10 MB - 50 MB"
  static String formatSizeRange(int minBytes, int maxBytes) {
    return '${formatBytes(minBytes)} - ${formatBytes(maxBytes)}';
  }

  /// Format download statistics summary
  static String formatDownloadSummary({
    required int totalDownloads,
    required int totalBytes,
    required Duration totalDuration,
  }) {
    final avgSpeed = totalDuration.inSeconds > 0
        ? totalBytes / totalDuration.inSeconds
        : 0.0;

    return '${formatCount(totalDownloads)} downloads, '
        '${formatBytes(totalBytes)} total, '
        'avg ${formatSpeed(avgSpeed)}';
  }
}
