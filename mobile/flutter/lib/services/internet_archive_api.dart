import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'package:flutter/foundation.dart';
import 'package:http/http.dart' as http;
import 'package:crypto/crypto.dart';
import 'package:archive/archive.dart';
import '../models/archive_metadata.dart';
import '../core/constants/internet_archive_constants.dart';
import '../core/errors/ia_exceptions.dart';

/// Pure Dart/Flutter implementation of Internet Archive API client
///
/// This replaces the Rust FFI implementation with native Dart code for:
/// - Metadata fetching from archive.org JSON API
/// - File downloads with progress tracking
/// - Checksum validation
/// - Rate limiting and error handling
///
/// API Reference: https://archive.org/developers/md-read.html
/// 
/// Compliance:
/// - Respects rate limits (max 30 requests/minute)
/// - Includes proper User-Agent header with contact info
/// - Implements exponential backoff for retries
/// - Handles all IA-specific HTTP status codes
class InternetArchiveApi {
  final http.Client _client;
  DateTime? _lastRequestTime;
  int _requestCount = 0;
  final DateTime _sessionStart = DateTime.now();
  
  /// App version for User-Agent header
  static const String _appVersion = '1.6.0';

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
    Duration retryDelay = const Duration(seconds: IARateLimits.defaultRetryDelaySecs);
    
    while (retries < IARateLimits.maxRetries) {
      try {
        await _enforceRateLimit();
        
        final response = await _client
            .get(
              Uri.parse(metadataUrl),
              headers: IAHeaders.standard(_appVersion),
            )
            .timeout(const Duration(seconds: IAHttpConfig.timeoutSeconds));

        _lastRequestTime = DateTime.now();
        _requestCount++;

        if (response.statusCode == 200) {
          final jsonData = json.decode(response.body);
          return ArchiveMetadata.fromJson(jsonData);
        } else if (response.statusCode == 429) {
          // Rate limited - respect Retry-After header and retry
          final retryAfter = int.tryParse(
                  response.headers['retry-after'] ?? '') ??
              IARateLimits.defaultRetryDelaySecs;
          
          if (retries < IARateLimits.maxRetries - 1) {
            if (kDebugMode) {
              print('Rate limited. Waiting ${retryAfter}s before retry (as requested by server)...');
            }
            // Wait the exact time the server told us to wait
            await Future.delayed(Duration(seconds: retryAfter));
            retries++;
            // Reset retry delay for next iteration if needed
            retryDelay = const Duration(seconds: IARateLimits.defaultRetryDelaySecs);
            continue;
          }
          // If we've exhausted retries, throw with the information
          throw RateLimitException(retryAfter);
        } else if (response.statusCode == 404) {
          throw Exception(IAErrorMessages.notFound);
        } else if (response.statusCode == 403) {
          throw Exception(IAErrorMessages.forbidden);
        } else if (response.statusCode >= 500) {
          // Server error - check for Retry-After header first, then use exponential backoff
          if (retries < IARateLimits.maxRetries - 1) {
            // Check if server provided a Retry-After header
            final serverRetryAfter = int.tryParse(response.headers['retry-after'] ?? '');
            final waitTime = serverRetryAfter != null 
                ? Duration(seconds: serverRetryAfter)
                : retryDelay;
            
            if (kDebugMode) {
              if (serverRetryAfter != null) {
                print('Server error (${response.statusCode}), retrying in ${waitTime.inSeconds}s (as requested by server)...');
              } else {
                print('Server error (${response.statusCode}), retrying in ${waitTime.inSeconds}s (exponential backoff)...');
              }
            }
            
            await Future.delayed(waitTime);
            retries++;
            
            // Only apply exponential backoff if server didn't specify retry-after
            if (serverRetryAfter == null) {
              retryDelay = Duration(
                  seconds: (retryDelay.inSeconds * IARateLimits.backoffMultiplier).toInt());
              // Cap at max backoff delay
              if (retryDelay.inSeconds > IARateLimits.maxBackoffDelaySecs) {
                retryDelay = const Duration(seconds: IARateLimits.maxBackoffDelaySecs);
              }
            }
            continue;
          }
          throw Exception('${IAErrorMessages.serverError} (${response.statusCode}). This is likely temporary.');
        } else {
          throw Exception(
              'Failed to fetch metadata: HTTP ${response.statusCode}');
        }
      } on TimeoutException {
        if (retries < IARateLimits.maxRetries - 1) {
          if (kDebugMode) {
            print('Request timeout, retrying in ${retryDelay.inSeconds}s...');
          }
          await Future.delayed(retryDelay);
          retries++;
          retryDelay = Duration(
              seconds: (retryDelay.inSeconds * IARateLimits.backoffMultiplier).toInt());
          continue;
        }
        rethrow;
      } on SocketException catch (e) {
        if (retries < IARateLimits.maxRetries - 1) {
          if (kDebugMode) {
            print('Network error: $e, retrying in ${retryDelay.inSeconds}s...');
          }
          await Future.delayed(retryDelay);
          retries++;
          retryDelay = Duration(
              seconds: (retryDelay.inSeconds * IARateLimits.backoffMultiplier).toInt());
          continue;
        }
        rethrow;
      }
    }

    throw Exception('Failed to fetch metadata after ${IARateLimits.maxRetries} attempts');
  }

  /// Download a file from a URL with progress tracking
  ///
  /// [url] - Full URL to the file
  /// [outputPath] - Local path where file should be saved
  /// [onProgress] - Optional callback for progress updates (downloaded, total)
  /// [cancellationToken] - Optional token to cancel the download
  ///
  /// Returns the path to the downloaded file
  /// Throws exception on failure
  ///
  /// Automatically retries on transient errors and respects server Retry-After headers
  Future<String> downloadFile(
    String url,
    String outputPath, {
    void Function(int downloaded, int total)? onProgress,
    CancellationToken? cancellationToken,
  }) async {
    int retries = 0;
    Duration retryDelay = const Duration(seconds: IARateLimits.defaultRetryDelaySecs);
    
    while (retries < IARateLimits.maxRetries) {
      try {
        if (kDebugMode) {
          print('Downloading from: $url (attempt ${retries + 1})');
          print('Saving to: $outputPath');
        }

        final request = http.Request('GET', Uri.parse(url));
        request.headers.addAll(IAHeaders.standard(_appVersion));

        final response = await _client.send(request);

        // Handle rate limiting and errors with retry-after support
        if (response.statusCode == 429) {
          // Rate limited - respect Retry-After header
          final retryAfter = int.tryParse(
                  response.headers['retry-after'] ?? '') ??
              IARateLimits.defaultRetryDelaySecs;
          
          if (retries < IARateLimits.maxRetries - 1) {
            if (kDebugMode) {
              print('Download rate limited. Waiting ${retryAfter}s before retry (as requested by server)...');
            }
            await response.stream.drain(); // Drain the stream
            await Future.delayed(Duration(seconds: retryAfter));
            retries++;
            continue;
          }
          await response.stream.drain();
          throw RateLimitException(retryAfter);
        } else if (response.statusCode >= 500) {
          // Server error - check for Retry-After header
          if (retries < IARateLimits.maxRetries - 1) {
            final serverRetryAfter = int.tryParse(response.headers['retry-after'] ?? '');
            final waitTime = serverRetryAfter != null 
                ? Duration(seconds: serverRetryAfter)
                : retryDelay;
            
            if (kDebugMode) {
              if (serverRetryAfter != null) {
                print('Download failed (${response.statusCode}), retrying in ${waitTime.inSeconds}s (as requested by server)...');
              } else {
                print('Download failed (${response.statusCode}), retrying in ${waitTime.inSeconds}s (exponential backoff)...');
              }
            }
            
            await response.stream.drain(); // Drain the stream
            await Future.delayed(waitTime);
            retries++;
            
            // Only apply exponential backoff if server didn't specify retry-after
            if (serverRetryAfter == null) {
              retryDelay = Duration(
                  seconds: (retryDelay.inSeconds * IARateLimits.backoffMultiplier).toInt());
              if (retryDelay.inSeconds > IARateLimits.maxBackoffDelaySecs) {
                retryDelay = const Duration(seconds: IARateLimits.maxBackoffDelaySecs);
              }
            }
            continue;
          }
          await response.stream.drain();
          throw ServerException(response.statusCode);
        } else if (response.statusCode != 200) {
          await response.stream.drain();
          throw Exception(
              'Failed to download file: HTTP ${response.statusCode}');
        }

        // Success - proceed with download
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
      } on SocketException catch (e) {
        if (retries < IARateLimits.maxRetries - 1) {
          if (kDebugMode) {
            print('Network error during download: $e, retrying in ${retryDelay.inSeconds}s...');
          }
          await Future.delayed(retryDelay);
          retries++;
          retryDelay = Duration(
              seconds: (retryDelay.inSeconds * IARateLimits.backoffMultiplier).toInt());
          continue;
        }
        rethrow;
      } on TimeoutException {
        if (retries < IARateLimits.maxRetries - 1) {
          if (kDebugMode) {
            print('Download timeout, retrying in ${retryDelay.inSeconds}s...');
          }
          await Future.delayed(retryDelay);
          retries++;
          retryDelay = Duration(
              seconds: (retryDelay.inSeconds * IARateLimits.backoffMultiplier).toInt());
          continue;
        }
        rethrow;
      }
    }

    throw Exception('Failed to download file after ${IARateLimits.maxRetries} attempts');
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

  /// Decompress/extract an archive file
  ///
  /// Supports ZIP, TAR, TAR.GZ, and GZ file formats.
  /// Returns list of extracted file paths.
  /// 
  /// Throws [FileSystemException] if file doesn't exist or directory can't be created.
  /// Throws [FormatException] if archive format is unsupported or corrupted.
  Future<List<String>> decompressFile(
    String archivePath,
    String outputDir,
  ) async {
    final file = File(archivePath);
    
    if (!await file.exists()) {
      throw FileSystemException('Archive file not found', archivePath);
    }

    // Create output directory if it doesn't exist
    final outDir = Directory(outputDir);
    if (!await outDir.exists()) {
      await outDir.create(recursive: true);
    }

    // Get just the filename without path
    final fileName = file.path.split(Platform.pathSeparator).last.toLowerCase();
    final bytes = await file.readAsBytes();
    
    if (kDebugMode) {
      print('Decompressing: $archivePath');
      print('Output directory: $outputDir');
      print('File size: ${bytes.length} bytes');
    }

    final extractedFiles = <String>[];

    try {
      if (fileName.endsWith('.zip')) {
        // Handle ZIP archives
        final archive = ZipDecoder().decodeBytes(bytes);
        extractedFiles.addAll(await _extractArchive(archive, outputDir));
        
      } else if (fileName.endsWith('.tar.gz') || fileName.endsWith('.tgz')) {
        // Handle TAR.GZ archives
        final gzipBytes = GZipDecoder().decodeBytes(bytes);
        final archive = TarDecoder().decodeBytes(gzipBytes);
        extractedFiles.addAll(await _extractArchive(archive, outputDir));
        
      } else if (fileName.endsWith('.tar')) {
        // Handle TAR archives
        final archive = TarDecoder().decodeBytes(bytes);
        extractedFiles.addAll(await _extractArchive(archive, outputDir));
        
      } else if (fileName.endsWith('.gz')) {
        // Handle single GZIP files
        final decompressed = GZipDecoder().decodeBytes(bytes);
        // Extract just the filename without the .gz extension
        // Handle both forward slashes and backslashes in path
        String baseFileName = fileName.substring(0, fileName.length - 3);
        // Remove any directory path components (handle / and \)
        if (baseFileName.contains('/')) {
          baseFileName = baseFileName.split('/').last;
        }
        if (baseFileName.contains('\\')) {
          baseFileName = baseFileName.split('\\').last;
        }
        final outputPath = '$outputDir${Platform.pathSeparator}$baseFileName';
        
        final outputFile = File(outputPath);
        await outputFile.writeAsBytes(decompressed);
        extractedFiles.add(outputPath);
        
      } else {
        throw FormatException(
          'Unsupported archive format. Supported: .zip, .tar, .tar.gz, .tgz, .gz',
          fileName,
        );
      }

      if (kDebugMode) {
        print('Successfully extracted ${extractedFiles.length} file(s)');
      }

      return extractedFiles;
      
    } catch (e) {
      if (e is FormatException) {
        rethrow;
      }
      throw FormatException('Failed to decompress archive: ${e.toString()}', archivePath);
    }
  }

  /// Extract files from an Archive object to the output directory
  Future<List<String>> _extractArchive(Archive archive, String outputDir) async {
    final extractedFiles = <String>[];

    for (final file in archive) {
      if (file.isFile) {
        final outputPath = '$outputDir${Platform.pathSeparator}${file.name}';
        
        // Create parent directories if needed
        final outputFile = File(outputPath);
        if (!await outputFile.parent.exists()) {
          await outputFile.parent.create(recursive: true);
        }

        // Write file content
        await outputFile.writeAsBytes(file.content as List<int>);
        extractedFiles.add(outputPath);
        
        if (kDebugMode) {
          print('Extracted: ${file.name} (${file.size} bytes)');
        }
      }
    }

    return extractedFiles;
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
    } else if (trimmed.contains('://${IAEndpoints.base.replaceAll('https://', '')}/metadata/')) {
      return trimmed;
    } else if (trimmed.startsWith('http://') || trimmed.startsWith('https://')) {
      // It's a URL but not a details or metadata URL - extract identifier
      final uri = Uri.parse(trimmed);
      final segments = uri.pathSegments;
      if (segments.isNotEmpty) {
        final identifier = segments.last;
        return IAUtils.buildMetadataUrl(identifier);
      }
      throw Exception('Cannot extract identifier from URL: $trimmed');
    } else {
      // Assume it's a bare identifier - validate it
      if (!IAUtils.isValidIdentifier(trimmed)) {
        throw Exception(IAErrorMessages.invalidIdentifier);
      }
      return IAUtils.buildMetadataUrl(trimmed);
    }
  }

  /// Enforce rate limiting between requests
  Future<void> _enforceRateLimit() async {
    if (_lastRequestTime != null) {
      final elapsed = DateTime.now().difference(_lastRequestTime!);
      final minDelay = const Duration(milliseconds: IARateLimits.minRequestDelayMs);

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
    return requestsPerMinute < IARateLimits.maxRequestsPerMinute;
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
