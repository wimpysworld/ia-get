import 'package:flutter/material.dart';
import 'package:url_launcher/url_launcher.dart';

/// Help and About screen for Internet Archive Helper
class HelpScreen extends StatelessWidget {
  const HelpScreen({super.key});

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(title: const Text('Help & About')),
      body: ListView(
        padding: const EdgeInsets.all(16),
        children: [
          // About section
          Card(
            child: Padding(
              padding: const EdgeInsets.all(16),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Row(
                    children: [
                      Icon(
                        Icons.info_outline,
                        color: Theme.of(context).primaryColor,
                      ),
                      const SizedBox(width: 8),
                      Expanded(
                        child: Text(
                          'About This App',
                          style: Theme.of(context).textTheme.titleLarge,
                        ),
                      ),
                    ],
                  ),
                  const SizedBox(height: 16),
                  const Text(
                    'Internet Archive Helper is your comprehensive companion for accessing the vast digital collection of the Internet Archive. Download books, movies, music, software, and historical documents with ease.',
                  ),
                  const SizedBox(height: 16),
                  Container(
                    padding: const EdgeInsets.all(12),
                    decoration: BoxDecoration(
                      color: Colors.blue.withValues(alpha: 0.1),
                      borderRadius: BorderRadius.circular(8),
                      border: Border.all(color: Colors.blue.withValues(alpha: 0.3)),
                    ),
                    child: Row(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Icon(Icons.info, size: 20, color: Colors.blue[700]),
                        const SizedBox(width: 8),
                        const Expanded(
                          child: Text(
                            'This is an unofficial, community-developed application and is not affiliated with or endorsed by the Internet Archive.',
                            style: TextStyle(fontSize: 13),
                          ),
                        ),
                      ],
                    ),
                  ),
                  const SizedBox(height: 16),
                  const Text('Version: 1.6.0'),
                  const SizedBox(height: 8),
                  const Text(
                    'Built with ❤️ for the Internet Archive community',
                  ),
                ],
              ),
            ),
          ),

          const SizedBox(height: 16),

          // How to use section
          Card(
            child: Padding(
              padding: const EdgeInsets.all(16),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Row(
                    children: [
                      Icon(
                        Icons.help_outline,
                        color: Theme.of(context).primaryColor,
                      ),
                      const SizedBox(width: 8),
                      Text(
                        'How to Use',
                        style: Theme.of(context).textTheme.titleLarge,
                      ),
                    ],
                  ),
                  const SizedBox(height: 16),
                  _buildHelpItem(
                    context,
                    Icons.search,
                    'Search & Browse',
                    'Enter an Internet Archive identifier in the search box or browse collections.',
                  ),
                  _buildHelpItem(
                    context,
                    Icons.filter_list,
                    'Filter Files',
                    'Use filters to find specific file types, sizes, or formats you need.',
                  ),
                  _buildHelpItem(
                    context,
                    Icons.download,
                    'Download',
                    'Select files and tap download. Files are saved to your device storage.',
                  ),
                  _buildHelpItem(
                    context,
                    Icons.pause_circle_outline,
                    'Resume Downloads',
                    'Interrupted downloads will automatically resume when you restart them.',
                  ),
                ],
              ),
            ),
          ),

          const SizedBox(height: 16),

          // Features section
          Card(
            child: Padding(
              padding: const EdgeInsets.all(16),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Row(
                    children: [
                      Icon(
                        Icons.star_outline,
                        color: Theme.of(context).primaryColor,
                      ),
                      const SizedBox(width: 8),
                      Text(
                        'Key Features',
                        style: Theme.of(context).textTheme.titleLarge,
                      ),
                    ],
                  ),
                  const SizedBox(height: 16),
                  _buildFeatureItem(
                    'High-performance downloads with progress tracking',
                  ),
                  _buildFeatureItem(
                    'Smart resume capability for interrupted downloads',
                  ),
                  _buildFeatureItem(
                    'Advanced filtering by file type, size, and format',
                  ),
                  _buildFeatureItem('Material 3 design optimized for mobile'),
                  _buildFeatureItem(
                    'Background downloads that continue when app is closed',
                  ),
                  _buildFeatureItem(
                    'Deep link support for Internet Archive URLs',
                  ),
                ],
              ),
            ),
          ),

          const SizedBox(height: 16),

          // Links section
          Card(
            child: Padding(
              padding: const EdgeInsets.all(16),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Row(
                    children: [
                      Icon(Icons.link, color: Theme.of(context).primaryColor),
                      const SizedBox(width: 8),
                      Text(
                        'Useful Links',
                        style: Theme.of(context).textTheme.titleLarge,
                      ),
                    ],
                  ),
                  const SizedBox(height: 16),
                  _buildLinkItem(
                    context,
                    Icons.public,
                    'Internet Archive',
                    'Visit the Internet Archive website',
                    'https://archive.org',
                  ),
                  _buildLinkItem(
                    context,
                    Icons.code,
                    'Source Code',
                    'View the project on GitHub',
                    'https://github.com/Gameaday/ia-get-cli',
                  ),
                  _buildLinkItem(
                    context,
                    Icons.bug_report,
                    'Report Issues',
                    'Report bugs or request features',
                    'https://github.com/Gameaday/ia-get-cli/issues',
                  ),
                  _buildLinkItem(
                    context,
                    Icons.policy,
                    'Privacy Policy',
                    'Read our privacy policy',
                    'https://github.com/Gameaday/ia-get-cli/blob/main/PRIVACY_POLICY.md',
                  ),
                ],
              ),
            ),
          ),

          const SizedBox(height: 16),

          // Privacy section
          Card(
            child: Padding(
              padding: const EdgeInsets.all(16),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Row(
                    children: [
                      Icon(
                        Icons.privacy_tip_outlined,
                        color: Theme.of(context).primaryColor,
                      ),
                      const SizedBox(width: 8),
                      Text(
                        'Privacy & Data',
                        style: Theme.of(context).textTheme.titleLarge,
                      ),
                    ],
                  ),
                  const SizedBox(height: 16),
                  const Text(
                    'Internet Archive Helper respects your privacy:',
                    style: TextStyle(fontWeight: FontWeight.w500),
                  ),
                  const SizedBox(height: 8),
                  _buildFeatureItem('No personal data collection or tracking'),
                  _buildFeatureItem('No advertisements or analytics'),
                  _buildFeatureItem(
                    'Only accesses Internet Archive public APIs',
                  ),
                  _buildFeatureItem(
                    'Downloaded files stored locally on your device',
                  ),
                  _buildFeatureItem('Open source - you can verify the code'),
                ],
              ),
            ),
          ),
        ],
      ),
    );
  }

  Widget _buildHelpItem(
    BuildContext context,
    IconData icon,
    String title,
    String description,
  ) {
    return Padding(
      padding: const EdgeInsets.only(bottom: 16),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Icon(icon, size: 20, color: Theme.of(context).primaryColor),
          const SizedBox(width: 12),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  title,
                  style: const TextStyle(fontWeight: FontWeight.w500),
                ),
                const SizedBox(height: 4),
                Text(description, style: Theme.of(context).textTheme.bodySmall),
              ],
            ),
          ),
        ],
      ),
    );
  }

  Widget _buildFeatureItem(String text) {
    return Padding(
      padding: const EdgeInsets.only(bottom: 8),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const Icon(Icons.check_circle, size: 16, color: Colors.green),
          const SizedBox(width: 8),
          Expanded(child: Text(text)),
        ],
      ),
    );
  }

  Widget _buildLinkItem(
    BuildContext context,
    IconData icon,
    String title,
    String description,
    String url,
  ) {
    return Padding(
      padding: const EdgeInsets.only(bottom: 12),
      child: InkWell(
        onTap: () => _launchUrl(url),
        borderRadius: BorderRadius.circular(8),
        child: Padding(
          padding: const EdgeInsets.all(8),
          child: Row(
            children: [
              Icon(icon, color: Theme.of(context).primaryColor),
              const SizedBox(width: 12),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      title,
                      style: const TextStyle(fontWeight: FontWeight.w500),
                    ),
                    Text(
                      description,
                      style: Theme.of(context).textTheme.bodySmall,
                    ),
                  ],
                ),
              ),
              const Icon(Icons.open_in_new, size: 16),
            ],
          ),
        ),
      ),
    );
  }

  Future<void> _launchUrl(String urlString) async {
    final url = Uri.parse(urlString);
    if (await canLaunchUrl(url)) {
      await launchUrl(url, mode: LaunchMode.externalApplication);
    }
  }
}
