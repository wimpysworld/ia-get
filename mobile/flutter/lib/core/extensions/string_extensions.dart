/// String extensions for enhanced functionality
extension StringExtensions on String {
  /// Validates if string is a valid Internet Archive identifier
  bool isValidIdentifier() {
    if (isEmpty) return false;
    // Identifiers should not contain spaces and should be reasonable length
    return !contains(' ') && length >= 3 && length <= 100;
  }

  /// Converts identifier to Archive.org details URL
  String toArchiveUrl() {
    return 'https://archive.org/details/$this';
  }

  /// Converts identifier to Archive.org metadata URL
  String toMetadataUrl() {
    return 'https://archive.org/metadata/$this';
  }

  /// Capitalizes first letter of string
  String capitalize() {
    if (isEmpty) return this;
    return '${this[0].toUpperCase()}${substring(1).toLowerCase()}';
  }

  /// Capitalizes first letter of each word
  String capitalizeWords() {
    if (isEmpty) return this;
    return split(' ').map((word) => word.capitalize()).join(' ');
  }

  /// Truncates string with ellipsis
  String truncateWithEllipsis(int maxLength) {
    if (length <= maxLength) return this;
    return '${substring(0, maxLength - 3)}...';
  }

  /// Validates email format
  bool isEmail() {
    final emailRegex = RegExp(
      r'^[a-zA-Z0-9.]+@[a-zA-Z0-9]+\.[a-zA-Z]+',
    );
    return emailRegex.hasMatch(this);
  }

  /// Validates URL format
  bool isUrl() {
    final urlRegex = RegExp(
      r'^https?:\/\/(www\.)?[-a-zA-Z0-9@:%._\+~#=]{1,256}\.[a-zA-Z0-9()]{1,6}\b([-a-zA-Z0-9()@:%_\+.~#?&//=]*)$',
    );
    return urlRegex.hasMatch(this);
  }

  /// Converts string to URL-friendly slug
  String toSlug() {
    return toLowerCase()
        .replaceAll(RegExp(r'[^\w\s-]'), '')
        .replaceAll(RegExp(r'[\s_]+'), '-')
        .replaceAll(RegExp(r'^-+|-+$'), '');
  }

  /// Removes HTML tags from string
  String removeHtmlTags() {
    return replaceAll(RegExp(r'<[^>]*>'), '');
  }

  /// Checks if string is numeric
  bool isNumeric() {
    return double.tryParse(this) != null;
  }

  /// Parses string to int safely
  int? toIntOrNull() {
    return int.tryParse(this);
  }

  /// Parses string to double safely
  double? toDoubleOrNull() {
    return double.tryParse(this);
  }

  /// Reverses the string
  String reversed() {
    return split('').reversed.join();
  }

  /// Checks if string contains only letters
  bool isAlpha() {
    return RegExp(r'^[a-zA-Z]+$').hasMatch(this);
  }

  /// Checks if string contains only alphanumeric characters
  bool isAlphanumeric() {
    return RegExp(r'^[a-zA-Z0-9]+$').hasMatch(this);
  }

  /// Extracts all numbers from string
  List<int> extractNumbers() {
    final matches = RegExp(r'\d+').allMatches(this);
    return matches.map((m) => int.parse(m.group(0)!)).toList();
  }

  /// Checks if string matches glob pattern
  bool matchesGlob(String pattern) {
    final regexPattern = pattern
        .replaceAll('*', '.*')
        .replaceAll('?', '.')
        .replaceAll('[', r'\[')
        .replaceAll(']', r'\]');
    return RegExp('^$regexPattern\$').hasMatch(this);
  }
}
