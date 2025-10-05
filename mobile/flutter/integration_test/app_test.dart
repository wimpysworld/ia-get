import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:internet_archive_helper/services/internet_archive_api.dart';
import 'dart:io';
import 'package:path_provider/path_provider.dart';
import 'package:crypto/crypto.dart';
import 'package:archive/archive.dart';

/// Comprehensive integration tests for Internet Archive Helper
/// 
/// Tests cover:
/// 1. Metadata fetching
/// 2. File downloading with progress
/// 3. Checksum validation
/// 4. Archive decompression
void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  group('Internet Archive API Integration Tests', () {
    late InternetArchiveApi api;
    late Directory tempDir;
    final testIdentifier = 'commute_test';

    setUpAll(() async {
      api = InternetArchiveApi();
      tempDir = await getTemporaryDirectory();
    });

    tearDown(() async {
      // Cleanup test files
      try {
        final testFiles = tempDir
            .listSync()
            .where((e) => e.path.contains('ia_test'));
        for (var file in testFiles) {
          if (file is File) {
            await file.delete();
          }
        }
      } catch (e) {
        // Ignore cleanup errors
      }
    });

    test('1. Fetch metadata from Internet Archive', () async {
      final metadata = await api.fetchMetadata(testIdentifier);
      
      expect(metadata.identifier, equals(testIdentifier));
      expect(metadata.title, isNotNull);
      expect(metadata.files, isNotEmpty);
      
      // Verify files have required properties
      final firstFile = metadata.files.first;
      expect(firstFile.name, isNotEmpty);
      expect(firstFile.downloadUrl, isNotNull);
    });

    test('2. Fetch metadata with different URL formats', () async {
      // Test with details URL
      final detailsUrl = 'https://archive.org/details/$testIdentifier';
      final metadata1 = await api.fetchMetadata(detailsUrl);
      expect(metadata1.identifier, equals(testIdentifier));

      // Test with metadata URL
      final metadataUrl = 'https://archive.org/metadata/$testIdentifier';
      final metadata2 = await api.fetchMetadata(metadataUrl);
      expect(metadata2.identifier, equals(testIdentifier));
    });

    test('3. Download file with progress tracking', () async {
      final metadata = await api.fetchMetadata(testIdentifier);
      
      // Find small file for testing
      final smallFile = metadata.files.firstWhere(
        (f) => (f.size ?? 0) > 0 && (f.size ?? 0) < 500 * 1024,
        orElse: () => metadata.files.first,
      );

      final outputPath = '${tempDir.path}/ia_test_download_${smallFile.name}';
      var progressCalled = false;

      final result = await api.downloadFile(
        smallFile.downloadUrl!,
        outputPath,
        onProgress: (downloaded, total) {
          progressCalled = true;
          expect(downloaded, lessThanOrEqualTo(total));
        },
      );

      expect(result, equals(outputPath));
      expect(await File(outputPath).exists(), isTrue);
      expect(progressCalled, isTrue);
    });

    test('4. Validate MD5 checksum after download', () async {
      final metadata = await api.fetchMetadata(testIdentifier);
      
      // Find file with MD5 checksum
      final fileWithMd5 = metadata.files.firstWhere(
        (f) => f.md5 != null && f.md5!.isNotEmpty && (f.size ?? 0) < 500 * 1024,
        orElse: () => metadata.files.first,
      );

      if (fileWithMd5.md5 == null) {
        return; // Skip if no MD5 available
      }

      final outputPath = '${tempDir.path}/ia_test_checksum_${fileWithMd5.name}';
      await api.downloadFile(fileWithMd5.downloadUrl!, outputPath);

      // Validate using API method
      final isValid = await api.validateChecksum(
        outputPath,
        fileWithMd5.md5!,
        'md5',
      );

      expect(isValid, isTrue);

      // Also manually verify MD5
      final file = File(outputPath);
      final bytes = await file.readAsBytes();
      final calculatedMd5 = md5.convert(bytes).toString();
      expect(calculatedMd5, equals(fileWithMd5.md5));
    });

    test('5. Handle invalid identifier gracefully', () async {
      const invalidIdentifier = 'this_does_not_exist_12345678';
      
      expect(
        () async => await api.fetchMetadata(invalidIdentifier),
        throwsException,
      );
    });

    test('6. Handle invalid download URL gracefully', () async {
      const invalidUrl = 'https://archive.org/download/fake/file.txt';
      final outputPath = '${tempDir.path}/ia_test_fake.txt';

      expect(
        () async => await api.downloadFile(invalidUrl, outputPath),
        throwsException,
      );
    });

    test('7. Rate limiting compliance', () async {
      final startTime = DateTime.now();
      
      // Make multiple requests
      for (int i = 0; i < 3; i++) {
        await api.fetchMetadata(testIdentifier);
      }
      
      final duration = DateTime.now().difference(startTime);
      
      // Should take at least 4 seconds due to rate limiting (2s between requests)
      expect(duration.inSeconds, greaterThanOrEqualTo(4));
    });
  });

  group('Archive Decompression Integration Tests', () {
    late InternetArchiveApi api;
    late Directory tempDir;

    setUpAll(() async {
      api = InternetArchiveApi();
      tempDir = await getTemporaryDirectory();
    });

    test('8. Decompress GZIP file', () async {
      // Create a test GZIP file
      final testContent = 'Test decompression content';
      final testFile = File('${tempDir.path}/ia_test_file.txt');
      await testFile.writeAsString(testContent);

      // Compress it
      final bytes = await testFile.readAsBytes();
      final compressed = const GZipEncoder().encode(bytes);
      final gzipFile = File('${tempDir.path}/ia_test_file.txt.gz');
      await gzipFile.writeAsBytes(compressed);

      // Decompress using API
      final extractDir = '${tempDir.path}/ia_test_extracted';
      final extractedFiles = await api.decompressFile(
        gzipFile.path,
        extractDir,
      );

      expect(extractedFiles.length, equals(1));
      expect(await File(extractedFiles.first).exists(), isTrue);

      // Verify content
      final extractedContent = await File(extractedFiles.first).readAsString();
      expect(extractedContent, equals(testContent));

      // Cleanup
      await testFile.delete();
      await gzipFile.delete();
      await Directory(extractDir).delete(recursive: true);
    });

    test('9. Handle unsupported archive format', () async {
      final testFile = File('${tempDir.path}/ia_test_unsupported.rar');
      await testFile.writeAsString('fake rar content');

      final extractDir = '${tempDir.path}/ia_test_extract';

      expect(
        () async => await api.decompressFile(testFile.path, extractDir),
        throwsA(isA<FormatException>()),
      );

      await testFile.delete();
    });

    test('10. Handle non-existent archive file', () async {
      const nonExistentPath = '/fake/path/file.zip';
      const extractDir = '/fake/extract';

      expect(
        () async => await api.decompressFile(nonExistentPath, extractDir),
        throwsA(isA<FileSystemException>()),
      );
    });
  });
}
