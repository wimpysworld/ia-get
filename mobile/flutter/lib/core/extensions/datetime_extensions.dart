/// DateTime extensions for enhanced functionality
extension DateTimeExtensions on DateTime {
  /// Checks if date is today
  bool isToday() {
    final now = DateTime.now();
    return year == now.year && month == now.month && day == now.day;
  }

  /// Checks if date is yesterday
  bool isYesterday() {
    final yesterday = DateTime.now().subtract(const Duration(days: 1));
    return year == yesterday.year &&
        month == yesterday.month &&
        day == yesterday.day;
  }

  /// Checks if date is tomorrow
  bool isTomorrow() {
    final tomorrow = DateTime.now().add(const Duration(days: 1));
    return year == tomorrow.year &&
        month == tomorrow.month &&
        day == tomorrow.day;
  }

  /// Returns relative time string (e.g., "2 hours ago")
  String timeAgo({bool short = false}) {
    final now = DateTime.now();
    final difference = now.difference(this);

    if (difference.inSeconds < 60) {
      return short ? 'now' : 'just now';
    } else if (difference.inMinutes < 60) {
      final minutes = difference.inMinutes;
      return short ? '${minutes}m' : '$minutes minute${minutes == 1 ? '' : 's'} ago';
    } else if (difference.inHours < 24) {
      final hours = difference.inHours;
      return short ? '${hours}h' : '$hours hour${hours == 1 ? '' : 's'} ago';
    } else if (difference.inDays < 7) {
      final days = difference.inDays;
      return short ? '${days}d' : '$days day${days == 1 ? '' : 's'} ago';
    } else if (difference.inDays < 30) {
      final weeks = (difference.inDays / 7).floor();
      return short ? '${weeks}w' : '$weeks week${weeks == 1 ? '' : 's'} ago';
    } else if (difference.inDays < 365) {
      final months = (difference.inDays / 30).floor();
      return short ? '${months}mo' : '$months month${months == 1 ? '' : 's'} ago';
    } else {
      final years = (difference.inDays / 365).floor();
      return short ? '${years}y' : '$years year${years == 1 ? '' : 's'} ago';
    }
  }

  /// Formats date with custom pattern
  String formatted(String pattern) {
    return pattern
        .replaceAll('yyyy', year.toString())
        .replaceAll('MM', month.toString().padLeft(2, '0'))
        .replaceAll('dd', day.toString().padLeft(2, '0'))
        .replaceAll('HH', hour.toString().padLeft(2, '0'))
        .replaceAll('mm', minute.toString().padLeft(2, '0'))
        .replaceAll('ss', second.toString().padLeft(2, '0'));
  }

  /// Returns start of day (00:00:00)
  DateTime startOfDay() {
    return DateTime(year, month, day);
  }

  /// Returns end of day (23:59:59)
  DateTime endOfDay() {
    return DateTime(year, month, day, 23, 59, 59, 999);
  }

  /// Adds business days (skipping weekends)
  DateTime addBusinessDays(int days) {
    var result = this;
    var remaining = days.abs();
    final increment = days > 0 ? 1 : -1;

    while (remaining > 0) {
      result = result.add(Duration(days: increment));
      if (result.weekday != DateTime.saturday &&
          result.weekday != DateTime.sunday) {
        remaining--;
      }
    }
    return result;
  }

  /// Checks if date is weekend
  bool isWeekend() {
    return weekday == DateTime.saturday || weekday == DateTime.sunday;
  }

  /// Checks if date is same day as other
  bool isSameDay(DateTime other) {
    return year == other.year && month == other.month && day == other.day;
  }
}
