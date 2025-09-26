import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../services/ia_get_service.dart';
import '../widgets/search_bar_widget.dart';
import '../widgets/archive_info_widget.dart';
import '../widgets/file_list_widget.dart';
import '../widgets/filter_controls_widget.dart';
import '../widgets/download_controls_widget.dart';
import 'download_screen.dart';
import 'help_screen.dart';

class HomeScreen extends StatefulWidget {
  const HomeScreen({super.key});

  @override
  State<HomeScreen> createState() => _HomeScreenState();
}

class _HomeScreenState extends State<HomeScreen> {
  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      context.read<IaGetService>().initialize();
    });
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: const Text('Internet Archive Helper'),
        actions: [
          IconButton(
            icon: const Icon(Icons.help_outline),
            onPressed: () {
              Navigator.push(
                context,
                MaterialPageRoute(builder: (_) => const HelpScreen()),
              );
            },
          ),
          IconButton(
            icon: const Icon(Icons.download_rounded),
            onPressed: () {
              Navigator.push(
                context,
                MaterialPageRoute(builder: (_) => const DownloadScreen()),
              );
            },
          ),
        ],
      ),
      body: Consumer<IaGetService>(
        builder: (context, service, child) {
          if (!service.isInitialized) {
            return const Center(
              child: Column(
                mainAxisAlignment: MainAxisAlignment.center,
                children: [
                  CircularProgressIndicator(),
                  SizedBox(height: 16),
                  Text('Initializing IA Get...'),
                ],
              ),
            );
          }

          return Column(
            children: [
              // Search bar
              const SearchBarWidget(),
              
              // Error display
              if (service.error != null)
                Container(
                  width: double.infinity,
                  padding: const EdgeInsets.all(16),
                  margin: const EdgeInsets.all(8),
                  decoration: BoxDecoration(
                    color: Colors.red.shade100,
                    borderRadius: BorderRadius.circular(8),
                    border: Border.all(color: Colors.red.shade300),
                  ),
                  child: Row(
                    children: [
                      Icon(Icons.error_outline, color: Colors.red.shade700),
                      const SizedBox(width: 8),
                      Expanded(
                        child: Text(
                          service.error!,
                          style: TextStyle(color: Colors.red.shade700),
                        ),
                      ),
                    ],
                  ),
                ),

              // Loading indicator
              if (service.isLoading)
                const LinearProgressIndicator(),

              // Archive information
              if (service.currentMetadata != null)
                ArchiveInfoWidget(metadata: service.currentMetadata!),

              // Filter controls
              if (service.currentMetadata != null)
                const FilterControlsWidget(),

              // File list
              Expanded(
                child: service.currentMetadata != null
                    ? FileListWidget(files: service.filteredFiles)
                    : const Center(
                        child: Column(
                          mainAxisAlignment: MainAxisAlignment.center,
                          children: [
                            Icon(
                              Icons.search,
                              size: 64,
                              color: Colors.grey,
                            ),
                            SizedBox(height: 16),
                            Text(
                              'Search for an Internet Archive identifier',
                              style: TextStyle(
                                fontSize: 16,
                                color: Colors.grey,
                              ),
                            ),
                            SizedBox(height: 8),
                            Text(
                              'e.g., "commute_test" or "nasa_images"',
                              style: TextStyle(
                                fontSize: 14,
                                color: Colors.grey,
                              ),
                            ),
                          ],
                        ),
                      ),
              ),

              // Download controls
              if (service.currentMetadata != null)
                const DownloadControlsWidget(),
            ],
          );
        },
      ),
    );
  }
}