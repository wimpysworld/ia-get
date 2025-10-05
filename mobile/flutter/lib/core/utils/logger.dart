/// Logging utility for consistent debug logging across the app
library;

import 'package:flutter/foundation.dart';

/// Log levels for categorizing log messages
enum LogLevel {
  debug,
  info,
  warning,
  error,
}

/// A simple logger utility for consistent logging across the app
class Logger {
  final String _tag;
  static bool _enabled = kDebugMode;

  Logger(this._tag);

  /// Enable or disable logging globally
  static void setEnabled(bool enabled) {
    _enabled = enabled;
  }

  /// Log a debug message
  void debug(String message, [dynamic data]) {
    _log(LogLevel.debug, message, data);
  }

  /// Log an info message
  void info(String message, [dynamic data]) {
    _log(LogLevel.info, message, data);
  }

  /// Log a warning message
  void warning(String message, [dynamic data]) {
    _log(LogLevel.warning, message, data);
  }

  /// Log an error message
  void error(String message, [dynamic error, StackTrace? stackTrace]) {
    _log(LogLevel.error, message, error);
    if (stackTrace != null && _enabled) {
      debugPrint('Stack trace: $stackTrace');
    }
  }

  void _log(LogLevel level, String message, [dynamic data]) {
    if (!_enabled) return;

    final timestamp = DateTime.now().toIso8601String();
    final levelStr = level.name.toUpperCase().padRight(7);
    final tagStr = _tag.padRight(20);

    var logMessage = '[$timestamp] [$levelStr] [$tagStr] $message';

    if (data != null) {
      logMessage += '\n  Data: $data';
    }

    // Use debugPrint to avoid being truncated in long messages
    debugPrint(logMessage);
  }

  /// Create a logger with a specific tag
  static Logger tag(String tag) {
    return Logger(tag);
  }
}

/// Convenience function to create a logger
Logger getLogger(String tag) {
  return Logger(tag);
}
