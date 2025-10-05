import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../services/archive_service.dart';
import '../services/background_download_service.dart';
import '../screens/download_screen.dart';
import '../screens/settings_screen.dart';
import '../utils/file_utils.dart';
import '../utils/permission_utils.dart';

class DownloadControlsWidget extends StatefulWidget {
  const DownloadControlsWidget({super.key});

  @override
  State<DownloadControlsWidget> createState() => _DownloadControlsWidgetState();
}

class _DownloadControlsWidgetState extends State<DownloadControlsWidget> {
  String _outputPath = '/storage/emulated/0/Download/ia-get';
  int _concurrentDownloads = 3;
  bool _autoDecompress = false;
  bool _verifyChecksums = true;

  @override
  void initState() {
    super.initState();
    _loadSettings();
  }

  Future<void> _loadSettings() async {
    final outputPath = await SettingsScreen.getDownloadPath();
    final concurrent = await SettingsScreen.getConcurrentDownloads();
    final decompress = await SettingsScreen.getAutoDecompress();
    final verify = await SettingsScreen.getVerifyChecksums();

    if (mounted) {
      setState(() {
        _outputPath = outputPath;
        _concurrentDownloads = concurrent;
        _autoDecompress = decompress;
        _verifyChecksums = verify;
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    return Consumer<ArchiveService>(
      builder: (context, service, child) {
        final selectedFiles = service.filteredFiles
            .where((f) => f.selected)
            .toList();
        final canDownload = selectedFiles.isNotEmpty;
        final totalSize = service.calculateTotalSize(selectedFiles);

        return Container(
          decoration: BoxDecoration(
            color: Theme.of(context).colorScheme.surface,
            border: Border(
              top: BorderSide(color: Colors.grey.shade300, width: 1),
            ),
          ),
          child: Column(
            children: [
              // Selection summary
              if (canDownload)
                Container(
                  width: double.infinity,
                  padding: const EdgeInsets.all(16),
                  child: Row(
                    children: [
                      const Icon(Icons.download, color: Colors.blue),
                      const SizedBox(width: 8),
                      Expanded(
                        child: Column(
                          crossAxisAlignment: CrossAxisAlignment.start,
                          children: [
                            Text(
                              '${selectedFiles.length} file${selectedFiles.length == 1 ? '' : 's'} selected',
                              style: const TextStyle(
                                fontWeight: FontWeight.w500,
                              ),
                            ),
                            Text(
                              'Total size: ${_formatSize(totalSize)}',
                              style: TextStyle(
                                fontSize: 12,
                                color: Colors.grey.shade600,
                              ),
                            ),
                            Text(
                              'Location: $_outputPath',
                              style: TextStyle(
                                fontSize: 11,
                                color: Colors.grey.shade500,
                              ),
                              maxLines: 1,
                              overflow: TextOverflow.ellipsis,
                            ),
                          ],
                        ),
                      ),
                      // Settings button
                      IconButton(
                        icon: const Icon(Icons.settings),
                        onPressed: _showDownloadSettings,
                        tooltip: 'Download settings',
                      ),
                    ],
                  ),
                ),

              // Download controls
              Padding(
                padding: const EdgeInsets.all(16),
                child: SizedBox(
                  width: double.infinity,
                  child: ElevatedButton.icon(
                    onPressed: canDownload ? _performDownload : null,
                    icon: const Icon(Icons.download),
                    label: Text(
                      canDownload
                          ? 'Download ${selectedFiles.length} File${selectedFiles.length == 1 ? '' : 's'}'
                          : 'Select Files to Download',
                    ),
                    style: ElevatedButton.styleFrom(
                      padding: const EdgeInsets.symmetric(vertical: 16),
                    ),
                  ),
                ),
              ),
            ],
          ),
        );
      },
    );
  }

  void _showDownloadSettings() {
    showModalBottomSheet(
      context: context,
      isScrollControlled: true,
      builder: (context) => Padding(
        padding: EdgeInsets.only(
          bottom: MediaQuery.of(context).viewInsets.bottom,
        ),
        child: Container(
          padding: const EdgeInsets.all(16),
          decoration: BoxDecoration(
            color: Theme.of(context).scaffoldBackgroundColor,
            borderRadius: const BorderRadius.vertical(
              top: Radius.circular(16),
            ),
          ),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              // Handle
              Center(
                child: Container(
                  width: 32,
                  height: 4,
                  margin: const EdgeInsets.only(bottom: 16),
                  decoration: BoxDecoration(
                    color: Colors.grey,
                    borderRadius: BorderRadius.circular(2),
                  ),
                ),
              ),

              const Text(
                'Download Settings',
                style: TextStyle(fontSize: 20, fontWeight: FontWeight.bold),
              ),
              const SizedBox(height: 24),

              // Output path
              const Text(
                'Download Location',
                style: TextStyle(fontWeight: FontWeight.w500),
              ),
              const SizedBox(height: 8),
              TextFormField(
                initialValue: _outputPath,
                decoration: const InputDecoration(
                  border: OutlineInputBorder(),
                  suffixIcon: Icon(Icons.folder),
                ),
                onChanged: (value) => _outputPath = value,
              ),
              const SizedBox(height: 16),

              // Concurrent downloads
              const Text(
                'Concurrent Downloads',
                style: TextStyle(fontWeight: FontWeight.w500),
              ),
              const SizedBox(height: 8),
              Row(
                children: [
                  Expanded(
                    child: Slider(
                      value: _concurrentDownloads.toDouble(),
                      min: 1,
                      max: 10,
                      divisions: 9,
                      label: _concurrentDownloads.toString(),
                      onChanged: (value) {
                        setState(() {
                          _concurrentDownloads = value.round();
                        });
                      },
                    ),
                  ),
                  SizedBox(
                    width: 40,
                    child: Text(
                      _concurrentDownloads.toString(),
                      textAlign: TextAlign.center,
                      style: const TextStyle(fontWeight: FontWeight.w500),
                    ),
                  ),
                ],
              ),
              const SizedBox(height: 16),

              // Auto decompress
              SwitchListTile(
                title: const Text('Auto-decompress archives'),
                subtitle: const Text(
                  'Automatically extract ZIP, TAR, and other archives',
                ),
                value: _autoDecompress,
                contentPadding: EdgeInsets.zero,
                onChanged: (value) {
                  setState(() {
                    _autoDecompress = value;
                  });
                },
              ),

              // Verify checksums
              SwitchListTile(
                title: const Text('Verify file checksums'),
                subtitle: const Text(
                  'Verify MD5/SHA1 checksums after download',
                ),
                value: _verifyChecksums,
                contentPadding: EdgeInsets.zero,
                onChanged: (value) {
                  setState(() {
                    _verifyChecksums = value;
                  });
                },
              ),

              const SizedBox(height: 24),

              // Close button
              SizedBox(
                width: double.infinity,
                child: ElevatedButton(
                  onPressed: () => Navigator.pop(context),
                  child: const Text('Done'),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }

  void _performDownload() async {
    final service = context.read<ArchiveService>();
    final downloadService = context.read<BackgroundDownloadService>();
    final selectedFiles = service.filteredFiles
        .where((f) => f.selected)
        .toList();

    if (service.currentMetadata == null) {
      ScaffoldMessenger.of(context).showSnackBar(
        const SnackBar(content: Text('No archive metadata available')),
      );
      return;
    }

    if (selectedFiles.isEmpty) {
      ScaffoldMessenger.of(context).showSnackBar(
        const SnackBar(content: Text('Please select files to download')),
      );
      return;
    }

    // Check and request storage permissions first
    final hasPermission = await PermissionUtils.hasStoragePermissions();

    if (!hasPermission) {
      if (!mounted) return;
      
      // Show rationale before requesting permission
      final shouldRequest = await PermissionUtils.showPermissionRationaleDialog(
        context: context,
        title: 'Storage Permission Required',
        message:
            'This app needs storage permission to download and save files from the Internet Archive. '
            'Your files will be saved to the Download folder.',
      );

      if (!shouldRequest) {
        if (!mounted) return;
        ScaffoldMessenger.of(context).showSnackBar(
          const SnackBar(
            content: Text('Storage permission is required to download files'),
            duration: Duration(seconds: 3),
          ),
        );
        return;
      }

      // Request storage permissions
      final granted = await PermissionUtils.requestStoragePermissions();

      if (!granted) {
        if (!mounted) return;

        // Show settings dialog if permission was denied
        await PermissionUtils.showSettingsDialog(
          context: context,
          message:
              'Storage permission is required to download files. '
              'Please enable it in app settings to continue.',
        );
        return;
      }
    }

    // Calculate total download size
    final totalSize = selectedFiles.fold<int>(
      0,
      (sum, file) => sum + (file.size ?? 0),
    );

    // Check disk space before starting download
    final hasSufficientSpace = await FileUtils.hasSufficientSpace(
      _outputPath,
      totalSize,
    );

    if (hasSufficientSpace == false) {
      // Insufficient disk space - show error dialog
      final availableSpace = await FileUtils.getAvailableSpace(_outputPath);
      final requiredSpace = FileUtils.getRequiredSpaceWithMargin(totalSize);
      final shortage = requiredSpace - (availableSpace ?? 0);

      if (!mounted) return;
      showDialog(
        context: context,
        builder: (context) => AlertDialog(
          title: Row(
            children: [
              Icon(Icons.warning, color: Colors.orange.shade700),
              const SizedBox(width: 8),
              const Text('Insufficient Disk Space'),
            ],
          ),
          content: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                'Not enough disk space available for this download.',
                style: TextStyle(color: Colors.grey.shade700, fontSize: 14),
              ),
              const SizedBox(height: 16),
              _buildSpaceInfoRow(
                'Required:',
                _formatSize(requiredSpace),
                color: Colors.red.shade700,
              ),
              _buildSpaceInfoRow(
                'Available:',
                availableSpace != null
                    ? _formatSize(availableSpace)
                    : 'Unknown',
              ),
              _buildSpaceInfoRow(
                'Shortage:',
                _formatSize(shortage),
                color: Colors.orange.shade700,
              ),
              const SizedBox(height: 8),
              Text(
                'Note: Required size includes a safety margin for temporary files.',
                style: TextStyle(
                  fontSize: 12,
                  color: Colors.grey.shade600,
                  fontStyle: FontStyle.italic,
                ),
              ),
            ],
          ),
          actions: [
            TextButton(
              onPressed: () => Navigator.pop(context),
              child: const Text('OK'),
            ),
          ],
        ),
      );
      return;
    }

    try {
      // Start background download
      final downloadId = await downloadService.startBackgroundDownload(
        identifier: service.currentMetadata!.identifier,
        selectedFiles: selectedFiles.map((f) => f.name).toList(),
        downloadPath: _outputPath,
        includeFormats: null, // Will be handled by file selection
        excludeFormats: null,
        maxSize: null,
      );

      if (downloadId != null) {
        if (!mounted) return;
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(
            content: Text(
              'Download started: ${selectedFiles.length} file${selectedFiles.length == 1 ? '' : 's'}',
            ),
            action: SnackBarAction(
              label: 'View',
              onPressed: () {
                // Navigate to downloads screen
                Navigator.push(
                  context,
                  MaterialPageRoute(builder: (_) => const DownloadScreen()),
                );
              },
            ),
            duration: const Duration(seconds: 3),
          ),
        );
      } else {
        throw Exception(
          'Failed to start download - native download service may not be initialized',
        );
      }
    } catch (e) {
      if (!mounted) return;

      // Show more helpful error message with actionable steps
      showDialog(
        context: context,
        builder: (context) => AlertDialog(
          title: const Row(
            children: [
              Icon(Icons.error_outline, color: Colors.red),
              SizedBox(width: 8),
              Text('Download Failed'),
            ],
          ),
          content: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              const Text(
                'Unable to start download. This could be due to:',
                style: TextStyle(fontWeight: FontWeight.w500),
              ),
              const SizedBox(height: 12),
              Text(
                '• Background download service not available\n'
                '• Missing storage permissions (check Settings)\n'
                '• Network connectivity issues\n'
                '• Invalid download path',
                style: TextStyle(fontSize: 14, color: Colors.grey.shade700),
              ),
              const SizedBox(height: 12),
              Container(
                padding: const EdgeInsets.all(12),
                decoration: BoxDecoration(
                  color: Colors.orange.shade50,
                  borderRadius: BorderRadius.circular(8),
                  border: Border.all(color: Colors.orange.shade200),
                ),
                child: Row(
                  children: [
                    Icon(
                      Icons.info_outline,
                      size: 20,
                      color: Colors.orange.shade700,
                    ),
                    const SizedBox(width: 8),
                    Expanded(
                      child: Text(
                        'Tip: Make sure storage permissions are enabled in app settings',
                        style: TextStyle(
                          fontSize: 12,
                          color: Colors.orange.shade900,
                        ),
                      ),
                    ),
                  ],
                ),
              ),
              const SizedBox(height: 12),
              Text(
                'Technical details: $e',
                style: TextStyle(
                  fontSize: 12,
                  color: Colors.grey.shade600,
                  fontStyle: FontStyle.italic,
                ),
              ),
            ],
          ),
          actions: [
            TextButton(
              onPressed: () => Navigator.pop(context),
              child: const Text('Cancel'),
            ),
            ElevatedButton.icon(
              onPressed: () async {
                // Capture context before async operations
                final navigator = Navigator.of(context);
                
                navigator.pop();
                // Retry after checking permissions again
                final hasPermission =
                    await PermissionUtils.hasStoragePermissions();
                if (!mounted) return;
                
                if (!hasPermission) {
                  await PermissionUtils.showSettingsDialog(
                    context: context,
                    message:
                        'Storage permission is required. Please enable it in Settings.',
                  );
                } else {
                  _performDownload();
                }
              },
              icon: const Icon(Icons.refresh),
              label: const Text('Retry'),
            ),
          ],
        ),
      );
    }
  }

  Widget _buildSpaceInfoRow(String label, String value, {Color? color}) {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 4),
      child: Row(
        mainAxisAlignment: MainAxisAlignment.spaceBetween,
        children: [
          Text(label, style: const TextStyle(fontWeight: FontWeight.w500)),
          Text(
            value,
            style: TextStyle(fontWeight: FontWeight.bold, color: color),
          ),
        ],
      ),
    );
  }

  String _formatSize(int bytes) {
    const units = ['B', 'KB', 'MB', 'GB', 'TB'];
    double size = bytes.toDouble();
    int unitIndex = 0;

    while (size >= 1024 && unitIndex < units.length - 1) {
      size /= 1024;
      unitIndex++;
    }

    return '${size.toStringAsFixed(size >= 100 ? 0 : 1)} ${units[unitIndex]}';
  }
}
