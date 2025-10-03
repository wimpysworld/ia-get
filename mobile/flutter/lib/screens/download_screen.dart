import 'package:flutter/material.dart';
import 'package:percent_indicator/percent_indicator.dart';
import '../models/download_progress.dart';

class DownloadScreen extends StatefulWidget {
  const DownloadScreen({super.key});

  @override
  State<DownloadScreen> createState() => _DownloadScreenState();
}

class _DownloadScreenState extends State<DownloadScreen> {
  final List<DownloadProgress> _activeDownloads = [];
  final List<DownloadProgress> _completedDownloads = [];

  @override
  Widget build(BuildContext context) {
    return WillPopScope(
      onWillPop: () async {
        // Always allow back navigation
        return true;
      },
      child: Scaffold(
        appBar: AppBar(
          title: const Text('Downloads'),
          actions: [
            IconButton(
              icon: const Icon(Icons.clear_all),
              onPressed: _activeDownloads.isEmpty ? null : _clearAllDownloads,
            ),
          ],
        ),
        body: _activeDownloads.isEmpty && _completedDownloads.isEmpty
            ? const Center(
                child: Column(
                  mainAxisAlignment: MainAxisAlignment.center,
                  children: [
                    Icon(
                      Icons.download_done,
                      size: 64,
                      color: Colors.grey,
                    ),
                    SizedBox(height: 16),
                    Text(
                      'No downloads yet',
                      style: TextStyle(
                        fontSize: 16,
                        color: Colors.grey,
                      ),
                    ),
                    SizedBox(height: 8),
                    Text(
                      'Start downloading files from the main screen',
                      style: TextStyle(
                        fontSize: 14,
                        color: Colors.grey,
                      ),
                    ),
                  ],
                ),
              )
            : ListView(
                padding: const EdgeInsets.all(16),
                children: [
                  if (_activeDownloads.isNotEmpty) ...[
                    const Text(
                      'Active Downloads',
                      style: TextStyle(
                        fontSize: 18,
                        fontWeight: FontWeight.bold,
                      ),
                    ),
                    const SizedBox(height: 8),
                    ..._activeDownloads.map(_buildActiveDownloadCard),
                    const SizedBox(height: 24),
                  ],
                  if (_completedDownloads.isNotEmpty) ...[
                    const Text(
                      'Completed Downloads',
                      style: TextStyle(
                        fontSize: 18,
                        fontWeight: FontWeight.bold,
                      ),
                    ),
                    const SizedBox(height: 8),
                    ..._completedDownloads.map(_buildCompletedDownloadCard),
                  ],
                ],
              ),
      ),
    );
  }

  Widget _buildActiveDownloadCard(DownloadProgress progress) {
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
                    progress.currentFile ?? 'Preparing download...',
                    style: const TextStyle(
                      fontWeight: FontWeight.w500,
                      fontSize: 16,
                    ),
                    overflow: TextOverflow.ellipsis,
                  ),
                ),
                IconButton(
                  icon: const Icon(Icons.pause),
                  onPressed: () => _pauseDownload(progress.sessionId),
                ),
                IconButton(
                  icon: const Icon(Icons.stop),
                  onPressed: () => _cancelDownload(progress.sessionId),
                ),
              ],
            ),
            const SizedBox(height: 12),
            LinearPercentIndicator(
              percent: progress.overallProgress,
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
                  '${(progress.overallProgress * 100).toStringAsFixed(1)}%',
                  style: const TextStyle(fontWeight: FontWeight.w500),
                ),
                Text(progress.speedFormatted),
              ],
            ),
            const SizedBox(height: 4),
            Row(
              mainAxisAlignment: MainAxisAlignment.spaceBetween,
              children: [
                Text(
                  '${progress.completedFiles}/${progress.totalFiles} files',
                  style: TextStyle(
                    fontSize: 12,
                    color: Colors.grey.shade600,
                  ),
                ),
                Text(
                  'ETA: ${progress.etaFormatted}',
                  style: TextStyle(
                    fontSize: 12,
                    color: Colors.grey.shade600,
                  ),
                ),
              ],
            ),
            const SizedBox(height: 4),
            Text(
              '${progress.downloadedFormatted} / ${progress.totalSizeFormatted}',
              style: TextStyle(
                fontSize: 12,
                color: Colors.grey.shade600,
              ),
            ),
          ],
        ),
      ),
    );
  }

  Widget _buildCompletedDownloadCard(DownloadProgress progress) {
    return Card(
      margin: const EdgeInsets.only(bottom: 12),
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Row(
          children: [
            const Icon(
              Icons.check_circle,
              color: Colors.green,
              size: 24,
            ),
            const SizedBox(width: 12),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    progress.currentFile ?? 'Download completed',
                    style: const TextStyle(
                      fontWeight: FontWeight.w500,
                    ),
                    overflow: TextOverflow.ellipsis,
                  ),
                  Text(
                    '${progress.totalFiles} files • ${progress.totalSizeFormatted}',
                    style: TextStyle(
                      fontSize: 12,
                      color: Colors.grey.shade600,
                    ),
                  ),
                ],
              ),
            ),
            IconButton(
              icon: const Icon(Icons.folder_open),
              onPressed: () => _openDownloadFolder(progress),
            ),
          ],
        ),
      ),
    );
  }

  void _pauseDownload(int sessionId) {
    // TODO: Implement pause functionality
    ScaffoldMessenger.of(context).showSnackBar(
      const SnackBar(content: Text('Download paused')),
    );
  }

  void _cancelDownload(int sessionId) {
    setState(() {
      _activeDownloads.removeWhere((p) => p.sessionId == sessionId);
    });
    ScaffoldMessenger.of(context).showSnackBar(
      const SnackBar(content: Text('Download cancelled')),
    );
  }

  void _clearAllDownloads() {
    setState(() {
      _activeDownloads.clear();
      _completedDownloads.clear();
    });
  }

  void _openDownloadFolder(DownloadProgress progress) {
    // TODO: Implement folder opening
    ScaffoldMessenger.of(context).showSnackBar(
      const SnackBar(content: Text('Opening download folder')),
    );
  }
}