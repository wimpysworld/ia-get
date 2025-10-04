import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import 'package:percent_indicator/percent_indicator.dart';
import 'package:open_file/open_file.dart';
import '../models/download_progress.dart';
import '../providers/download_provider.dart';
import '../utils/file_utils.dart';

class DownloadScreen extends StatefulWidget {
  const DownloadScreen({super.key});

  /// Route name for navigation tracking
  static const routeName = '/downloads';

  @override
  State<DownloadScreen> createState() => _DownloadScreenState();
}

class _DownloadScreenState extends State<DownloadScreen> {
  @override
  Widget build(BuildContext context) {
    return Consumer<DownloadProvider>(
      builder: (context, downloadProvider, child) {
        final downloads = downloadProvider.downloads;
        final activeDownloads = downloads.values
            .where((d) => d.status == 'downloading' || d.status == 'fetching_metadata')
            .toList();
        final completedDownloads = downloads.values
            .where((d) => d.status == 'complete')
            .toList();

        return PopScope(
          canPop: true,
          child: Scaffold(
            appBar: AppBar(
              title: const Text('Downloads'),
              actions: [
                if (downloads.isNotEmpty)
                  IconButton(
                    icon: const Icon(Icons.clear_all),
                    onPressed: () => _clearAllDownloads(downloadProvider),
                  ),
              ],
            ),
            body: downloads.isEmpty
                ? const Center(
                    child: Column(
                      mainAxisAlignment: MainAxisAlignment.center,
                      children: [
                        Icon(Icons.download_done, size: 64, color: Colors.grey),
                        SizedBox(height: 16),
                        Text(
                          'No downloads yet',
                          style: TextStyle(fontSize: 16, color: Colors.grey),
                        ),
                        SizedBox(height: 8),
                        Text(
                          'Start downloading files from the main screen',
                          style: TextStyle(fontSize: 14, color: Colors.grey),
                        ),
                      ],
                    ),
                  )
                : ListView(
                    padding: const EdgeInsets.all(16),
                    children: [
                      if (activeDownloads.isNotEmpty) ...[
                        const Text(
                          'Active Downloads',
                          style: TextStyle(fontSize: 18, fontWeight: FontWeight.bold),
                        ),
                        const SizedBox(height: 8),
                        ...activeDownloads.map((state) => _buildActiveDownloadCard(state, downloadProvider)),
                        const SizedBox(height: 24),
                      ],
                      if (completedDownloads.isNotEmpty) ...[
                        const Text(
                          'Completed Downloads',
                          style: TextStyle(fontSize: 18, fontWeight: FontWeight.bold),
                        ),
                        const SizedBox(height: 8),
                        ...completedDownloads.map((state) => _buildCompletedDownloadCard(state, downloadProvider)),
                      ],
                    ],
                  ),
          ),
        );
      },
    );
  }

  Widget _buildActiveDownloadCard(downloadState, DownloadProvider provider) {
    final identifier = downloadState.identifier;
    final overallProgress = downloadState.overallProgress / 100.0; // Convert to 0-1 range
    
    return Card(
      margin: const EdgeInsets.only(bottom: 12),
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                Expanded(
                  child: Text(
                    identifier,
                    style: const TextStyle(
                      fontWeight: FontWeight.w500,
                      fontSize: 16,
                    ),
                    overflow: TextOverflow.ellipsis,
                  ),
                ),
                IconButton(
                  icon: const Icon(Icons.stop),
                  tooltip: 'Cancel download',
                  onPressed: () => _cancelDownload(identifier, provider),
                ),
              ],
            ),
            const SizedBox(height: 8),
            Text(
              downloadState.status == 'fetching_metadata' 
                ? 'Fetching metadata...' 
                : 'Downloading ${downloadState.fileProgress.length} files',
              style: TextStyle(color: Colors.grey.shade600),
            ),
            const SizedBox(height: 12),
            LinearPercentIndicator(
              percent: overallProgress,
              backgroundColor: Colors.grey.shade300,
              progressColor: Theme.of(context).primaryColor,
              lineHeight: 8,
              barRadius: const Radius.circular(4),
            ),
            const SizedBox(height: 8),
            Row(
              mainAxisAlignment: MainAxisAlignment.spaceBetween,
              children: [
                Text(
                  '${(overallProgress * 100).toStringAsFixed(1)}%',
                  style: const TextStyle(fontWeight: FontWeight.w500),
                ),
                Text(
                  '${FileUtils.formatSize(downloadState.totalDownloaded)} / ${FileUtils.formatSize(downloadState.totalSize)}',
                  style: TextStyle(color: Colors.grey.shade600),
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }
  Widget _buildCompletedDownloadCard(downloadState, DownloadProvider provider) {
    final identifier = downloadState.identifier;
    final fileCount = downloadState.metadata?.files.length ?? 0;
    
    return Card(
      margin: const EdgeInsets.only(bottom: 12),
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Row(
          children: [
            const Icon(Icons.check_circle, color: Colors.green, size: 24),
            const SizedBox(width: 12),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    identifier,
                    style: const TextStyle(fontWeight: FontWeight.w500),
                    overflow: TextOverflow.ellipsis,
                  ),
                  Text(
                    '$fileCount files • ${FileUtils.formatSize(downloadState.totalSize)}',
                    style: TextStyle(fontSize: 12, color: Colors.grey.shade600),
                  ),
                ],
              ),
            ),
            IconButton(
              icon: const Icon(Icons.folder_open),
              tooltip: 'Open download folder',
              onPressed: () => _openDownloadFolder(identifier),
            ),
            IconButton(
              icon: const Icon(Icons.delete),
              tooltip: 'Clear from list',
              onPressed: () => provider.clearDownload(identifier),
            ),
          ],
        ),
      ),
    );
  }

  void _cancelDownload(String identifier, DownloadProvider provider) async {
    try {
      await provider.cancelDownload(identifier);
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          const SnackBar(content: Text('Download cancelled')),
        );
      }
    } catch (e) {
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('Failed to cancel: $e')),
        );
      }
    }
  }

  void _clearAllDownloads(DownloadProvider provider) {
    showDialog(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('Clear All Downloads'),
        content: const Text('Are you sure you want to clear all downloads?'),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context),
            child: const Text('Cancel'),
          ),
          TextButton(
            onPressed: () {
              provider.clearCompletedDownloads();
              Navigator.pop(context);
            },
            child: const Text('Clear'),
          ),
        ],
      ),
    );
  }

  void _openDownloadFolder(String identifier) async {
    // Try to open the download folder
    final downloadPath = '/storage/emulated/0/Download/ia-get/$identifier';
    
    try {
      final result = await OpenFile.open(downloadPath);
      if (mounted && result.type != ResultType.done) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('Could not open folder: ${result.message}')),
        );
      }
    } catch (e) {
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('Error opening folder: $e')),
        );
      }
    }
  }
}
