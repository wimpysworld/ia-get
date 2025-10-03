import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../services/ia_get_service.dart';
import '../widgets/archive_info_widget.dart';
import '../widgets/file_list_widget.dart';
import '../widgets/download_controls_widget.dart';
import '../widgets/download_manager_widget.dart';

/// Screen showing archive details with files and download options
class ArchiveDetailScreen extends StatefulWidget {
  const ArchiveDetailScreen({super.key});

  /// Route name for navigation tracking and state restoration
  static const routeName = '/archive-detail';

  @override
  State<ArchiveDetailScreen> createState() => _ArchiveDetailScreenState();
}

class _ArchiveDetailScreenState extends State<ArchiveDetailScreen> {
  bool _isPopping = false;

  @override
  Widget build(BuildContext context) {
    return PopScope(
      canPop: true,
      onPopInvokedWithResult: (didPop, result) {
        if (didPop) {
          // Clear metadata when going back to search
          // Use Provider.of with listen: false for safer context access in callbacks
          final service = Provider.of<IaGetService>(context, listen: false);
          service.clearMetadata();
        }
      },
      child: Scaffold(
        appBar: AppBar(
          title: Consumer<IaGetService>(
            builder: (context, service, child) {
              return Text(
                service.currentMetadata?.identifier ?? 'Archive Details',
                overflow: TextOverflow.ellipsis,
              );
            },
          ),
        ),
        body: Consumer<IaGetService>(
          builder: (context, service, child) {
            // If no metadata and not already popping, go back to search
            if (service.currentMetadata == null && !_isPopping) {
              _isPopping = true;
              WidgetsBinding.instance.addPostFrameCallback((_) {
                if (mounted && Navigator.of(context).canPop()) {
                  Navigator.of(context).pop();
                }
              });
              return const Center(child: CircularProgressIndicator());
            }

            // If we have metadata again, reset the popping flag
            if (service.currentMetadata != null && _isPopping) {
              _isPopping = false;
            }

            // Show loading if we're in the popping state but still have metadata
            if (_isPopping) {
              return const Center(child: CircularProgressIndicator());
            }

            return Column(
              children: [
                // Archive information
                ArchiveInfoWidget(metadata: service.currentMetadata!),

                // File list (with integrated filter controls)
                Expanded(child: FileListWidget(files: service.filteredFiles)),

                // Download controls
                const DownloadControlsWidget(),

                // Active downloads manager
                const DownloadManagerWidget(),
              ],
            );
          },
        ),
      ),
    );
  }
}
