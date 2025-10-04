import 'package:flutter/material.dart';
import '../models/file_filter.dart';

/// Advanced Filters Screen
/// 
/// Provides comprehensive filtering options including:
/// - Subfolder/path-based filtering
/// - Wildcard and regex patterns
/// - Size range filtering
/// - Format and source type filtering
class AdvancedFiltersScreen extends StatefulWidget {
  final FileFilter initialFilter;

  const AdvancedFiltersScreen({
    super.key,
    this.initialFilter = FileFilter.empty,
  });

  @override
  State<AdvancedFiltersScreen> createState() => _AdvancedFiltersScreenState();
}

class _AdvancedFiltersScreenState extends State<AdvancedFiltersScreen> {
  late List<String> _includePatterns;
  late List<String> _excludePatterns;
  late List<String> _includeSubfolders;
  late List<String> _excludeSubfolders;
  late List<String> _includeFormats;
  late List<String> _excludeFormats;
  late bool _includeOriginal;
  late bool _includeDerivative;
  late bool _includeMetadata;
  late bool _useRegex;
  
  int? _minSize;
  int? _maxSize;
  
  final _patternController = TextEditingController();
  final _subfolderController = TextEditingController();
  final _minSizeController = TextEditingController();
  final _maxSizeController = TextEditingController();

  @override
  void initState() {
    super.initState();
    final filter = widget.initialFilter;
    _includePatterns = List.from(filter.includePatterns);
    _excludePatterns = List.from(filter.excludePatterns);
    _includeSubfolders = List.from(filter.includeSubfolders);
    _excludeSubfolders = List.from(filter.excludeSubfolders);
    _includeFormats = List.from(filter.includeFormats);
    _excludeFormats = List.from(filter.excludeFormats);
    _minSize = filter.minSize;
    _maxSize = filter.maxSize;
    _includeOriginal = filter.includeOriginal;
    _includeDerivative = filter.includeDerivative;
    _includeMetadata = filter.includeMetadata;
    _useRegex = filter.useRegex;
    
    if (_minSize != null) {
      _minSizeController.text = (_minSize! / (1024 * 1024)).toStringAsFixed(0);
    }
    if (_maxSize != null) {
      _maxSizeController.text = (_maxSize! / (1024 * 1024)).toStringAsFixed(0);
    }
  }

  @override
  void dispose() {
    _patternController.dispose();
    _subfolderController.dispose();
    _minSizeController.dispose();
    _maxSizeController.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: const Text('Advanced Filters'),
        actions: [
          if (_hasActiveFilters())
            TextButton.icon(
              onPressed: _clearAll,
              icon: const Icon(Icons.clear_all, color: Colors.white),
              label: const Text('Clear All', style: TextStyle(color: Colors.white)),
            ),
        ],
      ),
      body: ListView(
        padding: const EdgeInsets.all(16),
        children: [
          // Info card
          Card(
            color: Colors.blue.shade50,
            child: Padding(
              padding: const EdgeInsets.all(12),
              child: Row(
                children: [
                  Icon(Icons.info_outline, color: Colors.blue.shade700),
                  const SizedBox(width: 8),
                  Expanded(
                    child: Text(
                      'Advanced filtering with subfolder, pattern, and size-based criteria',
                      style: TextStyle(color: Colors.blue.shade900, fontSize: 13),
                    ),
                  ),
                ],
              ),
            ),
          ),
          
          const SizedBox(height: 24),
          
          // Subfolder filtering
          _buildSectionHeader('Subfolder Filtering', Icons.folder),
          const SizedBox(height: 8),
          const Text(
            'Filter by folder paths (e.g., "data/", "images/*", "docs/2024")',
            style: TextStyle(fontSize: 12, color: Colors.grey),
          ),
          const SizedBox(height: 12),
          Row(
            children: [
              Expanded(
                child: TextField(
                  controller: _subfolderController,
                  decoration: const InputDecoration(
                    hintText: 'e.g., data/ or images/*',
                    border: OutlineInputBorder(),
                    prefixIcon: Icon(Icons.folder_open),
                  ),
                ),
              ),
              const SizedBox(width: 8),
              ElevatedButton(
                onPressed: () => _addSubfolder(true),
                child: const Text('Include'),
              ),
              const SizedBox(width: 8),
              ElevatedButton(
                onPressed: () => _addSubfolder(false),
                style: ElevatedButton.styleFrom(backgroundColor: Colors.red),
                child: const Text('Exclude'),
              ),
            ],
          ),
          const SizedBox(height: 12),
          if (_includeSubfolders.isNotEmpty) ...[
            const Text('Include folders:', style: TextStyle(fontWeight: FontWeight.w600)),
            const SizedBox(height: 4),
            Wrap(
              spacing: 8,
              runSpacing: 8,
              children: _includeSubfolders.map((folder) => Chip(
                label: Text(folder),
                deleteIcon: const Icon(Icons.close, size: 18),
                onDeleted: () => setState(() => _includeSubfolders.remove(folder)),
                backgroundColor: Colors.green.shade100,
              )).toList(),
            ),
            const SizedBox(height: 12),
          ],
          if (_excludeSubfolders.isNotEmpty) ...[
            const Text('Exclude folders:', style: TextStyle(fontWeight: FontWeight.w600)),
            const SizedBox(height: 4),
            Wrap(
              spacing: 8,
              runSpacing: 8,
              children: _excludeSubfolders.map((folder) => Chip(
                label: Text(folder),
                deleteIcon: const Icon(Icons.close, size: 18),
                onDeleted: () => setState(() => _excludeSubfolders.remove(folder)),
                backgroundColor: Colors.red.shade100,
              )).toList(),
            ),
            const SizedBox(height: 12),
          ],
          
          const SizedBox(height: 24),
          const Divider(),
          const SizedBox(height: 24),
          
          // File pattern filtering
          _buildSectionHeader('File Patterns', Icons.text_fields),
          const SizedBox(height: 8),
          Row(
            children: [
              const Text('Use Regex Patterns:', style: TextStyle(fontSize: 14)),
              const SizedBox(width: 8),
              Switch(
                value: _useRegex,
                onChanged: (value) => setState(() => _useRegex = value),
              ),
            ],
          ),
          const SizedBox(height: 8),
          Text(
            _useRegex 
              ? 'Enter regex patterns (e.g., "^data_\\d+\\.txt\$")'
              : 'Enter wildcard patterns (e.g., "*.pdf", "chapter*", "data_?.txt")',
            style: const TextStyle(fontSize: 12, color: Colors.grey),
          ),
          const SizedBox(height: 12),
          Row(
            children: [
              Expanded(
                child: TextField(
                  controller: _patternController,
                  decoration: InputDecoration(
                    hintText: _useRegex ? 'e.g., ^.*\\.pdf\$' : 'e.g., *.pdf',
                    border: const OutlineInputBorder(),
                    prefixIcon: const Icon(Icons.filter_alt),
                  ),
                ),
              ),
              const SizedBox(width: 8),
              ElevatedButton(
                onPressed: () => _addPattern(true),
                child: const Text('Include'),
              ),
              const SizedBox(width: 8),
              ElevatedButton(
                onPressed: () => _addPattern(false),
                style: ElevatedButton.styleFrom(backgroundColor: Colors.red),
                child: const Text('Exclude'),
              ),
            ],
          ),
          const SizedBox(height: 12),
          if (_includePatterns.isNotEmpty) ...[
            const Text('Include patterns:', style: TextStyle(fontWeight: FontWeight.w600)),
            const SizedBox(height: 4),
            Wrap(
              spacing: 8,
              runSpacing: 8,
              children: _includePatterns.map((pattern) => Chip(
                label: Text(pattern),
                deleteIcon: const Icon(Icons.close, size: 18),
                onDeleted: () => setState(() => _includePatterns.remove(pattern)),
                backgroundColor: Colors.green.shade100,
              )).toList(),
            ),
            const SizedBox(height: 12),
          ],
          if (_excludePatterns.isNotEmpty) ...[
            const Text('Exclude patterns:', style: TextStyle(fontWeight: FontWeight.w600)),
            const SizedBox(height: 4),
            Wrap(
              spacing: 8,
              runSpacing: 8,
              children: _excludePatterns.map((pattern) => Chip(
                label: Text(pattern),
                deleteIcon: const Icon(Icons.close, size: 18),
                onDeleted: () => setState(() => _excludePatterns.remove(pattern)),
                backgroundColor: Colors.red.shade100,
              )).toList(),
            ),
            const SizedBox(height: 12),
          ],
          
          const SizedBox(height: 24),
          const Divider(),
          const SizedBox(height: 24),
          
          // Size filtering
          _buildSectionHeader('Size Range', Icons.storage),
          const SizedBox(height: 8),
          const Text(
            'Filter by file size (in MB)',
            style: TextStyle(fontSize: 12, color: Colors.grey),
          ),
          const SizedBox(height: 12),
          Row(
            children: [
              Expanded(
                child: TextField(
                  controller: _minSizeController,
                  keyboardType: TextInputType.number,
                  decoration: const InputDecoration(
                    labelText: 'Min Size (MB)',
                    border: OutlineInputBorder(),
                    prefixIcon: Icon(Icons.arrow_upward),
                  ),
                  onChanged: (value) {
                    final size = int.tryParse(value);
                    _minSize = size != null ? size * 1024 * 1024 : null;
                  },
                ),
              ),
              const Padding(
                padding: EdgeInsets.symmetric(horizontal: 16),
                child: Text('to', style: TextStyle(fontWeight: FontWeight.bold)),
              ),
              Expanded(
                child: TextField(
                  controller: _maxSizeController,
                  keyboardType: TextInputType.number,
                  decoration: const InputDecoration(
                    labelText: 'Max Size (MB)',
                    border: OutlineInputBorder(),
                    prefixIcon: Icon(Icons.arrow_downward),
                  ),
                  onChanged: (value) {
                    final size = int.tryParse(value);
                    _maxSize = size != null ? size * 1024 * 1024 : null;
                  },
                ),
              ),
            ],
          ),
          
          const SizedBox(height: 24),
          const Divider(),
          const SizedBox(height: 24),
          
          // Source type filtering
          _buildSectionHeader('Source Type', Icons.source),
          const SizedBox(height: 12),
          Wrap(
            spacing: 8,
            runSpacing: 8,
            children: [
              FilterChip(
                label: const Text('Original'),
                selected: _includeOriginal,
                onSelected: (val) => setState(() => _includeOriginal = val),
                selectedColor: Colors.green.shade200,
              ),
              FilterChip(
                label: const Text('Derivative'),
                selected: _includeDerivative,
                onSelected: (val) => setState(() => _includeDerivative = val),
                selectedColor: Colors.orange.shade200,
              ),
              FilterChip(
                label: const Text('Metadata'),
                selected: _includeMetadata,
                onSelected: (val) => setState(() => _includeMetadata = val),
                selectedColor: Colors.purple.shade200,
              ),
            ],
          ),
          
          const SizedBox(height: 32),
        ],
      ),
      bottomNavigationBar: Container(
        padding: const EdgeInsets.all(16),
        decoration: BoxDecoration(
          color: Theme.of(context).colorScheme.surface,
          boxShadow: [
            BoxShadow(
              color: Colors.black.withValues(alpha: 0.1),
              blurRadius: 4,
              offset: const Offset(0, -2),
            ),
          ],
        ),
        child: SafeArea(
          child: Row(
            children: [
              if (_hasActiveFilters())
                Container(
                  padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
                  decoration: BoxDecoration(
                    color: Colors.blue.shade100,
                    borderRadius: BorderRadius.circular(20),
                  ),
                  child: Text(
                    '${_getActiveFilterCount()} active',
                    style: TextStyle(
                      color: Colors.blue.shade900,
                      fontWeight: FontWeight.w500,
                    ),
                  ),
                ),
              const Spacer(),
              ElevatedButton.icon(
                onPressed: _apply,
                icon: const Icon(Icons.check),
                label: const Text('Apply Filters'),
                style: ElevatedButton.styleFrom(
                  padding: const EdgeInsets.symmetric(horizontal: 24, vertical: 12),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }

  Widget _buildSectionHeader(String title, IconData icon) {
    return Row(
      children: [
        Icon(icon, size: 20),
        const SizedBox(width: 8),
        Text(
          title,
          style: const TextStyle(fontSize: 18, fontWeight: FontWeight.bold),
        ),
      ],
    );
  }

  void _addPattern(bool include) {
    final pattern = _patternController.text.trim();
    if (pattern.isEmpty) return;
    
    setState(() {
      if (include) {
        if (!_includePatterns.contains(pattern)) {
          _includePatterns.add(pattern);
        }
        _excludePatterns.remove(pattern);
      } else {
        if (!_excludePatterns.contains(pattern)) {
          _excludePatterns.add(pattern);
        }
        _includePatterns.remove(pattern);
      }
      _patternController.clear();
    });
  }

  void _addSubfolder(bool include) {
    final subfolder = _subfolderController.text.trim();
    if (subfolder.isEmpty) return;
    
    setState(() {
      if (include) {
        if (!_includeSubfolders.contains(subfolder)) {
          _includeSubfolders.add(subfolder);
        }
        _excludeSubfolders.remove(subfolder);
      } else {
        if (!_excludeSubfolders.contains(subfolder)) {
          _excludeSubfolders.add(subfolder);
        }
        _includeSubfolders.remove(subfolder);
      }
      _subfolderController.clear();
    });
  }

  bool _hasActiveFilters() {
    return _includePatterns.isNotEmpty ||
        _excludePatterns.isNotEmpty ||
        _includeSubfolders.isNotEmpty ||
        _excludeSubfolders.isNotEmpty ||
        _minSize != null ||
        _maxSize != null ||
        !_includeOriginal ||
        !_includeDerivative ||
        !_includeMetadata;
  }

  int _getActiveFilterCount() {
    int count = 0;
    if (_includePatterns.isNotEmpty) count++;
    if (_excludePatterns.isNotEmpty) count++;
    if (_includeSubfolders.isNotEmpty) count++;
    if (_excludeSubfolders.isNotEmpty) count++;
    if (_minSize != null || _maxSize != null) count++;
    if (!_includeOriginal || !_includeDerivative || !_includeMetadata) count++;
    return count;
  }

  void _clearAll() {
    setState(() {
      _includePatterns.clear();
      _excludePatterns.clear();
      _includeSubfolders.clear();
      _excludeSubfolders.clear();
      _minSize = null;
      _maxSize = null;
      _minSizeController.clear();
      _maxSizeController.clear();
      _includeOriginal = true;
      _includeDerivative = true;
      _includeMetadata = true;
      _useRegex = false;
    });
  }

  void _apply() {
    final filter = FileFilter(
      includePatterns: _includePatterns,
      excludePatterns: _excludePatterns,
      includeSubfolders: _includeSubfolders,
      excludeSubfolders: _excludeSubfolders,
      includeFormats: _includeFormats,
      excludeFormats: _excludeFormats,
      minSize: _minSize,
      maxSize: _maxSize,
      includeOriginal: _includeOriginal,
      includeDerivative: _includeDerivative,
      includeMetadata: _includeMetadata,
      useRegex: _useRegex,
    );
    
    Navigator.pop(context, filter);
  }
}
