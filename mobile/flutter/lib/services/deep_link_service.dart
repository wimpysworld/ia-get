import 'dart:async';
import 'package:app_links/app_links.dart';
import 'package:flutter/foundation.dart';

/// Service for handling deep links to Internet Archive content
class DeepLinkService {
  final _appLinks = AppLinks();
  StreamSubscription<Uri>? _linkSubscription;

  /// Callback when a valid archive link is received
  Function(String identifier)? onArchiveLinkReceived;

  /// Initialize deep link listening
  Future<void> initialize() async {
    try {
      // Add timeout to prevent hanging on initial link retrieval
      final initialUri = await _appLinks.getInitialLink().timeout(
        const Duration(seconds: 3),
        onTimeout: () {
          if (kDebugMode) {
            print('Deep link: getInitialLink timed out');
          }
          return null;
        },
      );
      
      if (initialUri != null) {
        _handleDeepLink(initialUri);
      }

      // Listen for links while app is running
      _linkSubscription = _appLinks.uriLinkStream.listen(
        (uri) {
          _handleDeepLink(uri);
        },
        onError: (err) {
          if (kDebugMode) {
            print('Deep link error: $err');
          }
        },
        cancelOnError: false, // Continue listening even after errors
      );

      if (kDebugMode) {
        print('Deep link service initialized');
      }
    } catch (e) {
      if (kDebugMode) {
        print('Failed to initialize deep link service: $e');
      }
      // Non-critical error - app can continue without deep linking
    }
  }

  /// Handle incoming deep link
  void _handleDeepLink(Uri uri) {
    if (kDebugMode) {
      print('Received deep link: $uri');
    }

    try {
      final identifier = _extractArchiveIdentifier(uri);
      if (identifier != null && identifier.isNotEmpty) {
        if (onArchiveLinkReceived != null) {
          onArchiveLinkReceived!(identifier);

          if (kDebugMode) {
            print('Extracted archive identifier: $identifier');
          }
        } else {
          if (kDebugMode) {
            print('Deep link handler not set for identifier: $identifier');
          }
        }
      } else {
        if (kDebugMode) {
          print('Could not extract archive identifier from: $uri');
        }
      }
    } catch (e) {
      if (kDebugMode) {
        print('Error handling deep link: $e');
      }
    }
  }

  /// Extract archive identifier from various URL formats
  String? _extractArchiveIdentifier(Uri uri) {
    // Handle https://archive.org/details/[identifier]
    if (uri.host == 'archive.org' && uri.pathSegments.isNotEmpty) {
      // Path format: /details/identifier
      if (uri.pathSegments.length >= 2 && uri.pathSegments[0] == 'details') {
        return uri.pathSegments[1];
      }

      // Path format: /download/identifier
      if (uri.pathSegments.length >= 2 && uri.pathSegments[0] == 'download') {
        return uri.pathSegments[1];
      }

      // Path format: /metadata/identifier
      if (uri.pathSegments.length >= 2 && uri.pathSegments[0] == 'metadata') {
        return uri.pathSegments[1];
      }
    }

    // Handle custom scheme: iaget://identifier
    if (uri.scheme == 'iaget') {
      // Format: iaget://identifier or iaget:identifier
      if (uri.host.isNotEmpty) {
        return uri.host;
      } else if (uri.path.isNotEmpty) {
        // Remove leading slash if present
        return uri.path.replaceFirst('/', '');
      }
    }

    return null;
  }

  /// Dispose resources
  void dispose() {
    _linkSubscription?.cancel();
    _linkSubscription = null;
  }
}
