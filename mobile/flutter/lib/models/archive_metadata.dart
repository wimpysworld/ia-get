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
    final files = filesList
        .map((file) => ArchiveFile.fromJson(file as Map<String, dynamic>))
        .toList();
    
    return ArchiveMetadata(
      identifier: json['metadata']?['identifier'] ?? 'unknown',
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
  final String? downloadUrl;
  final String? md5;
  final String? sha1;
  bool selected;
  
  ArchiveFile({
    required this.name,
    this.size,
    this.format,
    this.downloadUrl,
    this.md5,
    this.sha1,
    this.selected = false,
  });
  
  factory ArchiveFile.fromJson(Map<String, dynamic> json) {
    return ArchiveFile(
      name: json['name'] ?? '',
      size: json['size'],
      format: json['format'],
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
}