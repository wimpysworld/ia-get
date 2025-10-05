import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'package:flutter/foundation.dart';
import 'package:http/http.dart' as http;
import 'package:crypto/crypto.dart';
import '../models/archive_metadata.dart';

/// Pure Dart/Flutter implementation of Internet Archive API client
///
/// This replaces the Rust FFI implementation with native Dart code for:
/// - Metadata fetching from archive.org JSON API
/// - File downloads with progress tracking
/// - Checksum validation
/// - Rate limiting and error handling
///
/// API Reference: https://archive.org/developers/md-read.html
class InternetArchiveApi {
  final http.Client _client;
  DateTime? _lastRequestTime;
  int _requestCount = 0;
  final DateTime _sessionStart = DateTime.now();

  // Rate limiting constants
  static const int _minRequestDelayMs = 100;
  static const int _httpTimeoutSeconds = 30;
  static const int _maxRetries = 3;
  static const int _defaultRetryDelaySecs = 30;

  InternetArchiveApi({http.Client? client})
      : _client = client ?? http.Client();

  /// Fetch metadata for an Internet Archive item
  ///
  /// [identifier] can be:
  /// - A simple identifier: "commute_test"
  /// - A details URL: "https://archive.org/details/commute_test"
  /// - A metadata URL: "https://archive.org/metadata/commute_test"
  ///
  /// Returns the parsed [ArchiveMetadata] or throws an exception on error
  Future<ArchiveMetadata> fetchMetadata(String identifier) async {
    final metadataUrl = _getMetadataUrl(identifier);
    
    if (kDebugMode) {
      print('Fetching metadata from: $metadataUrl');
    }

    // Retry logic for transient errors
    int retries = 0;
    Duration retryDelay = Duration(seconds: _defaultRetryDelaySecs);
    
    while (retries < _maxRetries) {
      try {
        await _enforceRateLimit();
        
        final response = await _client
            .get(
              Uri.parse(metadataUrl),
              headers: {
                'Accept': 'application/json, text/plain, */*',
                'Accept-Language': 'en-US,en;q=0.9',
                'Cache-Control': 'no-cache',
                'DNT': '1', // Do Not Track - be respectful
                'User-Agent': 'ia-get-flutter/1.6.0',
              },
            )
            .timeout(Duration(seconds: _httpTimeoutSeconds));

        _lastRequestTime = DateTime.now();
        _requestCount++;

        if (response.statusCode == 200) {
          final jsonData = json.decode(response.body);
          return ArchiveMetadata.fromJson(jsonData);
        } else if (response.statusCode == 429) {
          // Rate limited
          final retryAfter = int.tryParse(
                  response.headers['retry-after'] ?? '') ??
              _defaultRetryDelaySecs;
          throw Exception(
              'Rate limited by Archive.org. Please wait ${retryAfter}s before retrying.');
        } else if (response.statusCode == 404) {
          throw Exception(
              'Archive item not found: $identifier. The identifier may be incorrect.');
        } else if (response.statusCode == 403) {
          throw Exception(
              'Access forbidden. This item may be restricted or require authentication.');
        } else if (response.statusCode >= 500) {
          // Server error - retry
          if (retries < _maxRetries - 1) {
            if (kDebugMode) {
              print(
                  'Server error (${response.statusCode}), retrying in ${retryDelay.inSeconds}s...');
            }
            await Future.delayed(retryDelay);
            retries++;
            retryDelay = Duration(seconds: retryDelay.inSeconds * 2);
            continue;
          }
          throw Exception(
              'Archive.org server error (${response.statusCode}). This is likely temporary.');
        } else {
          throw Exception(
              'Failed to fetch metadata: HTTP ${response.statusCode}');
        }
      } on TimeoutException {
        if (retries < _maxRetries - 1) {
          if (kDebugMode) {
            print('Request timeout, retrying in ${retryDelay.inSeconds}s...');
          }
          await Future.delayed(retryDelay);
          retries++;
          retryDelay = Duration(seconds: retryDelay.inSeconds * 2);
          continue;
        }
        rethrow;
      } on SocketException catch (e) {
        if (retries < _maxRetries - 1) {
          if (kDebugMode) {
            print('Network error: $e, retrying in ${retryDelay.inSeconds}s...');
          }
          await Future.delayed(retryDelay);
          retries++;
          retryDelay = Duration(seconds: retryDelay.inSeconds * 2);
          continue;
        }
        rethrow;
      }
    }

    throw Exception('Failed to fetch metadata after $_maxRetries attempts');
  }

  /// Download a file from the Internet Archive
  ///
  /// [url] - Full download URL (typically constructed from metadata)
  /// [outputPath] - Local file path to save the download
  /// [onProgress] - Optional callback for progress updates (downloaded, total)
  ///
  /// Returns the downloaded file path on success
  Future<String> downloadFile(
    String url,
    String outputPath, {
    void Function(int downloaded, int total)? onProgress,
    CancellationToken? cancellationToken,
  }) async {
    if (kDebugMode) {
      print('Downloading from: $url');
      print('Saving to: $outputPath');
    }

    final request = http.Request('GET', Uri.parse(url));
    request.headers.addAll({
      'Accept': '*/*',
      'User-Agent': 'ia-get-flutter/1.6.0',
    });

    final response = await _client.send(request);

    if (response.statusCode != 200) {
      throw Exception(
          'Failed to download file: HTTP ${response.statusCode}');
    }

    final contentLength = response.contentLength ?? 0;
    int downloaded = 0;

    final outputFile = File(outputPath);
    await outputFile.parent.create(recursive: true);
    
    final sink = outputFile.openWrite();

    try {
      await for (final chunk in response.stream) {
        if (cancellationToken?.isCancelled ?? false) {
          throw Exception('Download cancelled by user');
        }

        sink.add(chunk);
        downloaded += chunk.length;

        onProgress?.call(downloaded, contentLength);
      }

      await sink.flush();
      await sink.close();

      if (kDebugMode) {
        print('Download complete: $outputPath');
      }

      return outputPath;
    } catch (e) {
      await sink.close();
      // Clean up partial download
      if (await outputFile.exists()) {
        await outputFile.delete();
      }
      rethrow;
    }
  }

  /// Validate file checksum
  ///
  /// [filePath] - Path to the file to validate
  /// [expectedHash] - Expected hash value (hex string)
  /// [hashType] - Hash algorithm: 'md5', 'sha1', or 'sha256'
  ///
  /// Returns true if hash matches, false otherwise
  Future<bool> validateChecksum(
    String filePath,
    String expectedHash,
    String hashType,
  ) async {
    final file = File(filePath);
    if (!await file.exists()) {
      throw Exception('File not found: $filePath');
    }

    if (kDebugMode) {
      print('Validating $hashType checksum for: $filePath');
    }

    final bytes = await file.readAsBytes();
    Digest digest;

    switch (hashType.toLowerCase()) {
      case 'md5':
        digest = md5.convert(bytes);
        break;
      case 'sha1':
        digest = sha1.convert(bytes);
        break;
      case 'sha256':
        digest = sha256.convert(bytes);
        break;
      default:
        throw Exception('Unsupported hash type: $hashType');
    }

    final actualHash = digest.toString();
    final matches = actualHash.toLowerCase() == expectedHash.toLowerCase();

    if (kDebugMode) {
      print('Expected: $expectedHash');
      print('Actual:   $actualHash');
      print('Match: $matches');
    }

    return matches;
  }

  /// Convert various input formats to metadata URL
  ///
  /// Handles:
  /// - Details URL: https://archive.org/details/identifier
  /// - Metadata URL: https://archive.org/metadata/identifier
  /// - Simple identifier: identifier
  String _getMetadataUrl(String input) {
    final trimmed = input.trim();

    if (trimmed.contains('/details/')) {
      return trimmed.replaceAll('/details/', '/metadata/');
    } else if (trimmed.contains('://archive.org/metadata/')) {
      return trimmed;
    } else if (trimmed.startsWith('http://') || trimmed.startsWith('https://')) {
      // It's a URL but not a details or metadata URL - extract identifier
      final uri = Uri.parse(trimmed);
      final segments = uri.pathSegments;
      if (segments.isNotEmpty) {
        final identifier = segments.last;
        return 'https://archive.org/metadata/$identifier';
      }
      throw Exception('Cannot extract identifier from URL: $trimmed');
    } else {
      // Assume it's a bare identifier
      return 'https://archive.org/metadata/$trimmed';
    }
  }

  /// Enforce rate limiting between requests
  Future<void> _enforceRateLimit() async {
    if (_lastRequestTime != null) {
      final elapsed = DateTime.now().difference(_lastRequestTime!);
      final minDelay = Duration(milliseconds: _minRequestDelayMs);

      if (elapsed < minDelay) {
        final waitTime = minDelay - elapsed;
        await Future.delayed(waitTime);
      }
    }
  }

  /// Get API usage statistics
  Map<String, dynamic> getStats() {
    final sessionDuration = DateTime.now().difference(_sessionStart);
    final minutes = sessionDuration.inSeconds / 60.0;
    final requestsPerMinute = minutes > 0 ? _requestCount / minutes : 0.0;

    return {
      'requestCount': _requestCount,
      'sessionDuration': sessionDuration.toString(),
      'requestsPerMinute': requestsPerMinute.toStringAsFixed(1),
    };
  }

  /// Check if request rate is healthy (under 30 requests/minute)
  bool isRateHealthy() {
    final sessionDuration = DateTime.now().difference(_sessionStart);
    final minutes = sessionDuration.inSeconds / 60.0;
    if (minutes <= 0) return true;
    
    final requestsPerMinute = _requestCount / minutes;
    return requestsPerMinute < 30.0;
  }

  /// Close the HTTP client
  void dispose() {
    _client.close();
  }
}

/// Simple cancellation token for downloads
class CancellationToken {
  bool _isCancelled = false;

  bool get isCancelled => _isCancelled;

  void cancel() {
    _isCancelled = true;
  }
}
