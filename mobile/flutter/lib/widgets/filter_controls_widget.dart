import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../services/ia_get_service.dart';

class FilterControlsWidget extends StatefulWidget {
  const FilterControlsWidget({super.key});

  @override
  State<FilterControlsWidget> createState() => _FilterControlsWidgetState();
}

class _FilterControlsWidgetState extends State<FilterControlsWidget> {
  final List<String> _selectedIncludeFormats = [];
  final List<String> _selectedExcludeFormats = [];
  String? _maxSize;

  final List<String> _commonFormats = [
    'pdf', 'epub', 'txt', 'mp3', 'mp4', 'avi', 
    'jpg', 'png', 'gif', 'zip', 'rar', 'iso'
  ];

  final List<String> _sizeOptions = [
    '10MB', '50MB', '100MB', '500MB', '1GB', '5GB'
  ];

  @override
  Widget build(BuildContext context) {
    return Card(
      margin: const EdgeInsets.symmetric(horizontal: 16),
      child: ExpansionTile(
        leading: const Icon(Icons.filter_list),
        title: const Text('Filter Files'),
        subtitle: _hasActiveFilters()
            ? Text('${_getActiveFilterCount()} filters active')
            : const Text('Tap to filter files'),
        children: [
          Padding(
            padding: const EdgeInsets.all(16),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                // Include formats
                const Text(
                  'Include Formats',
                  style: TextStyle(fontWeight: FontWeight.w500),
                ),
                const SizedBox(height: 8),
                Wrap(
                  spacing: 8,
                  runSpacing: 4,
                  children: _commonFormats.map((format) {
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
                        _applyFilters();
                      },
                    );
                  }).toList(),
                ),
                
                const SizedBox(height: 16),
                
                // Exclude formats
                const Text(
                  'Exclude Formats',
                  style: TextStyle(fontWeight: FontWeight.w500),
                ),
                const SizedBox(height: 8),
                Wrap(
                  spacing: 8,
                  runSpacing: 4,
                  children: _commonFormats.map((format) {
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
                        _applyFilters();
                      },
                      selectedColor: Colors.red.shade200,
                      checkmarkColor: Colors.red.shade700,
                    );
                  }).toList(),
                ),
                
                const SizedBox(height: 16),
                
                // Max file size
                const Text(
                  'Maximum File Size',
                  style: TextStyle(fontWeight: FontWeight.w500),
                ),
                const SizedBox(height: 8),
                DropdownButtonFormField<String>(
                  value: _maxSize,
                  decoration: const InputDecoration(
                    hintText: 'No limit',
                    border: OutlineInputBorder(),
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
                    _applyFilters();
                  },
                ),
                
                const SizedBox(height: 16),
                
                // Clear filters button
                if (_hasActiveFilters())
                  SizedBox(
                    width: double.infinity,
                    child: OutlinedButton(
                      onPressed: _clearFilters,
                      child: const Text('Clear All Filters'),
                    ),
                  ),
              ],
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

  void _applyFilters() {
    context.read<IaGetService>().filterFiles(
      includeFormats: _selectedIncludeFormats.isNotEmpty
          ? _selectedIncludeFormats
          : null,
      excludeFormats: _selectedExcludeFormats.isNotEmpty
          ? _selectedExcludeFormats
          : null,
      maxSize: _maxSize,
    );
  }

  void _clearFilters() {
    setState(() {
      _selectedIncludeFormats.clear();
      _selectedExcludeFormats.clear();
      _maxSize = null;
    });
    _applyFilters();
  }
}