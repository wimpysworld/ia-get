import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../models/download_progress.dart';
import '../services/background_download_service.dart';
import '../services/notification_service.dart';
import '../utils/file_utils.dart';

/// Widget for managing and displaying active downloads
class DownloadManagerWidget extends StatefulWidget {
  const DownloadManagerWidget({super.key});

  @override
  State<DownloadManagerWidget> createState() => _DownloadManagerWidgetState();
}

class _DownloadManagerWidgetState extends State<DownloadManagerWidget> {
  @override
  void initState() {
    super.initState();
    // Initialize notification service when widget is created
    NotificationService.initialize();
  }

  @override
  Widget build(BuildContext context) {
    return Consumer<BackgroundDownloadService>(
      builder: (context, downloadService, child) {
        if (!downloadService.hasActiveDownloads) {
          return const SizedBox.shrink();
        }

        return Container(
          margin: const EdgeInsets.all(8),
          decoration: BoxDecoration(
            color: Theme.of(context).colorScheme.surface,
            borderRadius: BorderRadius.circular(12),
            border: Border.all(
              color: Theme.of(context).colorScheme.outline.withOpacity(0.2),
            ),
            boxShadow: [
              BoxShadow(
                color: Colors.black.withOpacity(0.1),
                blurRadius: 8,
                offset: const Offset(0, 2),
              ),
            ],
          ),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              _buildHeader(context, downloadService),
              const Divider(height: 1),
              if (downloadService.activeDownloadCount > 0)
                _buildStatisticsBar(context, downloadService),
              _buildDownloadList(context, downloadService),
            ],
          ),
        );
      },
    );
  }

  Widget _buildHeader(BuildContext context, BackgroundDownloadService service) {
    return Padding(
      padding: const EdgeInsets.all(16),
      child: Row(
        children: [
          Icon(
            Icons.download_rounded,
            color: Theme.of(context).colorScheme.primary,
          ),
          const SizedBox(width: 8),
          Expanded(
            child: Text(
              'Downloads (${service.activeDownloadCount})',
              style: Theme.of(
                context,
              ).textTheme.titleMedium?.copyWith(fontWeight: FontWeight.w600),
            ),
          ),
          _buildHeaderActions(context, service),
        ],
      ),
    );
  }

  Widget _buildHeaderActions(
    BuildContext context,
    BackgroundDownloadService service,
  ) {
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        // Minimize/Expand button
        IconButton(
          icon: const Icon(Icons.minimize_rounded, size: 20),
          onPressed: () {
            // Minimize to just show notification summary
            NotificationService.showDownloadSummary(
              activeDownloads: service.activeDownloadCount,
              completedDownloads: 0,
              averageProgress: _calculateAverageProgress(
                service.activeDownloads.values,
              ),
            );
          },
          tooltip: 'Minimize to notifications',
        ),

        // Pause/Resume all button
        IconButton(
          icon: Icon(
            _hasActiveDownloads(service)
                ? Icons.pause_rounded
                : Icons.play_arrow_rounded,
            size: 20,
          ),
          onPressed: () => _toggleAllDownloads(service),
          tooltip: _hasActiveDownloads(service) ? 'Pause all' : 'Resume all',
        ),
      ],
    );
  }

  Widget _buildStatisticsBar(
    BuildContext context,
    BackgroundDownloadService service,
  ) {
    final stats = service.getStatistics();
    final averageSpeed = stats['averageSpeed'] as double;
    final activeBytesDownloaded = stats['activeBytesDownloaded'] as int;

    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 8),
      color: Theme.of(context).colorScheme.surfaceVariant.withOpacity(0.3),
      child: Row(
        mainAxisAlignment: MainAxisAlignment.spaceAround,
        children: [
          _buildStatItem(
            context,
            icon: Icons.speed_rounded,
            label: 'Speed',
            value: averageSpeed > 0
                ? FileUtils.formatTransferSpeed(averageSpeed)
                : '-',
          ),
          _buildStatItem(
            context,
            icon: Icons.cloud_download_rounded,
            label: 'Downloaded',
            value: FileUtils.formatBytes(activeBytesDownloaded),
          ),
          _buildStatItem(
            context,
            icon: Icons.schedule_rounded,
            label: 'Session',
            value: _formatSessionDuration(service.sessionDuration),
          ),
        ],
      ),
    );
  }

  Widget _buildStatItem(
    BuildContext context, {
    required IconData icon,
    required String label,
    required String value,
  }) {
    return Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(icon, size: 16, color: Theme.of(context).colorScheme.primary),
            const SizedBox(width: 4),
            Text(
              label,
              style: Theme.of(context).textTheme.bodySmall?.copyWith(
                color: Theme.of(context).colorScheme.onSurfaceVariant,
              ),
            ),
          ],
        ),
        const SizedBox(height: 2),
        Text(
          value,
          style: Theme.of(
            context,
          ).textTheme.labelLarge?.copyWith(fontWeight: FontWeight.w600),
        ),
      ],
    );
  }

  Widget _buildDownloadList(
    BuildContext context,
    BackgroundDownloadService service,
  ) {
    final downloads = service.activeDownloads.values.toList();
    downloads.sort((a, b) => b.startTime.compareTo(a.startTime));

    return ListView.separated(
      shrinkWrap: true,
      physics: const NeverScrollableScrollPhysics(),
      itemCount: downloads.length,
      separatorBuilder: (context, index) => const Divider(height: 1),
      itemBuilder: (context, index) {
        final download = downloads[index];
        return _buildDownloadItem(context, service, download);
      },
    );
  }

  Widget _buildDownloadItem(
    BuildContext context,
    BackgroundDownloadService service,
    DownloadProgress download,
  ) {
    return Padding(
      padding: const EdgeInsets.all(16),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          _buildDownloadHeader(context, download),
          const SizedBox(height: 8),
          _buildProgressIndicator(context, download),
          const SizedBox(height: 8),
          _buildDownloadInfo(context, download),
          const SizedBox(height: 12),
          _buildDownloadActions(context, service, download),
        ],
      ),
    );
  }

  Widget _buildDownloadHeader(BuildContext context, DownloadProgress download) {
    return Row(
      children: [
        _buildStatusIcon(context, download),
        const SizedBox(width: 8),
        Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                download.identifier,
                style: Theme.of(
                  context,
                ).textTheme.titleSmall?.copyWith(fontWeight: FontWeight.w600),
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
              ),
              if (download.currentFile != null)
                Text(
                  download.currentFile!,
                  style: Theme.of(context).textTheme.bodySmall?.copyWith(
                    color: Theme.of(context).colorScheme.onSurfaceVariant,
                  ),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                ),
            ],
          ),
        ),
        Text(
          _formatProgress(download),
          style: Theme.of(context).textTheme.bodySmall?.copyWith(
            fontWeight: FontWeight.w500,
            color: _getStatusColor(context, download),
          ),
        ),
      ],
    );
  }

  Widget _buildStatusIcon(BuildContext context, DownloadProgress download) {
    IconData icon;
    Color color;

    switch (download.status) {
      case DownloadStatus.downloading:
        icon = Icons.download_rounded;
        color = Theme.of(context).colorScheme.primary;
        break;
      case DownloadStatus.paused:
        icon = Icons.pause_rounded;
        color = Theme.of(context).colorScheme.secondary;
        break;
      case DownloadStatus.completed:
        icon = Icons.check_circle_rounded;
        color = Colors.green;
        break;
      case DownloadStatus.error:
        icon = Icons.error_rounded;
        color = Colors.red;
        break;
      case DownloadStatus.cancelled:
        icon = Icons.cancel_rounded;
        color = Colors.orange;
        break;
      default:
        icon = Icons.schedule_rounded;
        color = Theme.of(context).colorScheme.outline;
    }

    return Icon(icon, size: 20, color: color);
  }

  Widget _buildProgressIndicator(
    BuildContext context,
    DownloadProgress download,
  ) {
    final progress = download.progress ?? 0.0;
    final isIndeterminate =
        download.status == DownloadStatus.queued || progress < 0;

    return Column(
      children: [
        LinearProgressIndicator(
          value: isIndeterminate ? null : progress,
          backgroundColor: Theme.of(context).colorScheme.surfaceVariant,
          valueColor: AlwaysStoppedAnimation<Color>(
            _getStatusColor(context, download),
          ),
        ),
        const SizedBox(height: 4),
        Row(
          mainAxisAlignment: MainAxisAlignment.spaceBetween,
          children: [
            Text(
              '${download.completedFiles ?? 0} / ${download.totalFiles} files',
              style: Theme.of(context).textTheme.bodySmall,
            ),
            if (download.transferSpeed != null)
              Text(
                FileUtils.formatTransferSpeed(download.transferSpeed!),
                style: Theme.of(context).textTheme.bodySmall,
              ),
          ],
        ),
      ],
    );
  }

  Widget _buildDownloadInfo(BuildContext context, DownloadProgress download) {
    final info = <String>[];

    if (download.downloadedBytes != null && download.totalBytes != null) {
      info.add(
        '${FileUtils.formatBytes(download.downloadedBytes!)} / '
        '${FileUtils.formatBytes(download.totalBytes!)}',
      );
    }

    if (download.errorMessage != null) {
      info.add('Error: ${download.errorMessage}');
    }

    if (info.isEmpty) return const SizedBox.shrink();

    return Text(
      info.join(' • '),
      style: Theme.of(context).textTheme.bodySmall?.copyWith(
        color: download.status == DownloadStatus.error
            ? Colors.red
            : Theme.of(context).colorScheme.onSurfaceVariant,
      ),
      maxLines: 2,
      overflow: TextOverflow.ellipsis,
    );
  }

  Widget _buildDownloadActions(
    BuildContext context,
    BackgroundDownloadService service,
    DownloadProgress download,
  ) {
    return Row(
      children: [
        // Primary action button
        if (download.status == DownloadStatus.downloading)
          _buildActionButton(
            context,
            icon: Icons.pause_rounded,
            label: 'Pause',
            onPressed: () => service.pauseDownload(download.downloadId),
          )
        else if (download.status == DownloadStatus.paused)
          _buildActionButton(
            context,
            icon: Icons.play_arrow_rounded,
            label: 'Resume',
            onPressed: () => service.resumeDownload(download.downloadId),
          )
        else if (download.status == DownloadStatus.error)
          _buildActionButton(
            context,
            icon: Icons.refresh_rounded,
            label: 'Retry',
            onPressed: () => _retryDownload(service, download),
          ),

        const SizedBox(width: 8),

        // Cancel button (always available except for completed)
        if (download.status != DownloadStatus.completed)
          _buildActionButton(
            context,
            icon: Icons.close_rounded,
            label: 'Cancel',
            onPressed: () => _cancelDownload(context, service, download),
            color: Colors.red,
          ),

        const Spacer(),

        // View details button
        _buildActionButton(
          context,
          icon: Icons.info_outline_rounded,
          label: 'Details',
          onPressed: () => _showDownloadDetails(context, download),
        ),
      ],
    );
  }

  Widget _buildActionButton(
    BuildContext context, {
    required IconData icon,
    required String label,
    required VoidCallback onPressed,
    Color? color,
  }) {
    return TextButton.icon(
      onPressed: onPressed,
      icon: Icon(icon, size: 16, color: color),
      label: Text(label, style: TextStyle(color: color)),
      style: TextButton.styleFrom(
        padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
        minimumSize: Size.zero,
        tapTargetSize: MaterialTapTargetSize.shrinkWrap,
      ),
    );
  }

  // Helper methods
  String _formatProgress(DownloadProgress download) {
    if (download.progress == null) return '0%';
    return '${(download.progress! * 100).toInt()}%';
  }

  Color _getStatusColor(BuildContext context, DownloadProgress download) {
    switch (download.status) {
      case DownloadStatus.downloading:
        return Theme.of(context).colorScheme.primary;
      case DownloadStatus.paused:
        return Theme.of(context).colorScheme.secondary;
      case DownloadStatus.completed:
        return Colors.green;
      case DownloadStatus.error:
        return Colors.red;
      case DownloadStatus.cancelled:
        return Colors.orange;
      default:
        return Theme.of(context).colorScheme.outline;
    }
  }

  double _calculateAverageProgress(Iterable<DownloadProgress> downloads) {
    if (downloads.isEmpty) return 0.0;
    final total = downloads.fold(0.0, (sum, d) => sum + (d.progress ?? 0.0));
    return total / downloads.length;
  }

  bool _hasActiveDownloads(BackgroundDownloadService service) {
    return service.activeDownloads.values.any(
      (d) => d.status == DownloadStatus.downloading,
    );
  }

  String _formatSessionDuration(Duration? duration) {
    if (duration == null) return '-';

    final hours = duration.inHours;
    final minutes = duration.inMinutes.remainder(60);
    final seconds = duration.inSeconds.remainder(60);

    if (hours > 0) {
      return '${hours}h ${minutes}m';
    } else if (minutes > 0) {
      return '${minutes}m ${seconds}s';
    } else {
      return '${seconds}s';
    }
  }

  Future<void> _toggleAllDownloads(BackgroundDownloadService service) async {
    if (_hasActiveDownloads(service)) {
      // Pause all active downloads
      for (final download in service.activeDownloads.values) {
        if (download.status == DownloadStatus.downloading) {
          await service.pauseDownload(download.downloadId);
        }
      }
    } else {
      // Resume all paused downloads
      for (final download in service.activeDownloads.values) {
        if (download.status == DownloadStatus.paused) {
          await service.resumeDownload(download.downloadId);
        }
      }
    }
  }

  Future<void> _retryDownload(
    BackgroundDownloadService service,
    DownloadProgress download,
  ) async {
    // Implementation would need to restart the download with same parameters
    // This requires storing original download parameters
    ScaffoldMessenger.of(context).showSnackBar(
      const SnackBar(content: Text('Retry functionality not yet implemented')),
    );
  }

  Future<void> _cancelDownload(
    BuildContext context,
    BackgroundDownloadService service,
    DownloadProgress download,
  ) async {
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('Cancel Download'),
        content: Text(
          'Are you sure you want to cancel the download of "${download.identifier}"?',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: const Text('No'),
          ),
          TextButton(
            onPressed: () => Navigator.pop(context, true),
            child: const Text('Yes'),
          ),
        ],
      ),
    );

    if (confirmed == true) {
      await service.cancelDownload(download.downloadId);
      await NotificationService.cancelNotification(download.downloadId);
    }
  }

  void _showDownloadDetails(BuildContext context, DownloadProgress download) {
    showDialog(
      context: context,
      builder: (context) => AlertDialog(
        title: Text(download.identifier),
        content: SingleChildScrollView(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            mainAxisSize: MainAxisSize.min,
            children: [
              _DetailRow('Status', download.status.name),
              _DetailRow('Progress', _formatProgress(download)),
              _DetailRow(
                'Files',
                '${download.completedFiles ?? 0} / ${download.totalFiles}',
              ),
              if (download.downloadedBytes != null)
                _DetailRow(
                  'Downloaded',
                  FileUtils.formatBytes(download.downloadedBytes!),
                ),
              if (download.totalBytes != null)
                _DetailRow(
                  'Total Size',
                  FileUtils.formatBytes(download.totalBytes!),
                ),
              if (download.transferSpeed != null)
                _DetailRow(
                  'Speed',
                  FileUtils.formatTransferSpeed(download.transferSpeed!),
                ),
              if (download.currentFile != null)
                _DetailRow('Current File', download.currentFile!),
              if (download.errorMessage != null)
                _DetailRow('Error', download.errorMessage!),
              _DetailRow('Started', download.startTime.toString()),
            ],
          ),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context),
            child: const Text('Close'),
          ),
        ],
      ),
    );
  }
}

class _DetailRow extends StatelessWidget {
  final String label;
  final String value;

  const _DetailRow(this.label, this.value);

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(bottom: 8),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          SizedBox(
            width: 100,
            child: Text(
              '$label:',
              style: Theme.of(
                context,
              ).textTheme.bodySmall?.copyWith(fontWeight: FontWeight.w600),
            ),
          ),
          Expanded(
            child: Text(value, style: Theme.of(context).textTheme.bodySmall),
          ),
        ],
      ),
    );
  }
}
