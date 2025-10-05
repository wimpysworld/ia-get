/// Custom exceptions for Internet Archive operations
library;

/// Base exception for all Internet Archive related errors
abstract class IAException implements Exception {
  final String message;
  final String? details;
  final dynamic originalError;
  
  const IAException(this.message, {this.details, this.originalError});
  
  @override
  String toString() {
    var str = 'IAException: $message';
    if (details != null) {
      str += '\nDetails: $details';
    }
    if (originalError != null) {
      str += '\nCaused by: $originalError';
    }
    return str;
  }
}

/// Exception thrown when an item is not found
class ItemNotFoundException extends IAException {
  final String identifier;
  
  const ItemNotFoundException(this.identifier)
      : super('Item not found: $identifier',
            details: 'The identifier may be incorrect or the item may have been removed.');
}

/// Exception thrown when access is forbidden
class AccessForbiddenException extends IAException {
  final String identifier;
  
  const AccessForbiddenException(this.identifier)
      : super('Access forbidden to item: $identifier',
            details: 'This item may be restricted or require authentication.');
}

/// Exception thrown when rate limited by the API
class RateLimitException extends IAException {
  final int retryAfterSeconds;
  
  const RateLimitException([this.retryAfterSeconds = 30])
      : super('Rate limited by Internet Archive',
            details: 'Please wait $retryAfterSeconds seconds before retrying.');
}

/// Exception thrown for network-related errors
class NetworkException extends IAException {
  const NetworkException(String message, {dynamic originalError})
      : super('Network error: $message', originalError: originalError);
}

/// Exception thrown for server errors
class ServerException extends IAException {
  final int statusCode;
  
  const ServerException(this.statusCode)
      : super('Server error: HTTP $statusCode',
            details: 'The Internet Archive server returned an error. Please try again later.');
}

/// Exception thrown when request times out
class TimeoutException extends IAException {
  const TimeoutException()
      : super('Request timed out',
            details: 'The request took too long to complete. Please check your connection and try again.');
}

/// Exception thrown for invalid identifiers
class InvalidIdentifierException extends IAException {
  final String identifier;
  
  const InvalidIdentifierException(this.identifier)
      : super('Invalid identifier format: $identifier',
            details: 'Identifiers must be 3-100 characters and contain only alphanumeric characters, hyphens, underscores, and periods.');
}

/// Exception thrown for download errors
class DownloadException extends IAException {
  final String filename;
  
  const DownloadException(this.filename, String message, {dynamic originalError})
      : super('Download failed: $message', originalError: originalError);
}

/// Exception thrown for checksum validation failures
class ChecksumException extends IAException {
  final String filename;
  final String expectedHash;
  final String actualHash;
  
  const ChecksumException(this.filename, this.expectedHash, this.actualHash)
      : super('Checksum validation failed for: $filename',
            details: 'Expected: $expectedHash\nActual: $actualHash');
}

/// Exception thrown for parsing errors
class ParsingException extends IAException {
  const ParsingException(String message, {dynamic originalError})
      : super('Failed to parse response: $message', originalError: originalError);
}
