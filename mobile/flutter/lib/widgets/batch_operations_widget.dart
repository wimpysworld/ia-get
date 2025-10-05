import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../models/archive_metadata.dart';
import '../providers/download_provider.dart';
import '../core/utils/formatting_utils.dart';

/// Widget for batch operations on selected files
/// 
/// Provides quick actions for multiple selected files:
/// - Download all selected files
/// - Deselect all
/// - Copy file names
/// - Share file list
class BatchOperationsWidget extends StatelessWidget {
  final String identifier;
  final List<ArchiveFile> selectedFiles;
  final VoidCallback onDeselectAll;

  const BatchOperationsWidget({
    super.key,
    required this.identifier,
    required this.selectedFiles,
    required this.onDeselectAll,
  });

  @override
  Widget build(BuildContext context) {
    if (selectedFiles.isEmpty) {
      return const SizedBox.shrink();
    }

    final totalSize = selectedFiles.fold<int>(
      0,
      (sum, file) => sum + (file.size ?? 0),
    );

    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 12),
      decoration: BoxDecoration(
        color: Theme.of(context).primaryColor.withValues(alpha: 0.1),
        border: Border(
          top: BorderSide(
            color: Theme.of(context).primaryColor.withValues(alpha: 0.3),
            width: 2,
          ),
        ),
      ),
      child: Row(
        children: [
          // Selection info
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              mainAxisSize: MainAxisSize.min,
              children: [
                Text(
                  '${selectedFiles.length} file${selectedFiles.length == 1 ? '' : 's'} selected',
                  style: Theme.of(context).textTheme.titleMedium?.copyWith(
                    fontWeight: FontWeight.bold,
                  ),
                ),
                const SizedBox(height: 2),
                Text(
                  'Total: ${FormattingUtils.formatBytes(totalSize)}',
                  style: Theme.of(context).textTheme.bodySmall?.copyWith(
                    color: Colors.grey.shade700,
                  ),
                ),
              ],
            ),
          ),
          
          // Deselect button
          IconButton(
            icon: const Icon(Icons.clear),
            tooltip: 'Deselect all',
            onPressed: onDeselectAll,
          ),
          
          const SizedBox(width: 8),
          
          // Download button
          ElevatedButton.icon(
            icon: const Icon(Icons.download, size: 20),
            label: const Text('Download'),
            style: ElevatedButton.styleFrom(
              backgroundColor: Theme.of(context).primaryColor,
              foregroundColor: Colors.white,
              padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 12),
            ),
            onPressed: () => _handleBatchDownload(context),
          ),
        ],
      ),
    );
  }

  void _handleBatchDownload(BuildContext context) async {
    final provider = context.read<DownloadProvider>();
    
    // Show confirmation dialog
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('Confirm Batch Download'),
        content: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text('Download ${selectedFiles.length} selected file${selectedFiles.length == 1 ? '' : 's'}?'),
            const SizedBox(height: 12),
            Text(
              'Total size: ${FormattingUtils.formatBytes(selectedFiles.fold<int>(0, (sum, f) => sum + (f.size ?? 0)))}',
              style: Theme.of(context).textTheme.bodyMedium?.copyWith(
                color: Colors.grey.shade700,
              ),
            ),
            const SizedBox(height: 8),
            Container(
              padding: const EdgeInsets.all(8),
              decoration: BoxDecoration(
                color: Colors.blue.shade50,
                borderRadius: BorderRadius.circular(4),
                border: Border.all(color: Colors.blue.shade200),
              ),
              child: Row(
                children: [
                  Icon(Icons.info_outline, size: 16, color: Colors.blue.shade700),
                  const SizedBox(width: 8),
                  Expanded(
                    child: Text(
                      'Files will be downloaded concurrently',
                      style: TextStyle(
                        fontSize: 12,
                        color: Colors.blue.shade900,
                      ),
                    ),
                  ),
                ],
              ),
            ),
          ],
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(context).pop(false),
            child: const Text('Cancel'),
          ),
          ElevatedButton(
            onPressed: () => Navigator.of(context).pop(true),
            child: const Text('Download'),
          ),
        ],
      ),
    );

    if (confirmed == true && context.mounted) {
      try {
        // Extract file names
        final fileNames = selectedFiles.map((f) => f.name).toList();
        
        // Start batch download
        await provider.batchDownload(identifier, fileNames);
        
        if (context.mounted) {
          ScaffoldMessenger.of(context).showSnackBar(
            SnackBar(
              content: Text('Started downloading ${fileNames.length} files'),
              backgroundColor: Colors.green,
              action: SnackBarAction(
                label: 'View',
                textColor: Colors.white,
                onPressed: () {
                  Navigator.pushNamed(context, '/downloads');
                },
              ),
            ),
          );
        }
      } catch (e) {
        if (context.mounted) {
          ScaffoldMessenger.of(context).showSnackBar(
            SnackBar(
              content: Text('Failed to start batch download: $e'),
              backgroundColor: Colors.red,
            ),
          );
        }
      }
    }
  }
}
