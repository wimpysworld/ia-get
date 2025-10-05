/// Internet Archive API Constants and Configuration
///
/// This file contains all constants related to the Internet Archive API,
/// including endpoints, rate limits, headers, and best practices.
///
/// References:
/// - Metadata API: https://archive.org/developers/md-read.html
/// - Search API: https://archive.org/developers/search.html
/// - Download API: https://archive.org/developers/
/// - Rate Limiting: https://archive.org/services/docs/api/ratelimiting.html

library;

/// Internet Archive API Endpoints
class IAEndpoints {
  /// Base URL for the Internet Archive
  static const String base = 'https://archive.org';
  
  /// Metadata API endpoint
  /// Example: https://archive.org/metadata/{identifier}
  static const String metadata = '$base/metadata';
  
  /// Details page endpoint
  /// Example: https://archive.org/details/{identifier}
  static const String details = '$base/details';
  
  /// Download endpoint (S3-like URLs)
  /// Example: https://archive.org/download/{identifier}/{filename}
  static const String download = '$base/download';
  
  /// Advanced search API endpoint
  /// Example: https://archive.org/advancedsearch.php?q=...&output=json
  static const String advancedSearch = '$base/advancedsearch.php';
  
  /// Simple search endpoint
  /// Example: https://archive.org/search.php?query=...
  static const String search = '$base/search.php';
  
  /// Services API (for books, etc.)
  static const String services = '$base/services';
  
  /// Thumbnail/Image service
  /// Example: https://archive.org/services/img/{identifier}
  static const String thumbnail = '$services/img';
}

/// Rate Limiting Configuration
///
/// Internet Archive recommends:
/// - No more than 30 requests per minute for metadata
/// - Be respectful and implement exponential backoff
/// - Use appropriate User-Agent headers
class IARateLimits {
  /// Minimum delay between requests (milliseconds)
  static const int minRequestDelayMs = 100;
  
  /// Maximum requests per minute (recommended limit)
  static const int maxRequestsPerMinute = 30;
  
  /// Default retry delay in seconds
  static const int defaultRetryDelaySecs = 30;
  
  /// Maximum number of retry attempts
  static const int maxRetries = 3;
  
  /// Exponential backoff multiplier
  static const double backoffMultiplier = 2.0;
  
  /// Maximum backoff delay (10 minutes)
  static const int maxBackoffDelaySecs = 600;
}

/// HTTP Configuration
class IAHttpConfig {
  /// Default HTTP timeout in seconds
  static const int timeoutSeconds = 30;
  
  /// Download timeout (longer for large files)
  static const int downloadTimeoutSeconds = 300;
  
  /// Connection timeout
  static const int connectionTimeoutSeconds = 10;
}

/// HTTP Headers for API Compliance
class IAHeaders {
  /// User-Agent header - REQUIRED
  /// Format: AppName/Version (Contact: email)
  static String userAgent(String appVersion) =>
      'InternetArchiveHelper/$appVersion (Flutter; https://github.com/Gameaday/ia-get-cli)';
  
  /// Accept header for JSON responses
  static const String acceptJson = 'application/json, text/plain, */*';
  
  /// Accept-Language header
  static const String acceptLanguage = 'en-US,en;q=0.9';
  
  /// Cache-Control for fresh data
  static const String cacheControl = 'no-cache';
  
  /// Do Not Track header (be respectful)
  static const String doNotTrack = '1';
  
  /// Standard headers map
  static Map<String, String> standard(String appVersion) => {
        'User-Agent': userAgent(appVersion),
        'Accept': acceptJson,
        'Accept-Language': acceptLanguage,
        'Cache-Control': cacheControl,
        'DNT': doNotTrack,
      };
}

/// Search Query Parameters
class IASearchParams {
  /// Output format for search results
  static const String outputJson = 'json';
  
  /// Default number of results per page
  static const int defaultRows = 20;
  
  /// Maximum results per page
  static const int maxRows = 10000;
  
  /// Fields to return in search results
  static const List<String> defaultFields = [
    'identifier',
    'title',
    'description',
    'mediatype',
    'downloads',
    'item_size',
  ];
  
  /// Sort options
  static const String sortRelevance = '';
  static const String sortDownloads = '-downloads';
  static const String sortDate = '-publicdate';
  static const String sortTitle = 'title';
}

/// File Source Types
///
/// Types of files in Internet Archive items
class IAFileSourceTypes {
  /// Original uploaded files
  static const String original = 'original';
  
  /// Derivative files (converted, compressed, etc.)
  static const String derivative = 'derivative';
  
  /// Metadata files
  static const String metadata = 'metadata';
}

/// Best Practices for Internet Archive API Usage
class IABestPractices {
  /// Guidelines for using the API responsibly
  static const List<String> guidelines = [
    'Always include a descriptive User-Agent header with contact information',
    'Respect rate limits (max 30 requests/minute)',
    'Implement exponential backoff for retries',
    'Cache metadata responses when possible',
    'Use conditional requests (If-Modified-Since) when appropriate',
    'Handle 429 (Too Many Requests) responses gracefully',
    'Be mindful of Archive.org server load',
    'Consider using S3-like download URLs for better performance',
  ];
}

/// Error Messages
class IAErrorMessages {
  /// Item not found error
  static const String notFound = 'Archive item not found (404)';
  
  /// Access forbidden error
  static const String forbidden = 'Access to archive item is forbidden (403)';
  
  /// Server error
  static const String serverError = 'Internet Archive server error';
  
  /// Invalid identifier error
  static const String invalidIdentifier = 'Invalid identifier format';
  
  /// Rate limit error
  static const String rateLimit = 'Rate limit exceeded. Please slow down requests.';
  
  /// Network error
  static const String networkError = 'Network connection failed';
  
  /// Timeout error
  static const String timeout = 'Request timed out';
}

/// Utility Functions
class IAUtils {
  /// Validate an Internet Archive identifier
  ///
  /// Identifiers should be 3-100 characters and contain only:
  /// - Alphanumeric characters
  /// - Hyphens, underscores, periods
  static bool isValidIdentifier(String identifier) {
    if (identifier.isEmpty || identifier.length < 3 || identifier.length > 100) {
      return false;
    }
    
    // Check for valid characters
    final validPattern = RegExp(r'^[a-zA-Z0-9._-]+$');
    return validPattern.hasMatch(identifier);
  }
  
  /// Build a metadata URL from an identifier
  static String buildMetadataUrl(String identifier) {
    return '${IAEndpoints.metadata}/$identifier';
  }
  
  /// Build a download URL for a file
  static String buildDownloadUrl(String identifier, String filename) {
    return '${IAEndpoints.download}/$identifier/$filename';
  }
  
  /// Build a thumbnail URL
  static String buildThumbnailUrl(String identifier) {
    return '${IAEndpoints.thumbnail}/$identifier';
  }
  
  /// Build a search URL with parameters
  static String buildSearchUrl({
    required String query,
    int rows = IASearchParams.defaultRows,
    int page = 1,
    List<String>? fields,
    String? sort,
  }) {
    final fieldsStr = (fields ?? IASearchParams.defaultFields)
        .map((f) => 'fl[]=$f')
        .join('&');
    
    var url = '${IAEndpoints.advancedSearch}?'
        'q=${Uri.encodeComponent(query)}&'
        '$fieldsStr&'
        'rows=$rows&'
        'page=$page&'
        'output=${IASearchParams.outputJson}';
    
    if (sort != null && sort.isNotEmpty) {
      url += '&sort=$sort';
    }
    
    return url;
  }
}
