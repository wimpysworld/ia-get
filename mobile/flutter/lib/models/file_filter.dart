/// Advanced file filtering model
/// 
/// Supports multiple filter types including:
/// - Wildcard patterns (*.pdf, chapter*)
/// - Regex patterns (^data_\\d+\\.txt$)
/// - Subfolder filtering (subfolder/, subfolder/*)
/// - Size range filters
/// - Format filters
/// - Source type filters
class FileFilter {
  // Wildcard and regex patterns
  final List<String> includePatterns;
  final List<String> excludePatterns;
  
  // Subfolder filters
  final List<String> includeSubfolders;
  final List<String> excludeSubfolders;
  
  // Format filters
  final List<String> includeFormats;
  final List<String> excludeFormats;
  
  // Size filters (in bytes)
  final int? minSize;
  final int? maxSize;
  
  // Source type filters
  final bool includeOriginal;
  final bool includeDerivative;
  final bool includeMetadata;
  
  // Regex mode (if true, patterns are treated as regex)
  final bool useRegex;

  const FileFilter({
    this.includePatterns = const [],
    this.excludePatterns = const [],
    this.includeSubfolders = const [],
    this.excludeSubfolders = const [],
    this.includeFormats = const [],
    this.excludeFormats = const [],
    this.minSize,
    this.maxSize,
    this.includeOriginal = true,
    this.includeDerivative = true,
    this.includeMetadata = true,
    this.useRegex = false,
  });

  /// Check if this filter has any active criteria
  bool get hasActiveCriteria {
    return includePatterns.isNotEmpty ||
        excludePatterns.isNotEmpty ||
        includeSubfolders.isNotEmpty ||
        excludeSubfolders.isNotEmpty ||
        includeFormats.isNotEmpty ||
        excludeFormats.isNotEmpty ||
        minSize != null ||
        maxSize != null ||
        !includeOriginal ||
        !includeDerivative ||
        !includeMetadata;
  }

  /// Count of active filter criteria
  int get activeFilterCount {
    int count = 0;
    if (includePatterns.isNotEmpty) count++;
    if (excludePatterns.isNotEmpty) count++;
    if (includeSubfolders.isNotEmpty) count++;
    if (excludeSubfolders.isNotEmpty) count++;
    if (includeFormats.isNotEmpty) count++;
    if (excludeFormats.isNotEmpty) count++;
    if (minSize != null || maxSize != null) count++;
    if (!includeOriginal || !includeDerivative || !includeMetadata) count++;
    return count;
  }

  /// Get a human-readable summary of active filters
  String getSummary() {
    final parts = <String>[];

    if (includePatterns.isNotEmpty) {
      parts.add('Include: ${includePatterns.take(2).join(", ")}${includePatterns.length > 2 ? "..." : ""}');
    }

    if (includeSubfolders.isNotEmpty) {
      parts.add('Folders: ${includeSubfolders.take(2).join(", ")}${includeSubfolders.length > 2 ? "..." : ""}');
    }

    if (excludePatterns.isNotEmpty) {
      parts.add('Exclude: ${excludePatterns.take(2).join(", ")}${excludePatterns.length > 2 ? "..." : ""}');
    }

    if (includeFormats.isNotEmpty) {
      parts.add('Formats: ${includeFormats.take(2).join(", ")}${includeFormats.length > 2 ? "..." : ""}');
    }

    if (minSize != null || maxSize != null) {
      if (minSize != null && maxSize != null) {
        parts.add('Size: ${_formatSize(minSize!)} - ${_formatSize(maxSize!)}');
      } else if (minSize != null) {
        parts.add('Size: > ${_formatSize(minSize!)}');
      } else {
        parts.add('Size: < ${_formatSize(maxSize!)}');
      }
    }

    if (!includeOriginal || !includeDerivative || !includeMetadata) {
      final types = <String>[];
      if (includeOriginal) types.add('O');
      if (includeDerivative) types.add('D');
      if (includeMetadata) types.add('M');
      parts.add('Source: ${types.join(",")}');
    }

    return parts.join(' • ');
  }

  String _formatSize(int bytes) {
    const units = ['B', 'KB', 'MB', 'GB'];
    double size = bytes.toDouble();
    int unitIndex = 0;

    while (size >= 1024 && unitIndex < units.length - 1) {
      size /= 1024;
      unitIndex++;
    }

    return '${size.toStringAsFixed(size < 10 ? 1 : 0)}${units[unitIndex]}';
  }

  /// Create a copy with modified values
  FileFilter copyWith({
    List<String>? includePatterns,
    List<String>? excludePatterns,
    List<String>? includeSubfolders,
    List<String>? excludeSubfolders,
    List<String>? includeFormats,
    List<String>? excludeFormats,
    int? minSize,
    int? maxSize,
    bool? includeOriginal,
    bool? includeDerivative,
    bool? includeMetadata,
    bool? useRegex,
  }) {
    return FileFilter(
      includePatterns: includePatterns ?? this.includePatterns,
      excludePatterns: excludePatterns ?? this.excludePatterns,
      includeSubfolders: includeSubfolders ?? this.includeSubfolders,
      excludeSubfolders: excludeSubfolders ?? this.excludeSubfolders,
      includeFormats: includeFormats ?? this.includeFormats,
      excludeFormats: excludeFormats ?? this.excludeFormats,
      minSize: minSize ?? this.minSize,
      maxSize: maxSize ?? this.maxSize,
      includeOriginal: includeOriginal ?? this.includeOriginal,
      includeDerivative: includeDerivative ?? this.includeDerivative,
      includeMetadata: includeMetadata ?? this.includeMetadata,
      useRegex: useRegex ?? this.useRegex,
    );
  }

  /// Create an empty filter (no filtering)
  static const FileFilter empty = FileFilter();

  /// Convert to JSON for serialization
  Map<String, dynamic> toJson() {
    return {
      'includePatterns': includePatterns,
      'excludePatterns': excludePatterns,
      'includeSubfolders': includeSubfolders,
      'excludeSubfolders': excludeSubfolders,
      'includeFormats': includeFormats,
      'excludeFormats': excludeFormats,
      'minSize': minSize,
      'maxSize': maxSize,
      'includeOriginal': includeOriginal,
      'includeDerivative': includeDerivative,
      'includeMetadata': includeMetadata,
      'useRegex': useRegex,
    };
  }

  /// Create from JSON
  factory FileFilter.fromJson(Map<String, dynamic> json) {
    return FileFilter(
      includePatterns: (json['includePatterns'] as List<dynamic>?)
              ?.map((e) => e.toString())
              .toList() ??
          const [],
      excludePatterns: (json['excludePatterns'] as List<dynamic>?)
              ?.map((e) => e.toString())
              .toList() ??
          const [],
      includeSubfolders: (json['includeSubfolders'] as List<dynamic>?)
              ?.map((e) => e.toString())
              .toList() ??
          const [],
      excludeSubfolders: (json['excludeSubfolders'] as List<dynamic>?)
              ?.map((e) => e.toString())
              .toList() ??
          const [],
      includeFormats: (json['includeFormats'] as List<dynamic>?)
              ?.map((e) => e.toString())
              .toList() ??
          const [],
      excludeFormats: (json['excludeFormats'] as List<dynamic>?)
              ?.map((e) => e.toString())
              .toList() ??
          const [],
      minSize: json['minSize'] as int?,
      maxSize: json['maxSize'] as int?,
      includeOriginal: json['includeOriginal'] as bool? ?? true,
      includeDerivative: json['includeDerivative'] as bool? ?? true,
      includeMetadata: json['includeMetadata'] as bool? ?? true,
      useRegex: json['useRegex'] as bool? ?? false,
    );
  }
}
