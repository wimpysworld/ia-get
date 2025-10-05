import 'dart:async';
import 'package:flutter/widgets.dart';

/// Mixin that provides search functionality with debouncing and history
mixin SearchableMixin<T> on State {
  final _searchController = TextEditingController();
  final _searchHistory = <String>[];
  Timer? _debounceTimer;
  
  /// Get the search controller
  TextEditingController get searchController => _searchController;
  
  /// Get search history
  List<String> get searchHistory => List.unmodifiable(_searchHistory);
  
  /// Current search query
  String get searchQuery => _searchController.text;
  
  /// Debounce duration (default: 500ms)
  Duration get searchDebounce => const Duration(milliseconds: 500);
  
  /// Maximum search history items
  int get maxSearchHistory => 10;
  
  /// Override this to perform search
  Future<List<T>> performSearch(String query);
  
  /// Override this to handle search results
  void onSearchResults(List<T> results);
  
  /// Override this to handle search errors
  void onSearchError(Object error) {
    debugPrint('Search error: $error');
  }
  
  /// Initialize searchable mixin
  void initSearchable() {
    _searchController.addListener(_onSearchChanged);
  }
  
  /// Dispose searchable resources
  void disposeSearchable() {
    _debounceTimer?.cancel();
    _searchController.removeListener(_onSearchChanged);
    _searchController.dispose();
  }
  
  void _onSearchChanged() {
    _debounceTimer?.cancel();
    
    final query = _searchController.text.trim();
    if (query.isEmpty) {
      onSearchResults([]);
      return;
    }
    
    _debounceTimer = Timer(searchDebounce, () async {
      try {
        final results = await performSearch(query);
        onSearchResults(results);
        _addToHistory(query);
      } catch (e) {
        onSearchError(e);
      }
    });
  }
  
  void _addToHistory(String query) {
    if (query.isEmpty) return;
    
    _searchHistory.remove(query);
    _searchHistory.insert(0, query);
    
    if (_searchHistory.length > maxSearchHistory) {
      _searchHistory.removeRange(maxSearchHistory, _searchHistory.length);
    }
  }
  
  /// Clear search
  void clearSearch() {
    _searchController.clear();
    onSearchResults([]);
  }
  
  /// Search from history
  void searchFromHistory(String query) {
    _searchController.text = query;
  }
  
  /// Clear search history
  void clearHistory() {
    _searchHistory.clear();
  }
}
