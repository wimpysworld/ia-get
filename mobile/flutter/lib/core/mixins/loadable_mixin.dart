import 'package:flutter/material.dart';

/// Mixin for managing loading states
mixin LoadableMixin<T> on State {
  bool _isLoading = false;
  String? _error;
  T? _data;

  bool get isLoading => _isLoading;
  String? get error => _error;
  T? get data => _data;

  @override
  void initState() {
    super.initState();
    reload();
  }

  /// Override this to provide data loading logic
  Future<T> loadData();

  /// Reloads data
  Future<void> reload() async {
    setState(() {
      _isLoading = true;
      _error = null;
    });

    try {
      final result = await loadData();
      if (mounted) {
        setState(() {
          _data = result;
          _isLoading = false;
        });
      }
    } catch (e) {
      if (mounted) {
        setState(() {
          _error = e.toString();
          _isLoading = false;
        });
      }
    }
  }

  /// Builds widget based on state
  @override
  Widget build(BuildContext context) {
    if (_isLoading && _data == null) {
      return buildLoading(context);
    }

    if (_error != null && _data == null) {
      return buildError(context, _error!);
    }

    if (_data != null) {
      return buildContent(context, _data as T);
    }

    return const SizedBox.shrink();
  }

  /// Override to customize loading widget
  Widget buildLoading(BuildContext context) {
    return const Center(child: CircularProgressIndicator());
  }

  /// Override to customize error widget
  Widget buildError(BuildContext context, String error) {
    return Center(
      child: Column(
        mainAxisAlignment: MainAxisAlignment.center,
        children: [
          const Icon(Icons.error, size: 48, color: Colors.red),
          const SizedBox(height: 16),
          Text(error),
          const SizedBox(height: 16),
          ElevatedButton(
            onPressed: reload,
            child: const Text('Retry'),
          ),
        ],
      ),
    );
  }

  /// Override to build content widget
  Widget buildContent(BuildContext context, T data);
}
