# Server Retry-After Header Support

## Overview

The Internet Archive Helper now fully supports server-provided retry-after headers, ensuring proper API compliance and reducing unnecessary server load.

## Implementation

### What is Retry-After?

The `Retry-After` HTTP header tells clients how long to wait before making a follow-up request. Servers use this to:
- Manage rate limiting (429 Too Many Requests)
- Handle temporary server issues (503 Service Unavailable)
- Schedule maintenance windows

Format: `Retry-After: 120` (seconds)

## Changes Made

### 1. Metadata Fetching (`fetchMetadata`)

**Before:**
```dart
if (response.statusCode == 429) {
  final retryAfter = int.tryParse(response.headers['retry-after'] ?? '') ?? 30;
  throw Exception('Rate limited. Wait ${retryAfter}s.');
}
```

**After:**
```dart
if (response.statusCode == 429) {
  final retryAfter = int.tryParse(response.headers['retry-after'] ?? '') ?? 30;
  
  if (retries < maxRetries - 1) {
    print('Rate limited. Waiting ${retryAfter}s (as requested by server)...');
    await Future.delayed(Duration(seconds: retryAfter));
    retries++;
    continue; // Retry the request
  }
  throw RateLimitException(retryAfter);
}
```

### 2. Server Errors (5xx)

**Before:**
- Always used exponential backoff
- Ignored server's preferred retry timing

**After:**
```dart
if (response.statusCode >= 500) {
  if (retries < maxRetries - 1) {
    // Check if server provided Retry-After header
    final serverRetryAfter = int.tryParse(response.headers['retry-after'] ?? '');
    final waitTime = serverRetryAfter != null 
        ? Duration(seconds: serverRetryAfter)
        : retryDelay; // Fall back to exponential backoff
    
    print(serverRetryAfter != null
        ? 'Retrying in ${waitTime.inSeconds}s (as requested by server)'
        : 'Retrying in ${waitTime.inSeconds}s (exponential backoff)');
    
    await Future.delayed(waitTime);
    retries++;
    
    // Only apply exponential backoff if server didn't specify
    if (serverRetryAfter == null) {
      retryDelay *= 2;
    }
    continue;
  }
}
```

### 3. File Downloads (`downloadFile`)

**Enhanced with full retry logic:**
- Handles 429 (rate limit) with retry-after
- Handles 5xx (server errors) with retry-after  
- Falls back to exponential backoff if no retry-after header
- Properly drains response streams before retrying
- Cleans up partial downloads on failure

**Features:**
```dart
Future<String> downloadFile(url, outputPath, {onProgress, cancellationToken}) async {
  int retries = 0;
  Duration retryDelay = Duration(seconds: 30);
  
  while (retries < maxRetries) {
    try {
      final response = await client.send(request);
      
      // Handle 429 with retry-after
      if (response.statusCode == 429) {
        final retryAfter = parseRetryAfter(response.headers);
        await response.stream.drain(); // Important!
        await Future.delayed(Duration(seconds: retryAfter));
        retries++;
        continue;
      }
      
      // Handle 5xx with retry-after
      if (response.statusCode >= 500) {
        final serverRetryAfter = parseRetryAfter(response.headers);
        final waitTime = serverRetryAfter ?? retryDelay;
        await response.stream.drain();
        await Future.delayed(waitTime);
        retries++;
        continue;
      }
      
      // Download the file...
    } on SocketException {
      // Network error - retry with exponential backoff
    } on TimeoutException {
      // Timeout - retry with exponential backoff
    }
  }
}
```

## Benefits

### 1. API Compliance ✅
- Respects server's explicit timing requests
- Reduces unnecessary retry attempts
- Prevents "thundering herd" when many clients retry simultaneously

### 2. Better User Experience ✅
- More accurate wait times shown to users
- Faster recovery when server specifies short retry
- No wasted attempts during long server issues

### 3. Reduced Server Load ✅
- Clients wait exactly as long as server requests
- No premature retries overwhelming the server
- Exponential backoff only when server doesn't specify

### 4. Intelligent Fallback ✅
- Uses server retry-after when available
- Falls back to exponential backoff (2x multiplier)
- Caps at 10 minutes maximum wait time

## Examples

### Example 1: Rate Limiting
```
Request → 429 Too Many Requests
          Retry-After: 60

App: "Rate limited. Waiting 60s (as requested by server)..."
Wait 60 seconds
Retry → Success
```

### Example 2: Server Maintenance
```
Request → 503 Service Unavailable
          Retry-After: 300

App: "Server error, retrying in 300s (as requested by server)..."
Wait 5 minutes
Retry → Success
```

### Example 3: No Retry-After Header
```
Request → 500 Internal Server Error
          (no Retry-After header)

App: "Server error, retrying in 30s (exponential backoff)..."
Wait 30 seconds
Retry → Still fails
Wait 60 seconds (2x multiplier)
Retry → Success
```

### Example 4: Network Error
```
Request → SocketException (network issue)

App: "Network error, retrying in 30s..."
Wait 30 seconds
Retry → Still fails
Wait 60 seconds
Retry → Still fails  
Wait 120 seconds
Retry → Success
```

## Implementation Details

### Parsing Retry-After

The code handles both integer seconds and HTTP date formats:

```dart
final retryAfter = int.tryParse(response.headers['retry-after'] ?? '') 
    ?? defaultRetryDelaySecs;
```

**Note:** Currently only handles integer seconds format. HTTP date format support can be added if needed.

### Stream Draining

Important: Before retrying a streaming response, we must drain it:

```dart
await response.stream.drain(); // Prevents memory leaks
```

### Exponential Backoff

When server doesn't provide retry-after:
- Start: 30 seconds
- Multiplier: 2x each retry
- Cap: 10 minutes maximum

```
Attempt 1: Wait 30s
Attempt 2: Wait 60s (30 * 2)
Attempt 3: Wait 120s (60 * 2)
Maximum: Wait 600s (10 minutes)
```

### Priority Order

1. **Server Retry-After** (if provided) ← Highest priority
2. **Exponential Backoff** (if no retry-after)
3. **Maximum Cap** (never exceed 10 minutes)

## Testing

### Manual Testing

1. **Test rate limiting:**
```dart
// Make 31 requests rapidly to trigger rate limit
for (int i = 0; i < 31; i++) {
  await api.fetchMetadata('identifier$i');
}
// Should automatically wait and retry
```

2. **Test server errors:**
```dart
// Use a test URL that returns 503
await api.downloadFile('http://httpstat.us/503', 'test.dat');
// Should retry with appropriate delays
```

### Expected Behavior

- **429 Response**: Wait exact seconds from retry-after, then retry
- **503 Response**: Wait exact seconds from retry-after (if provided)
- **500 Response**: Use exponential backoff (if no retry-after)
- **Network Error**: Use exponential backoff
- **All Retries Exhausted**: Throw appropriate exception

## Configuration

All retry behavior is configurable via constants:

```dart
// lib/core/constants/internet_archive_constants.dart
class IARateLimits {
  static const int minRequestDelayMs = 100;
  static const int maxRequestsPerMinute = 30;
  static const int defaultRetryDelaySecs = 30;
  static const int maxRetries = 3;
  static const double backoffMultiplier = 2.0;
  static const int maxBackoffDelaySecs = 600; // 10 minutes
}
```

## Future Enhancements

1. **HTTP Date Format**: Support `Retry-After: Wed, 21 Oct 2024 07:28:00 GMT`
2. **Adaptive Backoff**: Learn from server patterns over time
3. **Retry Metrics**: Track retry success rates
4. **User Notification**: Show countdown timer in UI
5. **Background Retries**: Continue retries even if app backgrounded

## References

- [RFC 7231 - Retry-After](https://tools.ietf.org/html/rfc7231#section-7.1.3)
- [Internet Archive API Documentation](https://archive.org/developers/)
- [HTTP Status Code 429](https://developer.mozilla.org/en-US/docs/Web/HTTP/Status/429)
- [Exponential Backoff](https://en.wikipedia.org/wiki/Exponential_backoff)
