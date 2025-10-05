import 'package:flutter/material.dart';

/// Mixin for managing disposable resources
mixin DisposableMixin on State {
  final List<void Function()> _disposers = [];

  /// Registers a disposer function
  void registerDisposer(void Function() disposer) {
    _disposers.add(disposer);
  }

  @override
  void dispose() {
    for (final disposer in _disposers) {
      try {
        disposer();
      } catch (e) {
        debugPrint('Error disposing resource: $e');
      }
    }
    _disposers.clear();
    super.dispose();
  }
}
