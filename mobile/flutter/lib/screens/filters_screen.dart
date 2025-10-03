import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../services/ia_get_service.dart';

class FiltersScreen extends StatefulWidget {
  final List<String> initialIncludeFormats;
  final List<String> initialExcludeFormats;
  final String? initialMaxSize;

  const FiltersScreen({
    super.key,
    this.initialIncludeFormats = const [],
    this.initialExcludeFormats = const [],
    this.initialMaxSize,
  });

  @override
  State<FiltersScreen> createState() => _FiltersScreenState();
}

class _FiltersScreenState extends State<FiltersScreen> {
  late List<String> _selectedIncludeFormats;
  late List<String> _selectedExcludeFormats;
  late String? _maxSize;

  // Will be populated from available formats in the archive
  List<String> _availableFormats = [];

  final List<String> _sizeOptions = [
    '10MB', '50MB', '100MB', '500MB', '1GB', '5GB', '10GB'
  ];

  @override
  void initState() {
    super.initState();
    _selectedIncludeFormats = List.from(widget.initialIncludeFormats);
    _selectedExcludeFormats = List.from(widget.initialExcludeFormats);
    _maxSize = widget.initialMaxSize;
    
    // Load available formats from the current archive
    WidgetsBinding.instance.addPostFrameCallback((_) {
      final service = context.read<IaGetService>();
      final formats = service.getAvailableFormats();
      setState(() {
        _availableFormats = formats.toList()..sort();
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
          
          // Include formats section
          _buildSectionHeader('Include Formats'),
          const SizedBox(height: 8),
          Text(
            _availableFormats.isEmpty 
                ? 'Loading available formats...'
                : 'Show only these file formats (${_availableFormats.length} available)',
            style: const TextStyle(
              fontSize: 12,
              color: Colors.grey,
            ),
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
          _buildSectionHeader('Exclude Formats'),
          const SizedBox(height: 8),
          const Text(
            'Hide these file formats',
            style: TextStyle(
              fontSize: 12,
              color: Colors.grey,
            ),
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
            style: TextStyle(
              fontSize: 12,
              color: Colors.grey,
            ),
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
              ..._sizeOptions.map((size) => DropdownMenuItem<String>(
                value: size,
                child: Text(size),
              )),
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
              color: Colors.black.withOpacity(0.1),
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
                  padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
                  decoration: BoxDecoration(
                    color: Colors.blue.shade100,
                    borderRadius: BorderRadius.circular(20),
                  ),
                  child: Row(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      Icon(Icons.filter_list, size: 16, color: Colors.blue.shade700),
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
                  padding: const EdgeInsets.symmetric(horizontal: 24, vertical: 12),
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
      style: const TextStyle(
        fontSize: 18,
        fontWeight: FontWeight.bold,
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

  void _clearAllFilters() {
    setState(() {
      _selectedIncludeFormats.clear();
      _selectedExcludeFormats.clear();
      _maxSize = null;
    });
  }

  void _applyFilters() {
    final service = context.read<IaGetService>();
    service.filterFiles(
      includeFormats: _selectedIncludeFormats.isNotEmpty
          ? _selectedIncludeFormats
          : null,
      excludeFormats: _selectedExcludeFormats.isNotEmpty
          ? _selectedExcludeFormats
          : null,
      maxSize: _maxSize,
    );
    
    // Return the filter state to the caller
    Navigator.pop(context, {
      'includeFormats': _selectedIncludeFormats,
      'excludeFormats': _selectedExcludeFormats,
      'maxSize': _maxSize,
    });
  }
}
