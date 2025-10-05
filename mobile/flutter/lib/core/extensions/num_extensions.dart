/// Numeric extensions for enhanced functionality
extension NumExtensions on num {
  /// Converts bytes to human-readable file size
  String toFileSize() {
    const units = ['B', 'KB', 'MB', 'GB', 'TB'];
    var size = toDouble();
    var unitIndex = 0;

    while (size >= 1024 && unitIndex < units.length - 1) {
      size /= 1024;
      unitIndex++;
    }

    return '${size.toStringAsFixed(size < 10 ? 1 : 0)} ${units[unitIndex]}';
  }

  /// Converts to percentage string
  String toPercentage([int decimals = 0]) {
    return '${toStringAsFixed(decimals)}%';
  }

  /// Converts seconds to Duration
  Duration toDuration() {
    return Duration(seconds: toInt());
  }

  /// Checks if number is in range
  bool inRange(num min, num max) {
    return this >= min && this <= max;
  }

  /// Clamps value between min and max
  num clampValue(num min, num max) {
    if (this < min) return min;
    if (this > max) return max;
    return this;
  }
}
