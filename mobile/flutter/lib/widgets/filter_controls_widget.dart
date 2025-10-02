import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../services/ia_get_service.dart';
import '../screens/filters_screen.dart';

class FilterControlsWidget extends StatefulWidget {
  const FilterControlsWidget({super.key});

  @override
  State<FilterControlsWidget> createState() => _FilterControlsWidgetState();
}

class _FilterControlsWidgetState extends State<FilterControlsWidget> {
  List<String> _selectedIncludeFormats = [];
  List<String> _selectedExcludeFormats = [];
  String? _maxSize;

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 8),
      child: Row(
        children: [
          // Filters button with badge
          Stack(
            children: [
              OutlinedButton.icon(
                onPressed: _openFiltersScreen,
                icon: const Icon(Icons.filter_list),
                label: const Text('Filters'),
                style: OutlinedButton.styleFrom(
                  padding: const EdgeInsets.symmetric(
                    horizontal: 16,
                    vertical: 12,
                  ),
                ),
              ),
              
              // Badge showing active filter count
              if (_hasActiveFilters())
                Positioned(
                  right: 0,
                  top: 0,
                  child: Container(
                    padding: const EdgeInsets.all(4),
                    decoration: BoxDecoration(
                      color: Colors.red,
                      shape: BoxShape.circle,
                      border: Border.all(
                        color: Theme.of(context).colorScheme.surface,
                        width: 2,
                      ),
                    ),
                    constraints: const BoxConstraints(
                      minWidth: 20,
                      minHeight: 20,
                    ),
                    child: Center(
                      child: Text(
                        '${_getActiveFilterCount()}',
                        style: const TextStyle(
                          color: Colors.white,
                          fontSize: 10,
                          fontWeight: FontWeight.bold,
                        ),
                      ),
                    ),
                  ),
                ),
            ],
          ),
          
          const SizedBox(width: 8),
          
          // Active filters summary
          if (_hasActiveFilters())
            Expanded(
              child: Text(
                _getFilterSummary(),
                style: TextStyle(
                  fontSize: 12,
                  color: Colors.grey.shade600,
                ),
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
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

  String _getFilterSummary() {
    final parts = <String>[];
    
    if (_selectedIncludeFormats.isNotEmpty) {
      parts.add('Include: ${_selectedIncludeFormats.take(2).join(", ")}${_selectedIncludeFormats.length > 2 ? "..." : ""}');
    }
    
    if (_selectedExcludeFormats.isNotEmpty) {
      parts.add('Exclude: ${_selectedExcludeFormats.take(2).join(", ")}${_selectedExcludeFormats.length > 2 ? "..." : ""}');
    }
    
    if (_maxSize != null) {
      parts.add('Max: $_maxSize');
    }
    
    return parts.join(' • ');
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
