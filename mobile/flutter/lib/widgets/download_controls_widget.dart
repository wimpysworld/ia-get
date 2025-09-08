import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../services/ia_get_service.dart';
import '../models/archive_metadata.dart';

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
  Widget build(BuildContext context) {
    return Consumer<IaGetService>(
      builder: (context, service, child) {
        final selectedFiles = service.filteredFiles.where((f) => f.selected).toList();
        final canDownload = selectedFiles.isNotEmpty;
        final totalSize = service.calculateTotalSize(selectedFiles);

        return Container(
          decoration: BoxDecoration(
            color: Theme.of(context).colorScheme.surface,
            border: Border(
              top: BorderSide(
                color: Colors.grey.shade300,
                width: 1,
              ),
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
                              '${selectedFiles.length} files selected',
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
                          ],
                        ),
                      ),
                      // Settings button
                      IconButton(
                        icon: const Icon(Icons.settings),
                        onPressed: _showDownloadSettings,
                      ),
                    ],
                  ),
                ),

              // Download controls
              Padding(
                padding: const EdgeInsets.all(16),
                child: Row(
                  children: [
                    // Download button
                    Expanded(
                      child: ElevatedButton.icon(
                        onPressed: canDownload ? _startDownload : null,
                        icon: const Icon(Icons.download),
                        label: Text(canDownload 
                            ? 'Download ${selectedFiles.length} Files'
                            : 'Select Files to Download'),
                        style: ElevatedButton.styleFrom(
                          padding: const EdgeInsets.symmetric(vertical: 16),
                        ),
                      ),
                    ),
                    
                    if (canDownload) ...[
                      const SizedBox(width: 8),
                      
                      // Preview button
                      OutlinedButton.icon(
                        onPressed: _previewDownload,
                        icon: const Icon(Icons.preview),
                        label: const Text('Preview'),
                      ),
                    ],
                  ],
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
      builder: (context) => DraggableScrollableSheet(
        initialChildSize: 0.7,
        maxChildSize: 0.9,
        minChildSize: 0.5,
        builder: (context, scrollController) => Container(
          padding: const EdgeInsets.all(16),
          child: ListView(
            controller: scrollController,
            children: [
              // Handle
              Center(
                child: Container(
                  width: 32,
                  height: 4,
                  decoration: BoxDecoration(
                    color: Colors.grey,
                    borderRadius: BorderRadius.circular(2),
                  ),
                ),
              ),
              const SizedBox(height: 16),
              
              const Text(
                'Download Settings',
                style: TextStyle(
                  fontSize: 20,
                  fontWeight: FontWeight.bold,
                ),
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
                subtitle: const Text('Automatically extract ZIP, TAR, and other archives'),
                value: _autoDecompress,
                onChanged: (value) {
                  setState(() {
                    _autoDecompress = value;
                  });
                },
              ),

              // Verify checksums
              SwitchListTile(
                title: const Text('Verify file checksums'),
                subtitle: const Text('Verify MD5/SHA1 checksums after download'),
                value: _verifyChecksums,
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

  void _previewDownload() {
    final service = context.read<IaGetService>();
    final selectedFiles = service.filteredFiles.where((f) => f.selected).toList();
    final totalSize = service.calculateTotalSize(selectedFiles);

    showDialog(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('Download Preview'),
        content: SizedBox(
          width: double.maxFinite,
          height: 300,
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text('Files to download: ${selectedFiles.length}'),
              Text('Total size: ${_formatSize(totalSize)}'),
              Text('Download location: $_outputPath'),
              Text('Concurrent downloads: $_concurrentDownloads'),
              if (_autoDecompress) const Text('• Auto-decompress enabled'),
              if (_verifyChecksums) const Text('• Checksum verification enabled'),
              
              const SizedBox(height: 16),
              const Text(
                'Files:',
                style: TextStyle(fontWeight: FontWeight.w500),
              ),
              const SizedBox(height: 8),
              
              Expanded(
                child: ListView.builder(
                  itemCount: selectedFiles.length,
                  itemBuilder: (context, index) {
                    final file = selectedFiles[index];
                    return ListTile(
                      dense: true,
                      title: Text(
                        file.displayName,
                        style: const TextStyle(fontSize: 14),
                      ),
                      trailing: Text(
                        file.sizeFormatted,
                        style: TextStyle(
                          fontSize: 12,
                          color: Colors.grey.shade600,
                        ),
                      ),
                    );
                  },
                ),
              ),
            ],
          ),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context),
            child: const Text('Cancel'),
          ),
          ElevatedButton(
            onPressed: () {
              Navigator.pop(context);
              _startDownload();
            },
            child: const Text('Start Download'),
          ),
        ],
      ),
    );
  }

  void _startDownload() {
    final service = context.read<IaGetService>();
    final selectedFiles = service.filteredFiles.where((f) => f.selected).toList();

    if (selectedFiles.isEmpty) {
      ScaffoldMessenger.of(context).showSnackBar(
        const SnackBar(
          content: Text('Please select files to download'),
        ),
      );
      return;
    }

    // Show confirmation dialog
    showDialog(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('Start Download'),
        content: Text(
          'Download ${selectedFiles.length} files to $_outputPath?',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context),
            child: const Text('Cancel'),
          ),
          ElevatedButton(
            onPressed: () {
              Navigator.pop(context);
              _performDownload(selectedFiles);
            },
            child: const Text('Start'),
          ),
        ],
      ),
    );
  }

  void _performDownload(List<ArchiveFile> files) {
    // TODO: Implement actual download using FFI
    ScaffoldMessenger.of(context).showSnackBar(
      SnackBar(
        content: Text('Starting download of ${files.length} files...'),
        duration: const Duration(seconds: 2),
      ),
    );

    // Navigate to download screen
    Navigator.pushNamed(context, '/downloads');
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