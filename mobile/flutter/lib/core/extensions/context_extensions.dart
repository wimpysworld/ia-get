import 'package:flutter/material.dart';

/// BuildContext extensions for easier access to common properties
extension ContextExtensions on BuildContext {
  /// Returns Theme.of(context)
  ThemeData get theme => Theme.of(this);

  /// Returns Theme.of(context).textTheme
  TextTheme get textTheme => theme.textTheme;

  /// Returns Theme.of(context).colorScheme
  ColorScheme get colorScheme => theme.colorScheme;

  /// Returns MediaQuery.of(context)
  MediaQueryData get mediaQuery => MediaQuery.of(this);

  /// Returns screen size
  Size get screenSize => mediaQuery.size;

  /// Returns screen width
  double get screenWidth => screenSize.width;

  /// Returns screen height
  double get screenHeight => screenSize.height;

  /// Returns Navigator.of(context)
  NavigatorState get navigator => Navigator.of(this);

  /// Checks if device is in landscape mode
  bool get isLandscape => mediaQuery.orientation == Orientation.landscape;

  /// Checks if device is in portrait mode
  bool get isPortrait => mediaQuery.orientation == Orientation.portrait;

  /// Hides keyboard
  void hideKeyboard() {
    FocusScope.of(this).unfocus();
  }

  /// Shows snackbar with message
  void showSnackBar(String message, {Duration duration = const Duration(seconds: 3)}) {
    ScaffoldMessenger.of(this).showSnackBar(
      SnackBar(content: Text(message), duration: duration),
    );
  }

  /// Navigates to route
  Future<T?> push<T>(Widget page) {
    return navigator.push<T>(MaterialPageRoute(builder: (_) => page));
  }

  /// Pops current route
  void pop<T>([T? result]) {
    navigator.pop(result);
  }
}
