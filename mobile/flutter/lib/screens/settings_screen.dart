import 'package:flutter/material.dart';
import 'package:shared_preferences/shared_preferences.dart';

class SettingsScreen extends StatefulWidget {
  const SettingsScreen({super.key});

  @override
  State<SettingsScreen> createState() => _SettingsScreenState();
  
  /// Get current download path preference
  static Future<String> getDownloadPath() async {
    final prefs = await SharedPreferences.getInstance();
    return prefs.getString('download_path') ?? '/storage/emulated/0/Download/ia-get';
  }
  
  /// Get concurrent downloads preference
  static Future<int> getConcurrentDownloads() async {
    final prefs = await SharedPreferences.getInstance();
    return prefs.getInt('concurrent_downloads') ?? 3;
  }
  
  /// Get auto-decompress preference
  static Future<bool> getAutoDecompress() async {
    final prefs = await SharedPreferences.getInstance();
    return prefs.getBool('auto_decompress') ?? false;
  }
  
  /// Get verify checksums preference
  static Future<bool> getVerifyChecksums() async {
    final prefs = await SharedPreferences.getInstance();
    return prefs.getBool('verify_checksums') ?? true;
  }
  
  /// Get show hidden files preference
  static Future<bool> getShowHiddenFiles() async {
    final prefs = await SharedPreferences.getInstance();
    return prefs.getBool('show_hidden_files') ?? false;
  }
}

class _SettingsScreenState extends State<SettingsScreen> {
  late SharedPreferences _prefs;
  bool _isLoading = true;
  
  // Settings values
  String _downloadPath = '/storage/emulated/0/Download/ia-get';
  int _concurrentDownloads = 3;
  bool _autoDecompress = false;
  bool _verifyChecksums = true;
  bool _showHiddenFiles = false;
  
  // Settings keys
  static const String _keyDownloadPath = 'download_path';
  static const String _keyConcurrentDownloads = 'concurrent_downloads';
  static const String _keyAutoDecompress = 'auto_decompress';
  static const String _keyVerifyChecksums = 'verify_checksums';
  static const String _keyShowHiddenFiles = 'show_hidden_files';
  
  @override
  void initState() {
    super.initState();
    _loadSettings();
  }
  
  Future<void> _loadSettings() async {
    _prefs = await SharedPreferences.getInstance();
    setState(() {
      _downloadPath = _prefs.getString(_keyDownloadPath) ?? _downloadPath;
      _concurrentDownloads = _prefs.getInt(_keyConcurrentDownloads) ?? _concurrentDownloads;
      _autoDecompress = _prefs.getBool(_keyAutoDecompress) ?? _autoDecompress;
      _verifyChecksums = _prefs.getBool(_keyVerifyChecksums) ?? _verifyChecksums;
      _showHiddenFiles = _prefs.getBool(_keyShowHiddenFiles) ?? _showHiddenFiles;
      _isLoading = false;
    });
  }
  
  Future<void> _saveDownloadPath(String value) async {
    await _prefs.setString(_keyDownloadPath, value);
    setState(() {
      _downloadPath = value;
    });
  }
  
  Future<void> _saveConcurrentDownloads(int value) async {
    await _prefs.setInt(_keyConcurrentDownloads, value);
    setState(() {
      _concurrentDownloads = value;
    });
  }
  
  Future<void> _saveAutoDecompress(bool value) async {
    await _prefs.setBool(_keyAutoDecompress, value);
    setState(() {
      _autoDecompress = value;
    });
  }
  
  Future<void> _saveVerifyChecksums(bool value) async {
    await _prefs.setBool(_keyVerifyChecksums, value);
    setState(() {
      _verifyChecksums = value;
    });
  }
  
  Future<void> _saveShowHiddenFiles(bool value) async {
    await _prefs.setBool(_keyShowHiddenFiles, value);
    setState(() {
      _showHiddenFiles = value;
    });
  }
  
  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: const Text('Settings'),
      ),
      body: _isLoading
          ? const Center(child: CircularProgressIndicator())
          : ListView(
              children: [
                // Download Settings Section
                _buildSectionHeader('Download Settings'),
                
                ListTile(
                  leading: const Icon(Icons.folder),
                  title: const Text('Download Location'),
                  subtitle: Text(_downloadPath),
                  trailing: const Icon(Icons.edit),
                  onTap: _showDownloadPathDialog,
                ),
                
                ListTile(
                  leading: const Icon(Icons.file_download),
                  title: const Text('Concurrent Downloads'),
                  subtitle: Text('$_concurrentDownloads files at a time'),
                  trailing: SizedBox(
                    width: 100,
                    child: Row(
                      mainAxisAlignment: MainAxisAlignment.end,
                      children: [
                        IconButton(
                          icon: const Icon(Icons.remove),
                          onPressed: _concurrentDownloads > 1
                              ? () => _saveConcurrentDownloads(_concurrentDownloads - 1)
                              : null,
                        ),
                        Text('$_concurrentDownloads'),
                        IconButton(
                          icon: const Icon(Icons.add),
                          onPressed: _concurrentDownloads < 10
                              ? () => _saveConcurrentDownloads(_concurrentDownloads + 1)
                              : null,
                        ),
                      ],
                    ),
                  ),
                ),
                
                SwitchListTile(
                  secondary: const Icon(Icons.archive),
                  title: const Text('Auto-decompress Archives'),
                  subtitle: const Text('Automatically extract ZIP, TAR, and other archives'),
                  value: _autoDecompress,
                  onChanged: _saveAutoDecompress,
                ),
                
                SwitchListTile(
                  secondary: const Icon(Icons.verified),
                  title: const Text('Verify Checksums'),
                  subtitle: const Text('Verify MD5/SHA1 checksums after download'),
                  value: _verifyChecksums,
                  onChanged: _saveVerifyChecksums,
                ),
                
                const Divider(),
                
                // File Browser Settings Section
                _buildSectionHeader('File Browser'),
                
                SwitchListTile(
                  secondary: const Icon(Icons.visibility),
                  title: const Text('Show Hidden Files'),
                  subtitle: const Text('Show files starting with . or _'),
                  value: _showHiddenFiles,
                  onChanged: _saveShowHiddenFiles,
                ),
                
                const Divider(),
                
                // About Section
                _buildSectionHeader('About'),
                
                ListTile(
                  leading: const Icon(Icons.info),
                  title: const Text('App Version'),
                  subtitle: const Text('1.6.0'),
                ),
                
                ListTile(
                  leading: const Icon(Icons.library_books),
                  title: const Text('Internet Archive'),
                  subtitle: const Text('A digital library of Internet sites and other cultural artifacts'),
                  onTap: () {
                    // Could open browser to archive.org
                  },
                ),
                
                const SizedBox(height: 16),
                
                // Reset Settings
                Padding(
                  padding: const EdgeInsets.symmetric(horizontal: 16),
                  child: OutlinedButton.icon(
                    onPressed: _showResetDialog,
                    icon: const Icon(Icons.restore),
                    label: const Text('Reset to Defaults'),
                  ),
                ),
                
                const SizedBox(height: 32),
              ],
            ),
    );
  }
  
  Widget _buildSectionHeader(String title) {
    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 16, 16, 8),
      child: Text(
        title,
        style: TextStyle(
          color: Theme.of(context).primaryColor,
          fontWeight: FontWeight.bold,
          fontSize: 14,
        ),
      ),
    );
  }
  
  void _showDownloadPathDialog() {
    final controller = TextEditingController(text: _downloadPath);
    
    showDialog(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('Download Location'),
        content: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            TextField(
              controller: controller,
              decoration: const InputDecoration(
                labelText: 'Path',
                hintText: '/storage/emulated/0/Download/ia-get',
                border: OutlineInputBorder(),
              ),
            ),
            const SizedBox(height: 8),
            Text(
              'Downloads will be saved to this directory',
              style: TextStyle(
                fontSize: 12,
                color: Colors.grey.shade600,
              ),
            ),
          ],
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context),
            child: const Text('Cancel'),
          ),
          ElevatedButton(
            onPressed: () {
              final path = controller.text.trim();
              if (path.isNotEmpty) {
                _saveDownloadPath(path);
                Navigator.pop(context);
              }
            },
            child: const Text('Save'),
          ),
        ],
      ),
    );
  }
  
  void _showResetDialog() {
    showDialog(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('Reset Settings'),
        content: const Text(
          'Are you sure you want to reset all settings to their default values?',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context),
            child: const Text('Cancel'),
          ),
          ElevatedButton(
            onPressed: () async {
              await _prefs.clear();
              await _loadSettings();
              if (mounted) {
                Navigator.pop(context);
                ScaffoldMessenger.of(context).showSnackBar(
                  const SnackBar(
                    content: Text('Settings reset to defaults'),
                  ),
                );
              }
            },
            child: const Text('Reset'),
          ),
        ],
      ),
    );
  }
}
