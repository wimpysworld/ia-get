import 'dart:async';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

/// Service for managing Android notifications for downloads
class NotificationService {
  static const _platform = MethodChannel(
    'com.internetarchive.helper/notifications',
  );

  static bool _isInitialized = false;
  static const String _downloadChannelId = 'download_progress';
  static const String _completionChannelId = 'download_completion';

  /// Initialize the notification service
  static Future<void> initialize() async {
    if (_isInitialized) return;

    try {
      await _platform.invokeMethod('initialize', {
        'channels': [
          {
            'id': _downloadChannelId,
            'name': 'Download Progress',
            'description': 'Shows progress of ongoing downloads',
            'importance': 'low', // Less intrusive for progress notifications
            'showBadge': false,
          },
          {
            'id': _completionChannelId,
            'name': 'Download Complete',
            'description': 'Notifications when downloads are completed',
            'importance': 'default',
            'showBadge': true,
          },
        ],
      });

      _isInitialized = true;
    } catch (e) {
      debugPrint('Failed to initialize notification service: $e');
    }
  }

  /// Request notification permissions (Android 13+)
  static Future<bool> requestPermissions() async {
    try {
      final result = await _platform.invokeMethod('requestPermissions');
      return result == true;
    } catch (e) {
      debugPrint('Failed to request notification permissions: $e');
      return false;
    }
  }

  /// Check if notification permissions are granted
  static Future<bool> arePermissionsGranted() async {
    try {
      final result = await _platform.invokeMethod('arePermissionsGranted');
      return result == true;
    } catch (e) {
      debugPrint('Failed to check notification permissions: $e');
      return false;
    }
  }

  /// Show download progress notification
  static Future<void> showDownloadProgress({
    required String downloadId,
    required String title,
    required String description,
    required double progress,
    int? currentFile,
    int? totalFiles,
  }) async {
    if (!_isInitialized) await initialize();

    try {
      await _platform.invokeMethod('showProgressNotification', {
        'notificationId': downloadId.hashCode,
        'channelId': _downloadChannelId,
        'title': title,
        'description': description,
        'progress': (progress * 100).round(),
        'maxProgress': 100,
        'indeterminate': progress < 0,
        'ongoing': true,
        'cancelable': true,
        'downloadId': downloadId,
        'actions': [
          {'id': 'pause', 'title': 'Pause', 'icon': 'pause'},
          {'id': 'cancel', 'title': 'Cancel', 'icon': 'close'},
        ],
        'extras': {'currentFile': currentFile, 'totalFiles': totalFiles},
      });
    } catch (e) {
      debugPrint('Failed to show download progress notification: $e');
    }
  }

  /// Show paused download notification
  static Future<void> showDownloadPaused({
    required String downloadId,
    required String title,
    required String description,
    required double progress,
  }) async {
    if (!_isInitialized) await initialize();

    try {
      await _platform.invokeMethod('showProgressNotification', {
        'notificationId': downloadId.hashCode,
        'channelId': _downloadChannelId,
        'title': '$title (Paused)',
        'description': description,
        'progress': (progress * 100).round(),
        'maxProgress': 100,
        'indeterminate': false,
        'ongoing': false,
        'cancelable': true,
        'downloadId': downloadId,
        'actions': [
          {'id': 'resume', 'title': 'Resume', 'icon': 'play'},
          {'id': 'cancel', 'title': 'Cancel', 'icon': 'close'},
        ],
      });
    } catch (e) {
      debugPrint('Failed to show paused download notification: $e');
    }
  }

  /// Show download completion notification
  static Future<void> showDownloadComplete({
    required String downloadId,
    required String title,
    required String archiveName,
    required int fileCount,
    String? downloadPath,
  }) async {
    if (!_isInitialized) await initialize();

    try {
      await _platform.invokeMethod('showNotification', {
        'notificationId': downloadId.hashCode,
        'channelId': _completionChannelId,
        'title': 'Download Complete',
        'description': '$title - $fileCount files downloaded',
        'largeIcon': 'archive_icon',
        'autoCancel': true,
        'downloadId': downloadId,
        'actions': [
          {'id': 'open_folder', 'title': 'Open Folder', 'icon': 'folder_open'},
          {'id': 'share', 'title': 'Share', 'icon': 'share'},
        ],
        'extras': {
          'downloadPath': downloadPath,
          'archiveName': archiveName,
          'fileCount': fileCount,
        },
      });
    } catch (e) {
      debugPrint('Failed to show download complete notification: $e');
    }
  }

  /// Show download error notification
  static Future<void> showDownloadError({
    required String downloadId,
    required String title,
    required String errorMessage,
  }) async {
    if (!_isInitialized) await initialize();

    try {
      await _platform.invokeMethod('showNotification', {
        'notificationId': downloadId.hashCode,
        'channelId': _completionChannelId,
        'title': 'Download Failed',
        'description': '$title - $errorMessage',
        'largeIcon': 'error_icon',
        'autoCancel': true,
        'priority': 'high',
        'downloadId': downloadId,
        'actions': [
          {'id': 'retry', 'title': 'Retry', 'icon': 'refresh'},
          {'id': 'dismiss', 'title': 'Dismiss', 'icon': 'close'},
        ],
      });
    } catch (e) {
      debugPrint('Failed to show download error notification: $e');
    }
  }

  /// Cancel/dismiss a notification
  static Future<void> cancelNotification(String downloadId) async {
    try {
      await _platform.invokeMethod('cancelNotification', {
        'notificationId': downloadId.hashCode,
      });
    } catch (e) {
      debugPrint('Failed to cancel notification: $e');
    }
  }

  /// Cancel all notifications
  static Future<void> cancelAllNotifications() async {
    try {
      await _platform.invokeMethod('cancelAllNotifications');
    } catch (e) {
      debugPrint('Failed to cancel all notifications: $e');
    }
  }

  /// Show a summary notification when multiple downloads are active
  static Future<void> showDownloadSummary({
    required int activeDownloads,
    required int completedDownloads,
    required double averageProgress,
  }) async {
    if (!_isInitialized) await initialize();

    try {
      await _platform.invokeMethod('showNotification', {
        'notificationId': 'download_summary'.hashCode,
        'channelId': _downloadChannelId,
        'title': 'Downloads in Progress',
        'description': '$activeDownloads active, $completedDownloads completed',
        'progress': (averageProgress * 100).round(),
        'maxProgress': 100,
        'ongoing': true,
        'groupSummary': true,
        'actions': [
          {'id': 'pause_all', 'title': 'Pause All', 'icon': 'pause'},
          {'id': 'open_app', 'title': 'Open App', 'icon': 'app'},
        ],
      });
    } catch (e) {
      debugPrint('Failed to show download summary notification: $e');
    }
  }

  /// Update app icon badge count (if supported)
  static Future<void> updateBadgeCount(int count) async {
    try {
      await _platform.invokeMethod('updateBadgeCount', {'count': count});
    } catch (e) {
      debugPrint('Failed to update badge count: $e');
    }
  }
}
