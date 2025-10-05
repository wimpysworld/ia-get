import 'package:flutter/material.dart';

/// Mixin for pull-to-refresh functionality
mixin RefreshableMixin on State {
  bool _isRefreshing = false;

  bool get isRefreshing => _isRefreshing;

  /// Override this to provide refresh logic
  Future<void> onRefresh();

  /// Handles refresh action
  Future<void> handleRefresh() async {
    if (_isRefreshing) return;

    setState(() {
      _isRefreshing = true;
    });

    try {
      await onRefresh();
    } finally {
      if (mounted) {
        setState(() {
          _isRefreshing = false;
        });
      }
    }
  }

  /// Wraps content in RefreshIndicator
  Widget buildRefreshable(Widget child) {
    return RefreshIndicator(
      onRefresh: handleRefresh,
      child: child,
    );
  }
}
