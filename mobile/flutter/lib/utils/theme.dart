import 'package:flutter/material.dart';

/// Internet Archive Helper Material 3 Theme
///
/// This theme implements Material 3 design principles with Internet Archive
/// branding colors and modern accessibility standards.
class AppTheme {
  // Internet Archive brand colors
  static const Color internetArchiveBlue = Color(0xFF004B87);
  static const Color internetArchiveOrange = Color(0xFFFF6B35);
  static const Color internetArchiveGray = Color(0xFF666666);

  // Material 3 color tokens
  static const Color primaryColor = internetArchiveBlue;
  static const Color secondaryColor = internetArchiveOrange;
  static const Color tertiaryColor = Color(0xFF0088CC);

  // Semantic colors
  static const Color successColor = Color(0xFF2E7D32);
  static const Color warningColor = Color(0xFFED6C02);
  static const Color errorColor = Color(0xFFD32F2F);
  static const Color infoColor = Color(0xFF0288D1);

  // Light theme with Material 3 design
  static ThemeData get lightTheme {
    final ColorScheme lightColorScheme = ColorScheme.fromSeed(
      seedColor: primaryColor,
      brightness: Brightness.light,
      // Override specific colors for Internet Archive branding
      primary: internetArchiveBlue,
      secondary: internetArchiveOrange,
      tertiary: tertiaryColor,
      error: errorColor,
    );

    return ThemeData(
      useMaterial3: true,
      colorScheme: lightColorScheme,

      // App Bar with modern Material 3 styling
      appBarTheme: AppBarTheme(
        centerTitle: true,
        elevation: 0,
        scrolledUnderElevation: 1,
        backgroundColor: lightColorScheme.surface,
        foregroundColor: lightColorScheme.onSurface,
        surfaceTintColor: lightColorScheme.surfaceTint,
        iconTheme: IconThemeData(color: lightColorScheme.onSurface),
      ),

      // Cards with Material 3 styling
      cardTheme: CardThemeData(
        elevation: 1,
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(16)),
        clipBehavior: Clip.hardEdge,
      ),

      // Filled buttons with Material 3 styling
      filledButtonTheme: FilledButtonThemeData(
        style: FilledButton.styleFrom(
          shape: RoundedRectangleBorder(
            borderRadius: BorderRadius.circular(20),
          ),
          padding: const EdgeInsets.symmetric(horizontal: 24, vertical: 16),
        ),
      ),

      // Elevated buttons
      elevatedButtonTheme: ElevatedButtonThemeData(
        style: ElevatedButton.styleFrom(
          shape: RoundedRectangleBorder(
            borderRadius: BorderRadius.circular(20),
          ),
          padding: const EdgeInsets.symmetric(horizontal: 24, vertical: 16),
          elevation: 2,
        ),
      ),

      // Text buttons
      textButtonTheme: TextButtonThemeData(
        style: TextButton.styleFrom(
          shape: RoundedRectangleBorder(
            borderRadius: BorderRadius.circular(20),
          ),
          padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 12),
        ),
      ),

      // Input fields with Material 3 styling
      inputDecorationTheme: InputDecorationTheme(
        filled: true,
        border: OutlineInputBorder(borderRadius: BorderRadius.circular(12)),
        enabledBorder: OutlineInputBorder(
          borderRadius: BorderRadius.circular(12),
          borderSide: BorderSide(color: lightColorScheme.outline),
        ),
        focusedBorder: OutlineInputBorder(
          borderRadius: BorderRadius.circular(12),
          borderSide: BorderSide(color: lightColorScheme.primary, width: 2),
        ),
        errorBorder: OutlineInputBorder(
          borderRadius: BorderRadius.circular(12),
          borderSide: BorderSide(color: lightColorScheme.error),
        ),
        contentPadding: const EdgeInsets.symmetric(
          horizontal: 16,
          vertical: 16,
        ),
      ),

      // Progress indicators
      progressIndicatorTheme: ProgressIndicatorThemeData(
        color: lightColorScheme.primary,
        linearTrackColor: lightColorScheme.surfaceContainerHighest,
      ),

      // Bottom navigation
      bottomNavigationBarTheme: BottomNavigationBarThemeData(
        type: BottomNavigationBarType.fixed,
        backgroundColor: lightColorScheme.surface,
        selectedItemColor: lightColorScheme.primary,
        unselectedItemColor: lightColorScheme.onSurfaceVariant,
        elevation: 3,
      ),

      // FAB with Material 3 styling
      floatingActionButtonTheme: FloatingActionButtonThemeData(
        backgroundColor: lightColorScheme.primaryContainer,
        foregroundColor: lightColorScheme.onPrimaryContainer,
        elevation: 3,
        focusElevation: 4,
        hoverElevation: 4,
        highlightElevation: 2,
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(16)),
      ),

      // Dividers
      dividerTheme: DividerThemeData(
        color: lightColorScheme.outlineVariant,
        thickness: 1,
      ),

      // Lists
      listTileTheme: ListTileThemeData(
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(12)),
        contentPadding: const EdgeInsets.symmetric(horizontal: 16, vertical: 8),
      ),
    );
  }

  // Dark theme with Material 3 design
  static ThemeData get darkTheme {
    final ColorScheme darkColorScheme = ColorScheme.fromSeed(
      seedColor: primaryColor,
      brightness: Brightness.dark,
      // Override specific colors for Internet Archive branding
      primary: const Color(0xFF6BB6FF), // Lighter blue for dark theme
      secondary: internetArchiveOrange,
      tertiary: const Color(0xFF64B5F6),
      error: const Color(0xFFFF5449),
    );

    return ThemeData(
      useMaterial3: true,
      colorScheme: darkColorScheme,

      // App Bar with dark theme styling
      appBarTheme: AppBarTheme(
        centerTitle: true,
        elevation: 0,
        scrolledUnderElevation: 1,
        backgroundColor: darkColorScheme.surface,
        foregroundColor: darkColorScheme.onSurface,
        surfaceTintColor: darkColorScheme.surfaceTint,
        iconTheme: IconThemeData(color: darkColorScheme.onSurface),
      ),

      // Cards with Material 3 dark styling
      cardTheme: CardThemeData(
        elevation: 1,
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(16)),
        clipBehavior: Clip.hardEdge,
      ),

      // Filled buttons with Material 3 styling
      filledButtonTheme: FilledButtonThemeData(
        style: FilledButton.styleFrom(
          shape: RoundedRectangleBorder(
            borderRadius: BorderRadius.circular(20),
          ),
          padding: const EdgeInsets.symmetric(horizontal: 24, vertical: 16),
        ),
      ),

      // Elevated buttons
      elevatedButtonTheme: ElevatedButtonThemeData(
        style: ElevatedButton.styleFrom(
          shape: RoundedRectangleBorder(
            borderRadius: BorderRadius.circular(20),
          ),
          padding: const EdgeInsets.symmetric(horizontal: 24, vertical: 16),
          elevation: 2,
        ),
      ),

      // Text buttons
      textButtonTheme: TextButtonThemeData(
        style: TextButton.styleFrom(
          shape: RoundedRectangleBorder(
            borderRadius: BorderRadius.circular(20),
          ),
          padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 12),
        ),
      ),

      // Input fields with Material 3 dark styling
      inputDecorationTheme: InputDecorationTheme(
        filled: true,
        border: OutlineInputBorder(borderRadius: BorderRadius.circular(12)),
        enabledBorder: OutlineInputBorder(
          borderRadius: BorderRadius.circular(12),
          borderSide: BorderSide(color: darkColorScheme.outline),
        ),
        focusedBorder: OutlineInputBorder(
          borderRadius: BorderRadius.circular(12),
          borderSide: BorderSide(color: darkColorScheme.primary, width: 2),
        ),
        errorBorder: OutlineInputBorder(
          borderRadius: BorderRadius.circular(12),
          borderSide: BorderSide(color: darkColorScheme.error),
        ),
        contentPadding: const EdgeInsets.symmetric(
          horizontal: 16,
          vertical: 16,
        ),
      ),

      // Progress indicators
      progressIndicatorTheme: ProgressIndicatorThemeData(
        color: darkColorScheme.primary,
        linearTrackColor: darkColorScheme.surfaceContainerHighest,
      ),

      // Bottom navigation
      bottomNavigationBarTheme: BottomNavigationBarThemeData(
        type: BottomNavigationBarType.fixed,
        backgroundColor: darkColorScheme.surface,
        selectedItemColor: darkColorScheme.primary,
        unselectedItemColor: darkColorScheme.onSurfaceVariant,
        elevation: 3,
      ),

      // FAB with Material 3 dark styling
      floatingActionButtonTheme: FloatingActionButtonThemeData(
        backgroundColor: darkColorScheme.primaryContainer,
        foregroundColor: darkColorScheme.onPrimaryContainer,
        elevation: 3,
        focusElevation: 4,
        hoverElevation: 4,
        highlightElevation: 2,
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(16)),
      ),

      // Dividers
      dividerTheme: DividerThemeData(
        color: darkColorScheme.outlineVariant,
        thickness: 1,
      ),

      // Lists
      listTileTheme: ListTileThemeData(
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(12)),
        contentPadding: const EdgeInsets.symmetric(horizontal: 16, vertical: 8),
      ),
    );
  }
}
