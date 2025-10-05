# Flutter Architecture Improvements

## Overview
This document describes the improvements made to the Flutter app's architecture, organization, and Internet Archive API compliance.

## Changes Made

### 1. Fixed Critical Imports (Commit: 958d94b)
**Problem:** After removing FFI, several files still referenced the deleted `IaGetSimpleService`.

**Fixed:**
- `mobile/flutter/lib/providers/download_provider.dart` - Now uses `ArchiveService`
- `mobile/flutter/lib/services/background_download_service.dart` - Now uses `ArchiveService`
- `mobile/flutter/lib/models/download_progress.dart` - Updated comment
- Removed duplicate `archive_service_dart.dart`

### 2. Internet Archive API Compliance

#### Created Comprehensive Constants (`core/constants/internet_archive_constants.dart`)

**IAEndpoints** - All API endpoints in one place:
- Base URL, metadata, details, download
- Search APIs (simple and advanced)
- Services and thumbnail endpoints

**IARateLimits** - Rate limiting configuration:
- Min request delay: 100ms
- Max requests per minute: 30 (IA recommended)
- Exponential backoff with 2x multiplier
- Max backoff: 10 minutes

**IAHeaders** - Proper HTTP headers:
```dart
IAHeaders.standard(appVersion) // Returns complete header map
// Includes:
// - User-Agent: InternetArchiveHelper/1.6.0 (Flutter; GitHub URL)
// - Accept: application/json
// - Cache-Control: no-cache
// - DNT: 1 (Do Not Track)
```

**IASearchParams** - Search configuration:
- Default 20 results per page
- Max 10,000 results
- Standard fields (identifier, title, description, mediatype, etc.)
- Sort options

**IAUtils** - Utility functions:
- Identifier validation
- URL building for metadata, downloads, thumbnails
- Search URL construction with parameters

#### Enhanced `internet_archive_api.dart`

**Before:**
- Hard-coded values scattered throughout
- Simple retry logic
- Basic headers

**After:**
- Uses constants for all configurations
- Exponential backoff with proper caps
- Identifier validation
- Better error messages using IAErrorMessages
- Proper User-Agent header with contact info
- Respects Retry-After header from IA

**Example:**
```dart
// Old
headers: {
  'User-Agent': 'ia-get-flutter/1.6.0',
  'Accept': 'application/json',
  // ...
}

// New
headers: IAHeaders.standard(_appVersion)
// Generates proper User-Agent with contact info
// 'InternetArchiveHelper/1.6.0 (Flutter; https://github.com/Gameaday/ia-get-cli)'
```

### 3. Core Architecture Setup

#### Created `core/` directory structure:
```
core/
├── constants/
│   └── internet_archive_constants.dart  # All IA API constants
├── errors/
│   └── ia_exceptions.dart               # Custom exceptions
└── network/
    └── (future: HTTP client, interceptors)
```

#### Custom Exceptions (`core/errors/ia_exceptions.dart`)

Created strongly-typed exceptions for better error handling:

- `ItemNotFoundException` - Item not found (404)
- `AccessForbiddenException` - Access denied (403)
- `RateLimitException` - Rate limited (429) with retry-after
- `NetworkException` - Network errors
- `ServerException` - Server errors (5xx) with status code
- `TimeoutException` - Request timeout
- `InvalidIdentifierException` - Invalid identifier format
- `DownloadException` - Download failures
- `ChecksumException` - Checksum validation failures with expected/actual
- `ParsingException` - JSON parsing errors

**Benefits:**
- Type-safe error handling
- Clear error messages for users
- Easier debugging with stack traces
- Consistent error handling across app

## Internet Archive API Best Practices Implemented

### ✅ 1. Proper User-Agent Header
- Includes app name, version, and contact URL
- Format: `AppName/Version (Platform; Contact URL)`
- Helps IA track usage and contact if issues arise

### ✅ 2. Rate Limiting
- Enforces minimum 100ms between requests
- Tracks requests per minute
- Limits to 30 requests/minute (IA recommendation)
- Can check rate health with `isRateHealthy()`

### ✅ 3. Exponential Backoff
- Starts with 30s delay
- Doubles with each retry (2x multiplier)
- Caps at 10 minutes
- Maximum 3 retry attempts

### ✅ 4. Proper Error Handling
- Handles all IA-specific status codes (404, 403, 429, 5xx)
- Respects Retry-After header for 429 responses
- Clear error messages for each case
- Retries transient errors (timeouts, network issues)

### ✅ 5. Identifier Validation
- Validates before making requests
- Checks length (3-100 characters)
- Validates characters (alphanumeric, -, _, .)
- Prevents invalid API calls

### ✅ 6. URL Construction
- Utility functions for building URLs
- Consistent URL formats
- Handles S3-like download URLs
- Search URL with proper encoding

## Code Organization Improvements

### Before:
```
lib/
├── models/
├── providers/
├── screens/
├── services/    # Everything mixed together
├── utils/
└── widgets/
```

### After (In Progress):
```
lib/
├── core/                    # ✅ Created
│   ├── constants/          # ✅ IA API constants
│   ├── errors/             # ✅ Custom exceptions
│   └── network/            # 🔄 Future: HTTP client
├── features/               # 🔄 To be created
│   ├── search/
│   ├── archive_details/
│   ├── download/
│   └── settings/
├── models/                 # Existing
├── providers/             # Existing
├── screens/               # Existing
├── services/              # Existing
├── utils/                 # Existing
└── widgets/               # Existing
```

## Benefits

### For Development
- ✅ Clear separation of concerns
- ✅ Easy to find related code
- ✅ Centralized API knowledge
- ✅ Better testability
- ✅ Type-safe error handling
- ✅ Consistent patterns

### For Maintenance
- ✅ Single source of truth for constants
- ✅ Easy to update API endpoints
- ✅ Clear documentation
- ✅ Follows industry best practices
- ✅ Scalable architecture

### For Users
- ✅ Better error messages
- ✅ More reliable API interactions
- ✅ Respects IA server load
- ✅ Faster responses (good rate limiting)
- ✅ Proper retry logic

### For Internet Archive
- ✅ Proper User-Agent identification
- ✅ Respects rate limits
- ✅ Implements backoff strategies
- ✅ Reduces unnecessary load
- ✅ Easy to contact if issues

## Next Steps

### Phase 1: Complete Core Setup
- [x] Create constants
- [x] Create exceptions
- [ ] Create HTTP client wrapper with logging
- [ ] Add request/response interceptors
- [ ] Add caching layer

### Phase 2: Feature-Based Organization
- [ ] Create `features/` directory
- [ ] Move search functionality
- [ ] Move archive details functionality
- [ ] Move download functionality
- [ ] Move settings functionality

### Phase 3: Repository Pattern
- [ ] Create repositories for each feature
- [ ] Separate data sources (remote API, local cache)
- [ ] Implement proper domain layer

### Phase 4: Testing
- [ ] Unit tests for utilities
- [ ] Unit tests for repositories
- [ ] Widget tests for screens
- [ ] Integration tests for critical flows

### Phase 5: Advanced Features
- [ ] Implement caching
- [ ] Add offline support
- [ ] Improve search with filters
- [ ] Add batch operations
- [ ] Implement download queue management

## References

### Internet Archive API Documentation
- Metadata API: https://archive.org/developers/md-read.html
- Search API: https://archive.org/developers/search.html
- Rate Limiting: https://archive.org/services/docs/api/ratelimiting.html

### Flutter Best Practices
- Clean Architecture: https://blog.cleancoder.com/uncle-bob/2012/08/13/the-clean-architecture.html
- Feature-based organization: Common Flutter pattern
- Repository pattern: https://developer.android.com/topic/architecture/data-layer

## Summary

These changes establish a solid foundation for the Flutter app:
1. ✅ Fixed critical import errors
2. ✅ Implemented comprehensive IA API compliance
3. ✅ Created proper error handling
4. ✅ Centralized all IA API knowledge
5. ✅ Set up core architecture
6. 🔄 Planned feature-based reorganization

The app now follows industry best practices, respects Internet Archive's guidelines, and has a scalable architecture for future growth.
