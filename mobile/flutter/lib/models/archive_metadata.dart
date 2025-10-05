/// Archive metadata model
class ArchiveMetadata {
  final String identifier;
  final String? title;
  final String? description;
  final String? creator;
  final String? date;
  final int totalFiles;
  final int totalSize;
  final List<ArchiveFile> files;

  ArchiveMetadata({
    required this.identifier,
    this.title,
    this.description,
    this.creator,
    this.date,
    required this.totalFiles,
    required this.totalSize,
    required this.files,
  });

  factory ArchiveMetadata.fromJson(Map<String, dynamic> json) {
    final filesList = json['files'] as List<dynamic>? ?? [];
    final server = json['server'] as String? ?? json['d1'] as String? ?? '';
    final dir = json['dir'] as String? ?? '';

    final files = filesList.map((file) {
      final fileMap = file as Map<String, dynamic>;
      // Generate download URL from server and directory if not present
      if (fileMap['download_url'] == null &&
          server.isNotEmpty &&
          dir.isNotEmpty) {
        final fileName = fileMap['name'] as String? ?? '';
        fileMap['download_url'] = 'https://$server$dir/$fileName';
      }
      return ArchiveFile.fromJson(fileMap);
    }).toList();

    // Try multiple strategies to extract identifier
    String identifier = 'unknown';
    
    // Strategy 1: Check metadata.identifier
    if (json['metadata'] != null && json['metadata']['identifier'] != null) {
      identifier = json['metadata']['identifier'];
    } 
    // Strategy 2: Check top-level identifier
    else if (json['identifier'] != null) {
      identifier = json['identifier'];
    }
    // Strategy 3: Extract from directory path (e.g., /21/items/commute_test -> commute_test)
    else if (dir.isNotEmpty) {
      final parts = dir.split('/').where((p) => p.isNotEmpty).toList();
      if (parts.length >= 2) {
        // Directory format is usually /digits/items/identifier
        identifier = parts.last;
      }
    }

    return ArchiveMetadata(
      identifier: identifier,
      title: json['metadata']?['title'],
      description: json['metadata']?['description'],
      creator: json['metadata']?['creator'],
      date: json['metadata']?['date'],
      totalFiles: files.length,
      totalSize: json['item_size'] ?? 0,
      files: files,
    );
  }

  Map<String, dynamic> toJson() {
    return {
      'metadata': {
        'identifier': identifier,
        'title': title,
        'description': description,
        'creator': creator,
        'date': date,
      },
      'item_size': totalSize,
      'files': files.map((f) => f.toJson()).toList(),
    };
  }
}

/// Individual file in an archive
class ArchiveFile {
  final String name;
  final int? size;
  final String? format;
  final String? source;
  final String? downloadUrl;
  final String? md5;
  final String? sha1;
  bool selected;

  ArchiveFile({
    required this.name,
    this.size,
    this.format,
    this.source,
    this.downloadUrl,
    this.md5,
    this.sha1,
    this.selected = false,
  });
  
  /// Get the directory path of this file (everything before the last /)
  String get directory {
    final lastSlash = name.lastIndexOf('/');
    if (lastSlash == -1) return '';
    return name.substring(0, lastSlash);
  }
  
  /// Get just the filename (after the last /)
  String get filename {
    final lastSlash = name.lastIndexOf('/');
    if (lastSlash == -1) return name;
    return name.substring(lastSlash + 1);
  }
  
  /// Check if file is in a specific subfolder (supports wildcards)
  bool isInSubfolder(String pattern) {
    if (pattern.isEmpty) return true;
    
    final dir = directory;
    final patternLower = pattern.toLowerCase();
    final dirLower = dir.toLowerCase();
    
    // Exact match
    if (dirLower == patternLower) return true;
    
    // Starts with pattern (subfolder matching)
    if (dirLower.startsWith(patternLower)) return true;
    
    // Wildcard pattern matching
    if (patternLower.contains('*')) {
      final regexPattern = patternLower
          .replaceAll('\\', '\\\\')
          .replaceAll('.', '\\.')
          .replaceAll('*', '.*')
          .replaceAll('?', '.');
      try {
        final regex = RegExp('^$regexPattern\$');
        return regex.hasMatch(dirLower);
      } catch (_) {
        return false;
      }
    }
    
    return false;
  }

  factory ArchiveFile.fromJson(Map<String, dynamic> json) {
    return ArchiveFile(
      name: json['name'] ?? '',
      size: json['size'],
      format: json['format'],
      source: json['source'],
      downloadUrl: json['download_url'],
      md5: json['md5'],
      sha1: json['sha1'],
      selected: json['selected'] ?? false,
    );
  }

  Map<String, dynamic> toJson() {
    return {
      'name': name,
      'size': size,
      'format': format,
      'source': source,
      'download_url': downloadUrl,
      'md5': md5,
      'sha1': sha1,
      'selected': selected,
    };
  }

  String get sizeFormatted {
    if (size == null) return 'Unknown size';

    const units = ['B', 'KB', 'MB', 'GB', 'TB'];
    double bytes = size!.toDouble();
    int unitIndex = 0;

    while (bytes >= 1024 && unitIndex < units.length - 1) {
      bytes /= 1024;
      unitIndex++;
    }

    return '${bytes.toStringAsFixed(bytes >= 100 ? 0 : 1)} ${units[unitIndex]}';
  }

  String get displayName {
    // Remove common prefixes and clean up the name
    String cleanName = name;
    if (cleanName.contains('/')) {
      cleanName = cleanName.split('/').last;
    }
    return cleanName;
  }

  /// Check if this is an original file
  bool get isOriginal => source?.toLowerCase() == 'original';

  /// Check if this is a derivative file
  bool get isDerivative => source?.toLowerCase() == 'derivative';

  /// Check if this is a metadata file
  bool get isMetadata => source?.toLowerCase() == 'metadata';

  /// Get a user-friendly source type name
  String get sourceTypeName {
    if (isOriginal) return 'Original';
    if (isDerivative) return 'Derivative';
    if (isMetadata) return 'Metadata';
    return 'Unknown';
  }
}
