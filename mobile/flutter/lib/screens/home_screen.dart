import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../services/ia_get_service.dart';
import '../widgets/search_bar_widget.dart';
import '../widgets/download_manager_widget.dart';
import 'archive_detail_screen.dart';
import 'download_screen.dart';
import 'help_screen.dart';
import 'settings_screen.dart';

class HomeScreen extends StatefulWidget {
  const HomeScreen({super.key});

  @override
  State<HomeScreen> createState() => _HomeScreenState();
}

class _HomeScreenState extends State<HomeScreen> {
  bool _hasNavigated = false;
  
  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      context.read<IaGetService>().initialize();
      
      // Listen for metadata changes to navigate to detail screen
      context.read<IaGetService>().addListener(_onServiceChanged);
    });
  }
  
  @override
  void dispose() {
    context.read<IaGetService>().removeListener(_onServiceChanged);
    super.dispose();
  }
  
  void _onServiceChanged() {
    final service = context.read<IaGetService>();
    
    // Navigate to detail screen when metadata is loaded (only once)
    if (service.currentMetadata != null && mounted && !_hasNavigated) {
      _hasNavigated = true;
      
      Navigator.of(context).push(
        MaterialPageRoute(
          builder: (context) => const ArchiveDetailScreen(),
        ),
      ).then((_) {
        // Reset flag when returning from detail screen
        _hasNavigated = false;
      });
    } else if (service.currentMetadata == null) {
      // Reset flag when metadata is cleared
      _hasNavigated = false;
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: const Text('Search'),
        actions: [
          IconButton(
            icon: const Icon(Icons.settings),
            onPressed: () {
              Navigator.push(
                context,
                MaterialPageRoute(builder: (_) => const SettingsScreen()),
              );
            },
          ),
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
              
              // Error display with circuit breaker reset option
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
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Row(
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
                      // Add reset button if circuit breaker is open
                      if (service.error!.contains('temporarily unavailable') ||
                          service.error!.contains('circuit breaker'))
                        Padding(
                          padding: const EdgeInsets.only(top: 8),
                          child: ElevatedButton.icon(
                            onPressed: () {
                              service.resetCircuitBreaker();
                              ScaffoldMessenger.of(context).showSnackBar(
                                const SnackBar(
                                  content: Text('Service reset. You can try searching again.'),
                                ),
                              );
                            },
                            icon: const Icon(Icons.refresh),
                            label: const Text('Reset Service'),
                            style: ElevatedButton.styleFrom(
                              backgroundColor: Colors.orange,
                            ),
                          ),
                        ),
                    ],
                  ),
                ),

              // Search suggestions
              if (service.suggestions.isNotEmpty)
                Expanded(
                  child: Container(
                    width: double.infinity,
                    padding: const EdgeInsets.all(16),
                    margin: const EdgeInsets.symmetric(horizontal: 8),
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text(
                          'Did you mean:',
                          style: TextStyle(
                            fontSize: 16,
                            fontWeight: FontWeight.bold,
                            color: Colors.grey.shade700,
                          ),
                        ),
                        const SizedBox(height: 12),
                        Expanded(
                          child: ListView.builder(
                            shrinkWrap: true,
                            itemCount: service.suggestions.length,
                            itemBuilder: (context, index) {
                              final suggestion = service.suggestions[index];
                              return Card(
                                margin: const EdgeInsets.only(bottom: 8),
                                child: ListTile(
                                  leading: const Icon(Icons.archive),
                                  title: Text(suggestion['title']!),
                                  subtitle: Text(suggestion['identifier']!),
                                  trailing: const Icon(Icons.arrow_forward),
                                  onTap: () {
                                    // Reset circuit breaker before fetching
                                    service.resetCircuitBreaker();
                                    // Fetch metadata for the suggested archive
                                    service.fetchMetadata(suggestion['identifier']!);
                                  },
                                ),
                              );
                            },
                          ),
                        ),
                      ],
                    ),
                  ),
                ),

              // Loading indicator
              if (service.isLoading)
                const LinearProgressIndicator(),

              // Empty state when not loading and no metadata
              if (!service.isLoading && 
                  service.currentMetadata == null && 
                  service.suggestions.isEmpty &&
                  service.error == null)
                Expanded(
                  child: Center(
                    child: Column(
                      mainAxisAlignment: MainAxisAlignment.center,
                      children: [
                        Icon(
                          Icons.search,
                          size: 64,
                          color: Colors.grey.shade400,
                        ),
                        const SizedBox(height: 16),
                        Text(
                          'Search for an Internet Archive identifier',
                          style: TextStyle(
                            fontSize: 16,
                            color: Colors.grey.shade600,
                          ),
                        ),
                        const SizedBox(height: 8),
                        Text(
                          'e.g., "commute_test" or "nasa_images"',
                          style: TextStyle(
                            fontSize: 14,
                            color: Colors.grey.shade500,
                          ),
                        ),
                      ],
                    ),
                  ),
                ),
                
              // Active downloads manager at bottom
              const DownloadManagerWidget(),
            ],
          );
        },
      ),
    );
  }
}