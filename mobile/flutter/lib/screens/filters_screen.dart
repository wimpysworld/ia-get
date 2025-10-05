import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../services/archive_service.dart';

class FiltersScreen extends StatefulWidget {
  final List<String> initialIncludeFormats;
  final List<String> initialExcludeFormats;
  final String? initialMaxSize;
  final bool initialIncludeOriginal;
  final bool initialIncludeDerivative;
  final bool initialIncludeMetadata;

  const FiltersScreen({
    super.key,
    this.initialIncludeFormats = const [],
    this.initialExcludeFormats = const [],
    this.initialMaxSize,
    this.initialIncludeOriginal = true,
    this.initialIncludeDerivative = true,
    this.initialIncludeMetadata = true,
  });

  @override
  State<FiltersScreen> createState() => _FiltersScreenState();
}

class _FiltersScreenState extends State<FiltersScreen> {
  late List<String> _selectedIncludeFormats;
  late List<String> _selectedExcludeFormats;
  late String? _maxSize;

  // Source type filtering
  bool _includeOriginal = true;
  bool _includeDerivative = true;
  bool _includeMetadata = true;

  // Will be populated from available formats in the archive
  List<String> _availableFormats = [];

  final List<String> _sizeOptions = [
    '10MB',
    '50MB',
    '100MB',
    '500MB',
    '1GB',
    '5GB',
    '10GB',
  ];

  @override
  void initState() {
    super.initState();
    _selectedIncludeFormats = List.from(widget.initialIncludeFormats);
    _selectedExcludeFormats = List.from(widget.initialExcludeFormats);
    _maxSize = widget.initialMaxSize;
    _includeOriginal = widget.initialIncludeOriginal;
    _includeDerivative = widget.initialIncludeDerivative;
    _includeMetadata = widget.initialIncludeMetadata;

    // Load available formats from the current archive
    WidgetsBinding.instance.addPostFrameCallback((_) {
      final service = context.read<ArchiveService>();
      final formats = service.getAvailableFormats();
      setState(() {
        _availableFormats = List<String>.from(formats)..sort();
      });
    });
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: const Text('Filter Files'),
        actions: [
          // Clear all button
          if (_hasActiveFilters())
            TextButton.icon(
              onPressed: _clearAllFilters,
              icon: const Icon(Icons.clear_all, color: Colors.white),
              label: const Text(
                'Clear All',
                style: TextStyle(color: Colors.white),
              ),
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
                      'Select filters to refine your file selection',
                      style: TextStyle(
                        color: Colors.blue.shade900,
                        fontSize: 14,
                      ),
                    ),
                  ),
                ],
              ),
            ),
          ),

          const SizedBox(height: 24),

          // Source Type filtering section
          _buildSectionHeader('Content Source Type'),
          const SizedBox(height: 8),
          const Text(
            'Filter by where files originate from',
            style: TextStyle(fontSize: 12, color: Colors.grey),
          ),
          const SizedBox(height: 12),
          Wrap(
            spacing: 8,
            runSpacing: 8,
            children: [
              FilterChip(
                label: const Text('ORIGINAL'),
                selected: _includeOriginal,
                onSelected: (selected) {
                  setState(() {
                    _includeOriginal = selected;
                  });
                },
                selectedColor: Colors.green.shade200,
                checkmarkColor: Colors.green.shade700,
                avatar: _includeOriginal
                    ? null
                    : const Icon(Icons.upload_file, size: 18),
              ),
              FilterChip(
                label: const Text('DERIVATIVE'),
                selected: _includeDerivative,
                onSelected: (selected) {
                  setState(() {
                    _includeDerivative = selected;
                  });
                },
                selectedColor: Colors.orange.shade200,
                checkmarkColor: Colors.orange.shade700,
                avatar: _includeDerivative
                    ? null
                    : const Icon(Icons.auto_awesome, size: 18),
              ),
              FilterChip(
                label: const Text('METADATA'),
                selected: _includeMetadata,
                onSelected: (selected) {
                  setState(() {
                    _includeMetadata = selected;
                  });
                },
                selectedColor: Colors.purple.shade200,
                checkmarkColor: Colors.purple.shade700,
                avatar: _includeMetadata
                    ? null
                    : const Icon(Icons.info, size: 18),
              ),
            ],
          ),

          const SizedBox(height: 16),
          Text(
            '• Original: Files uploaded by users\n'
            '• Derivative: Generated versions (e.g., lower quality)\n'
            '• Metadata: Archive-generated metadata files',
            style: TextStyle(
              fontSize: 11,
              color: Colors.grey.shade600,
              fontStyle: FontStyle.italic,
            ),
          ),

          const SizedBox(height: 24),
          const Divider(),
          const SizedBox(height: 24),

          // Include formats section
          _buildSectionHeader('File Type Filters'),
          _buildSectionSubheader('Include Formats'),
          const SizedBox(height: 8),
          Text(
            _availableFormats.isEmpty
                ? 'Loading available formats...'
                : 'Show only these file formats (${_availableFormats.length} available)',
            style: const TextStyle(fontSize: 12, color: Colors.grey),
          ),
          const SizedBox(height: 12),
          if (_availableFormats.isEmpty)
            const Center(
              child: Padding(
                padding: EdgeInsets.all(16.0),
                child: CircularProgressIndicator(),
              ),
            )
          else
            Wrap(
              spacing: 8,
              runSpacing: 8,
              children: _availableFormats.map((format) {
                final isSelected = _selectedIncludeFormats.contains(format);
                return FilterChip(
                  label: Text(format.toUpperCase()),
                  selected: isSelected,
                  onSelected: (selected) {
                    setState(() {
                      if (selected) {
                        _selectedIncludeFormats.add(format);
                        _selectedExcludeFormats.remove(format);
                      } else {
                        _selectedIncludeFormats.remove(format);
                      }
                    });
                  },
                  selectedColor: Colors.blue.shade200,
                  checkmarkColor: Colors.blue.shade700,
                );
              }).toList(),
            ),

          const SizedBox(height: 24),
          const Divider(),
          const SizedBox(height: 24),

          // Exclude formats section
          _buildSectionSubheader('Exclude Formats'),
          const SizedBox(height: 8),
          const Text(
            'Hide these file formats',
            style: TextStyle(fontSize: 12, color: Colors.grey),
          ),
          const SizedBox(height: 12),
          if (_availableFormats.isEmpty)
            const Center(
              child: Padding(
                padding: EdgeInsets.all(16.0),
                child: CircularProgressIndicator(),
              ),
            )
          else
            Wrap(
              spacing: 8,
              runSpacing: 8,
              children: _availableFormats.map((format) {
                final isSelected = _selectedExcludeFormats.contains(format);
                return FilterChip(
                  label: Text(format.toUpperCase()),
                  selected: isSelected,
                  onSelected: (selected) {
                    setState(() {
                      if (selected) {
                        _selectedExcludeFormats.add(format);
                        _selectedIncludeFormats.remove(format);
                      } else {
                        _selectedExcludeFormats.remove(format);
                      }
                    });
                  },
                  selectedColor: Colors.red.shade200,
                  checkmarkColor: Colors.red.shade700,
                );
              }).toList(),
            ),

          const SizedBox(height: 24),
          const Divider(),
          const SizedBox(height: 24),

          // Max file size section
          _buildSectionHeader('Maximum File Size'),
          const SizedBox(height: 8),
          const Text(
            'Show only files smaller than this size',
            style: TextStyle(fontSize: 12, color: Colors.grey),
          ),
          const SizedBox(height: 12),
          DropdownButtonFormField<String>(
            value: _maxSize,
            decoration: const InputDecoration(
              hintText: 'No limit',
              border: OutlineInputBorder(),
              prefixIcon: Icon(Icons.storage),
            ),
            items: [
              const DropdownMenuItem<String>(
                value: null,
                child: Text('No limit'),
              ),
              ..._sizeOptions.map(
                (size) =>
                    DropdownMenuItem<String>(value: size, child: Text(size)),
              ),
            ],
            onChanged: (value) {
              setState(() {
                _maxSize = value;
              });
            },
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
              // Active filters count
              if (_hasActiveFilters())
                Container(
                  padding: const EdgeInsets.symmetric(
                    horizontal: 12,
                    vertical: 8,
                  ),
                  decoration: BoxDecoration(
                    color: Colors.blue.shade100,
                    borderRadius: BorderRadius.circular(20),
                  ),
                  child: Row(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      Icon(
                        Icons.filter_list,
                        size: 16,
                        color: Colors.blue.shade700,
                      ),
                      const SizedBox(width: 4),
                      Text(
                        '${_getActiveFilterCount()} active',
                        style: TextStyle(
                          color: Colors.blue.shade900,
                          fontWeight: FontWeight.w500,
                        ),
                      ),
                    ],
                  ),
                ),

              const Spacer(),

              // Apply button
              ElevatedButton.icon(
                onPressed: _applyFilters,
                icon: const Icon(Icons.check),
                label: const Text('Apply Filters'),
                style: ElevatedButton.styleFrom(
                  padding: const EdgeInsets.symmetric(
                    horizontal: 24,
                    vertical: 12,
                  ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }

  Widget _buildSectionHeader(String title) {
    return Text(
      title,
      style: const TextStyle(fontSize: 18, fontWeight: FontWeight.bold),
    );
  }

  Widget _buildSectionSubheader(String title) {
    return Text(
      title,
      style: const TextStyle(fontSize: 16, fontWeight: FontWeight.w600),
    );
  }

  bool _hasActiveFilters() {
    return _selectedIncludeFormats.isNotEmpty ||
        _selectedExcludeFormats.isNotEmpty ||
        _maxSize != null ||
        !_includeOriginal ||
        !_includeDerivative ||
        !_includeMetadata;
  }

  int _getActiveFilterCount() {
    int count = 0;
    if (_selectedIncludeFormats.isNotEmpty) count++;
    if (_selectedExcludeFormats.isNotEmpty) count++;
    if (_maxSize != null) count++;
    if (!_includeOriginal || !_includeDerivative || !_includeMetadata) count++;
    return count;
  }

  void _clearAllFilters() {
    setState(() {
      _selectedIncludeFormats.clear();
      _selectedExcludeFormats.clear();
      _maxSize = null;
      _includeOriginal = true;
      _includeDerivative = true;
      _includeMetadata = true;
    });
  }

  void _applyFilters() {
    final service = context.read<ArchiveService>();
    service.filterFiles(
      includeFormats: _selectedIncludeFormats.isNotEmpty
          ? _selectedIncludeFormats
          : null,
      excludeFormats: _selectedExcludeFormats.isNotEmpty
          ? _selectedExcludeFormats
          : null,
      maxSize: _maxSize,
      includeOriginal: _includeOriginal,
      includeDerivative: _includeDerivative,
      includeMetadata: _includeMetadata,
    );

    // Return the filter state to the caller
    Navigator.pop(context, {
      'includeFormats': _selectedIncludeFormats,
      'excludeFormats': _selectedExcludeFormats,
      'maxSize': _maxSize,
      'includeOriginal': _includeOriginal,
      'includeDerivative': _includeDerivative,
      'includeMetadata': _includeMetadata,
    });
  }
}
