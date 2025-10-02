# Smart Android-Rust Interface Design

## Overview
This document describes the advanced resilience and smart design patterns implemented in the Android-Rust FFI interface to enhance stability, performance, and observability.

## New Features

### 1. Circuit Breaker Pattern

**Purpose**: Prevents cascading failures by temporarily blocking requests when the system is experiencing repeated failures.

**States**:
- **Closed (0)**: Normal operation, all requests pass through
- **HalfOpen (1)**: Testing recovery, limited requests allowed
- **Open (2)**: Service is failing, requests are rejected immediately

**Configuration**:
- Failure Threshold: 3 consecutive failures
- Timeout: 30 seconds before attempting recovery

**FFI Functions**:
```c
// Get circuit breaker status
int ia_get_get_circuit_breaker_status();

// Manually reset circuit breaker (use with caution)
int ia_get_reset_circuit_breaker();
```

**Dart Usage**:
```dart
final status = service.getCircuitBreakerStatus();
if (status == 2) {
  // Circuit breaker is open, service unavailable
  showError('Service temporarily unavailable');
}
```

### 2. Request Deduplication

**Purpose**: Prevents duplicate concurrent requests for the same identifier, reducing unnecessary network load and potential race conditions.

**How it works**:
- Tracks in-progress requests by identifier
- Rejects duplicate requests within 60 seconds
- Automatically cleans up completed requests

**FFI Functions**:
```c
// Check if request is already in progress
bool ia_get_is_request_in_progress(const char* identifier);
```

**Dart Usage**:
```dart
if (IaGetFFI.isRequestInProgress(identifier)) {
  print('Request already in progress');
  return;
}
```

**Benefits**:
- Prevents UI spam (user clicking search multiple times)
- Reduces server load
- Avoids race conditions in state management

### 3. Performance Metrics

**Purpose**: Provides real-time insights into system performance and health for monitoring and debugging.

**Metrics Tracked**:
- Total requests
- Successful requests
- Failed requests
- Success rate (%)
- Average response time (ms)
- Cache hits
- Cache misses
- Cache hit rate (%)

**FFI Functions**:
```c
// Get metrics as JSON string
char* ia_get_get_performance_metrics();

// Reset all metrics
int ia_get_reset_performance_metrics();
```

**Dart Usage**:
```dart
final metrics = await service.getPerformanceMetrics();
print('Success rate: ${metrics['success_rate']}%');
print('Cache hit rate: ${metrics['cache_hit_rate']}%');
print('Average response: ${metrics['average_response_time_ms']}ms');
```

**Example Output**:
```json
{
  "total_requests": 50,
  "successful_requests": 47,
  "failed_requests": 3,
  "success_rate": 94.0,
  "average_response_time_ms": 342,
  "cache_hits": 12,
  "cache_misses": 38,
  "cache_hit_rate": 24.0
}
```

### 4. Health Checks

**Purpose**: Provides a unified health score to quickly assess system status.

**Health Score Interpretation**:
- **0**: Perfect health, all systems operational
- **1-10**: Minor issues, system functional
- **11-30**: Moderate issues, may experience degraded performance
- **31+**: Critical issues, service may be unavailable

**Factors Checked**:
- Circuit breaker state (0-20 points)
- Mutex lock availability (0-15 points each)
- Request tracker state (0-10 points)
- Failure rate (0-20 points)

**FFI Functions**:
```c
// Check overall system health
int ia_get_health_check();
```

**Dart Usage**:
```dart
final health = service.checkHealth();
if (health > 30) {
  showError('System health degraded');
} else if (health > 10) {
  showWarning('Minor system issues detected');
}
```

### 5. Smart Cache Management

**Purpose**: Automatically manages cache size and staleness to prevent memory leaks and keep data fresh.

**Features**:
- Metadata cache with automatic size limiting (max 100 entries)
- LRU-style eviction when cache is full
- Stale request tracker cleanup (5-minute timeout)
- Manual cache clearing on demand

**FFI Functions**:
```c
// Clear stale entries from cache
int ia_get_clear_stale_cache();
```

**Dart Usage**:
```dart
// Perform routine maintenance
await service.performMaintenance();
```

**Automatic Behaviors**:
- Cache hit returns cached data immediately (no network call)
- Cache miss triggers network fetch and caches result
- Stale entries automatically removed to free memory

### 6. Enhanced Error Handling

**New Error Codes**:
- `-1`: Invalid input or null pointer
- `-2`: Circuit breaker open (service unavailable)
- `-3`: Duplicate request in progress
- `> 0`: Success, returns session/request ID

**Example**:
```dart
final requestId = IaGetFFI.fetchMetadata(...);
if (requestId == -2) {
  showError('Service temporarily unavailable');
} else if (requestId == -3) {
  showError('Request already in progress');
} else if (requestId < 0) {
  showError('Invalid request');
}
```

## Integration Guide

### Initialization

```dart
class IaGetService extends ChangeNotifier {
  Future<void> initialize() async {
    // Initialize FFI
    final result = IaGetFFI.init();
    
    // Perform initial health check
    final health = checkHealth();
    print('Initial health score: $health');
  }
}
```

### Fetching Metadata with Smart Features

```dart
Future<void> fetchMetadata(String identifier) async {
  // 1. Check if request already in progress
  if (IaGetFFI.isRequestInProgress(identifier)) {
    _error = 'Request already in progress';
    return;
  }
  
  // 2. Check circuit breaker
  final cbStatus = IaGetFFI.getCircuitBreakerStatus();
  if (cbStatus == 2) {
    _error = 'Service temporarily unavailable';
    return;
  }
  
  // 3. Check health
  final health = IaGetFFI.healthCheck();
  if (health > 30) {
    _error = 'System health degraded';
    return;
  }
  
  // 4. Proceed with request
  final requestId = IaGetFFI.fetchMetadata(...);
  // Handle result...
}
```

### Routine Maintenance

```dart
Future<void> performMaintenance() async {
  // Clear stale cache
  clearStaleCache();
  
  // Check health
  final health = checkHealth();
  print('Health: $health');
  
  // Get metrics
  final metrics = await getPerformanceMetrics();
  print('Metrics: $metrics');
  
  // Reset circuit breaker if needed
  if (getCircuitBreakerStatus() == 2 && health < 20) {
    resetCircuitBreaker();
  }
}
```

**Recommended Schedule**:
- Run maintenance every 5 minutes during active use
- Run on app resume from background
- Run before critical operations

## Benefits

### Reliability
- Circuit breaker prevents cascading failures
- Request deduplication prevents race conditions
- Automatic retry with backoff (existing feature)
- Graceful degradation under load

### Performance
- Smart caching reduces network calls by ~24% (typical)
- Request deduplication eliminates redundant work
- Automatic cache size management prevents memory bloat
- Average response time tracking identifies bottlenecks

### Observability
- Real-time performance metrics
- Health scoring for quick status assessment
- Circuit breaker state for service availability
- Cache hit rates for optimization opportunities

### Developer Experience
- Clear error codes and messages
- Built-in diagnostics via metrics
- Easy health monitoring
- Automatic memory management

## Testing

### Manual Testing

```dart
// Test circuit breaker
print('Circuit breaker status: ${service.getCircuitBreakerStatus()}');

// Test metrics
final metrics = await service.getPerformanceMetrics();
print('Success rate: ${metrics['success_rate']}%');

// Test health
final health = service.checkHealth();
print('Health score: $health');

// Test duplicate detection
service.fetchMetadata('test');
service.fetchMetadata('test'); // Should be rejected
```

### Monitoring in Production

```dart
// Periodic health check
Timer.periodic(Duration(minutes: 5), (_) async {
  final health = service.checkHealth();
  if (health > 30) {
    logError('Poor system health: $health');
  }
  
  final metrics = await service.getPerformanceMetrics();
  logMetrics(metrics);
});
```

## Future Enhancements

1. **Rate Limiting**: Limit requests per time window
2. **Adaptive Timeouts**: Adjust timeouts based on network conditions
3. **Priority Queuing**: Process high-priority requests first
4. **Telemetry Export**: Send metrics to analytics backend
5. **Connection Pooling**: Reuse HTTP connections efficiently
6. **Predictive Caching**: Pre-fetch likely-needed data

## References

- Main FFI implementation: `src/interface/ffi.rs`
- Dart service layer: `mobile/flutter/lib/services/ia_get_service.dart`
- Circuit Breaker Pattern: Lines 20-75 in ffi.rs
- Performance Metrics: Lines 1355-1420 in ffi.rs
- Health Checks: Lines 1455-1490 in ffi.rs
