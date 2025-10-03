import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../models/archive_metadata.dart';
import '../services/ia_get_service.dart';
import '../screens/file_preview_screen.dart';
import '../screens/filters_screen.dart';

class FileListWidget extends StatefulWidget {
  final List<ArchiveFile> files;

  const FileListWidget({
    super.key,
    required this.files,
  });

  @override
  State<FileListWidget> createState() => _FileListWidgetState();
}

class _FileListWidgetState extends State<FileListWidget> {
  bool _selectAll = false;
  String _sortBy = 'name'; // name, size, format
  bool _sortAscending = true;
  
  // Filter state
  List<String> _selectedIncludeFormats = [];
  List<String> _selectedExcludeFormats = [];
  String? _maxSize;

  @override
  Widget build(BuildContext context) {
    final sortedFiles = _getSortedFiles();
    final selectedCount = sortedFiles.where((f) => f.selected).length;
    final totalSize = _calculateSelectedSize(sortedFiles);

    return Column(
      children: [
        // List controls
        Container(
          padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 8),
          decoration: BoxDecoration(
            color: Theme.of(context).colorScheme.surface,
            border: Border(
              bottom: BorderSide(
                color: Colors.grey.shade300,
                width: 1,
              ),
            ),
          ),
          child: Row(
            children: [
              // Select all checkbox
              Checkbox(
                value: _selectAll,
                onChanged: (value) {
                  setState(() {
                    _selectAll = value ?? false;
                    for (var file in sortedFiles) {
                      file.selected = _selectAll;
                    }
                  });
                  // Notify service that selection changed
                  context.read<IaGetService>().notifyFileSelectionChanged();
                },
              ),
              Text(
                _selectAll ? 'Deselect All' : 'Select All',
                style: const TextStyle(fontWeight: FontWeight.w500),
              ),
              
              const Spacer(),
              
              // Filter button with badge
              Stack(
                clipBehavior: Clip.none,
                children: [
                  IconButton(
                    icon: const Icon(Icons.filter_list, size: 20),
                    onPressed: _openFiltersScreen,
                    tooltip: 'Filter files',
                  ),
                  if (_hasActiveFilters())
                    Positioned(
                      right: 4,
                      top: 4,
                      child: Container(
                        padding: const EdgeInsets.all(4),
                        decoration: BoxDecoration(
                          color: Colors.red,
                          shape: BoxShape.circle,
                        ),
                        constraints: const BoxConstraints(
                          minWidth: 16,
                          minHeight: 16,
                        ),
                        child: Center(
                          child: Text(
                            '${_getActiveFilterCount()}',
                            style: const TextStyle(
                              color: Colors.white,
                              fontSize: 9,
                              fontWeight: FontWeight.bold,
                            ),
                          ),
                        ),
                      ),
                    ),
                ],
              ),
              
              const SizedBox(width: 4),
              
              // Sort dropdown
              PopupMenuButton<String>(
                icon: Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    const Icon(Icons.sort, size: 18),
                    Icon(
                      _sortAscending ? Icons.arrow_upward : Icons.arrow_downward,
                      size: 14,
                    ),
                  ],
                ),
                tooltip: 'Sort files',
                onSelected: (value) {
                  setState(() {
                    if (_sortBy == value) {
                      _sortAscending = !_sortAscending;
                    } else {
                      _sortBy = value;
                      _sortAscending = true;
                    }
                  });
                },
                itemBuilder: (context) => [
                  const PopupMenuItem(
                    value: 'name',
                    child: Row(
                      children: [
                        Icon(Icons.sort_by_alpha),
                        SizedBox(width: 8),
                        Text('Sort by Name'),
                      ],
                    ),
                  ),
                  const PopupMenuItem(
                    value: 'size',
                    child: Row(
                      children: [
                        Icon(Icons.storage),
                        SizedBox(width: 8),
                        Text('Sort by Size'),
                      ],
                    ),
                  ),
                  const PopupMenuItem(
                    value: 'format',
                    child: Row(
                      children: [
                        Icon(Icons.category),
                        SizedBox(width: 8),
                        Text('Sort by Format'),
                      ],
                    ),
                  ),
                ],
              ),
            ],
          ),
        ),

        // Selection summary
        if (selectedCount > 0)
          Container(
            width: double.infinity,
            padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 8),
            color: Theme.of(context).primaryColor.withOpacity(0.1),
            child: Text(
              '$selectedCount files selected • ${_formatSize(totalSize)}',
              style: TextStyle(
                color: Theme.of(context).primaryColor,
                fontWeight: FontWeight.w500,
              ),
            ),
          ),

        // File list
        Expanded(
          child: sortedFiles.isEmpty
              ? const Center(
                  child: Column(
                    mainAxisAlignment: MainAxisAlignment.center,
                    children: [
                      Icon(
                        Icons.filter_list_off,
                        size: 48,
                        color: Colors.grey,
                      ),
                      SizedBox(height: 16),
                      Text(
                        'No files match the current filters',
                        style: TextStyle(
                          fontSize: 16,
                          color: Colors.grey,
                        ),
                      ),
                    ],
                  ),
                )
              : ListView.builder(
                  itemCount: sortedFiles.length,
                  itemBuilder: (context, index) {
                    return _buildFileItem(sortedFiles[index]);
                  },
                ),
        ),
      ],
    );
  }

  Widget _buildFileItem(ArchiveFile file) {
    return CheckboxListTile(
      value: file.selected,
      onChanged: (selected) {
        setState(() {
          file.selected = selected ?? false;
          _updateSelectAllState();
        });
        // Notify service that selection changed
        context.read<IaGetService>().notifyFileSelectionChanged();
      },
      title: Text(
        file.displayName,
        maxLines: 2,
        overflow: TextOverflow.ellipsis,
      ),
      subtitle: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              if (file.format != null) ...[
                Container(
                  padding: const EdgeInsets.symmetric(
                    horizontal: 6,
                    vertical: 2,
                  ),
                  decoration: BoxDecoration(
                    color: _getFormatColor(file.format!),
                    borderRadius: BorderRadius.circular(4),
                  ),
                  child: Text(
                    file.format!.toUpperCase(),
                    style: const TextStyle(
                      fontSize: 10,
                      fontWeight: FontWeight.bold,
                      color: Colors.white,
                    ),
                  ),
                ),
                const SizedBox(width: 8),
              ],
              Text(
                file.sizeFormatted,
                style: TextStyle(
                  fontSize: 12,
                  color: Colors.grey.shade600,
                ),
              ),
            ],
          ),
          if (file.name != file.displayName)
            Text(
              file.name,
              style: TextStyle(
                fontSize: 10,
                color: Colors.grey.shade500,
              ),
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
            ),
        ],
      ),
      secondary: PopupMenuButton<String>(
        icon: const Icon(Icons.more_vert),
        onSelected: (action) => _handleFileAction(file, action),
        itemBuilder: (context) => [
          const PopupMenuItem(
            value: 'preview',
            child: Row(
              children: [
                Icon(Icons.preview),
                SizedBox(width: 8),
                Text('Preview'),
              ],
            ),
          ),
          const PopupMenuItem(
            value: 'info',
            child: Row(
              children: [
                Icon(Icons.info),
                SizedBox(width: 8),
                Text('File Info'),
              ],
            ),
          ),
          if (file.md5 != null || file.sha1 != null)
            const PopupMenuItem(
              value: 'checksum',
              child: Row(
                children: [
                  Icon(Icons.fingerprint),
                  SizedBox(width: 8),
                  Text('Checksums'),
                ],
              ),
            ),
        ],
      ),
      dense: true,
      contentPadding: const EdgeInsets.symmetric(horizontal: 8),
    );
  }

  List<ArchiveFile> _getSortedFiles() {
    final files = List<ArchiveFile>.from(widget.files);
    
    files.sort((a, b) {
      int result = 0;
      
      switch (_sortBy) {
        case 'name':
          result = a.displayName.toLowerCase().compareTo(b.displayName.toLowerCase());
          break;
        case 'size':
          final aSize = a.size ?? 0;
          final bSize = b.size ?? 0;
          result = aSize.compareTo(bSize);
          break;
        case 'format':
          final aFormat = a.format ?? '';
          final bFormat = b.format ?? '';
          result = aFormat.compareTo(bFormat);
          break;
      }
      
      return _sortAscending ? result : -result;
    });
    
    return files;
  }

  void _updateSelectAllState() {
    final selectedCount = widget.files.where((f) => f.selected).length;
    setState(() {
      _selectAll = selectedCount == widget.files.length && widget.files.isNotEmpty;
    });
  }

  int _calculateSelectedSize(List<ArchiveFile> files) {
    return files
        .where((f) => f.selected)
        .map((f) => f.size ?? 0)
        .fold(0, (sum, size) => sum + size);
  }

  Color _getFormatColor(String format) {
    final formatLower = format.toLowerCase();
    
    // Document formats
    if (['pdf', 'doc', 'docx', 'txt', 'epub'].contains(formatLower)) {
      return Colors.blue;
    }
    // Image formats
    if (['jpg', 'jpeg', 'png', 'gif', 'bmp', 'svg'].contains(formatLower)) {
      return Colors.green;
    }
    // Video formats
    if (['mp4', 'avi', 'mov', 'mkv', 'webm'].contains(formatLower)) {
      return Colors.purple;
    }
    // Audio formats
    if (['mp3', 'wav', 'flac', 'aac', 'ogg'].contains(formatLower)) {
      return Colors.orange;
    }
    // Archive formats
    if (['zip', 'rar', '7z', 'tar', 'gz'].contains(formatLower)) {
      return Colors.brown;
    }
    
    return Colors.grey;
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

  void _handleFileAction(ArchiveFile file, String action) {
    switch (action) {
      case 'preview':
        _showFilePreview(file);
        break;
      case 'info':
        _showFileInfo(file);
        break;
      case 'checksum':
        _showChecksums(file);
        break;
    }
  }

  void _showFilePreview(ArchiveFile file) {
    Navigator.push(
      context,
      MaterialPageRoute(
        builder: (context) => FilePreviewScreen(file: file),
      ),
    );
  }

  void _showFileInfo(ArchiveFile file) {
    showDialog(
      context: context,
      builder: (context) => AlertDialog(
        title: Text(file.displayName),
        content: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            _buildInfoRow('Full Path', file.name),
            if (file.format != null) _buildInfoRow('Format', file.format!),
            _buildInfoRow('Size', file.sizeFormatted),
            if (file.downloadUrl != null) 
              _buildInfoRow('URL', file.downloadUrl!, isUrl: true),
          ],
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

  void _showChecksums(ArchiveFile file) {
    showDialog(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('File Checksums'),
        content: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            if (file.md5 != null) _buildInfoRow('MD5', file.md5!),
            if (file.sha1 != null) _buildInfoRow('SHA1', file.sha1!),
          ],
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

  Widget _buildInfoRow(String label, String value, {bool isUrl = false}) {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 4),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            label,
            style: const TextStyle(
              fontWeight: FontWeight.w500,
              fontSize: 12,
            ),
          ),
          const SizedBox(height: 2),
          SelectableText(
            value,
            style: TextStyle(
              fontSize: 12,
              color: isUrl ? Colors.blue : null,
              decoration: isUrl ? TextDecoration.underline : null,
            ),
          ),
        ],
      ),
    );
  }
  
  bool _hasActiveFilters() {
    return _selectedIncludeFormats.isNotEmpty ||
           _selectedExcludeFormats.isNotEmpty ||
           _maxSize != null;
  }

  int _getActiveFilterCount() {
    int count = 0;
    if (_selectedIncludeFormats.isNotEmpty) count++;
    if (_selectedExcludeFormats.isNotEmpty) count++;
    if (_maxSize != null) count++;
    return count;
  }
  
  void _openFiltersScreen() async {
    final result = await Navigator.push<Map<String, dynamic>>(
      context,
      MaterialPageRoute(
        builder: (context) => FiltersScreen(
          initialIncludeFormats: _selectedIncludeFormats,
          initialExcludeFormats: _selectedExcludeFormats,
          initialMaxSize: _maxSize,
        ),
      ),
    );
    
    // Update local state with returned filter values
    if (result != null && mounted) {
      setState(() {
        _selectedIncludeFormats = List<String>.from(result['includeFormats'] ?? []);
        _selectedExcludeFormats = List<String>.from(result['excludeFormats'] ?? []);
        _maxSize = result['maxSize'] as String?;
      });
    }
  }
}