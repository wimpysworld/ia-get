/// Search result model for Internet Archive search API responses
class SearchResult {
  final String identifier;
  final String title;
  final String description;

  SearchResult({
    required this.identifier,
    required this.title,
    required this.description,
  });

  /// Factory constructor to handle Internet Archive API quirk where
  /// title and description can be either a string or a list of strings
  factory SearchResult.fromJson(Map<String, dynamic> json) {
    return SearchResult(
      identifier: _extractString(json['identifier'], ''),
      title: _extractString(json['title'], 'Untitled'),
      description: _extractString(json['description'], ''),
    );
  }

  /// Helper method to extract a string value from either a string or list
  /// 
  /// The Internet Archive API sometimes returns fields as:
  /// - A single string: "Example Title"
  /// - A list of strings: ["Example Title", "Alternative Title"]
  /// 
  /// This method handles both cases, taking the first element if it's a list.
  static String _extractString(dynamic value, String defaultValue) {
    if (value == null) return defaultValue;
    
    if (value is List) {
      return value.isNotEmpty ? value.first.toString() : defaultValue;
    }
    
    return value.toString();
  }

  /// Convert to the Map format expected by the UI
  Map<String, String> toMap() {
    return {
      'identifier': identifier,
      'title': title,
      'description': description,
    };
  }

  /// Convert to JSON
  Map<String, dynamic> toJson() {
    return {
      'identifier': identifier,
      'title': title,
      'description': description,
    };
  }
}
