# Startup Sequence Documentation

This document details the startup processes for both the mobile app and CLI application, including the order of operations, error handling strategies, and design decisions made to ensure resilience.

## Mobile App (Flutter) Startup Sequence

### Phase 1: Flutter Framework Initialization

**File:** `mobile/flutter/lib/main.dart` - `main()` function

1. **Flutter Binding Initialization**
   ```dart
   WidgetsFlutterBinding.ensureInitialized();
   ```
   - Ensures Flutter framework is ready
   - Must complete before any platform-specific operations
   - **Error Handling:** Crashes if this fails (framework-level issue)

2. **System UI Configuration**
   ```dart
   await SystemChrome.setPreferredOrientations([...]);
   SystemChrome.setSystemUIOverlayStyle(...);
   ```
   - Sets device orientations (portrait and landscape)
   - Configures status bar and navigation bar appearance
   - **Error Handling:** Non-critical, app continues if this fails
   - **Timeout:** Platform-specific, typically < 100ms

3. **Run App Widget**
   ```dart
   runApp(const IAGetMobileApp());
   ```
   - Mounts the root widget tree
   - **No async operations** - synchronous mount

### Phase 2: Provider Initialization

**File:** `mobile/flutter/lib/main.dart` - `IAGetMobileApp.build()`

Providers are created in declaration order with `lazy: false` for critical services:

1. **IaGetService** (`lazy: false`)
   - Created immediately when widget builds
   - Constructor is synchronous
   - Actual initialization happens later in `initialize()` method
   - **Design:** Eager creation ensures service exists before first frame

2. **BackgroundDownloadService** (`lazy: false`)
   - Created immediately after IaGetService
   - Constructor is synchronous
   - Actual initialization happens later in `initialize()` method
   - **Design:** Eager creation for background download capability

3. **DeepLinkService** (`lazy: true` by default)
   - Created on first access via `context.read<DeepLinkService>()`
   - **Design:** Only needed when handling deep links

### Phase 3: App Initialization

**File:** `mobile/flutter/lib/main.dart` - `_AppInitializerState`

1. **Check Onboarding Status** (in `initState`)
   ```dart
   _initializeApp() -> _checkOnboardingStatus()
   ```
   - Checks SharedPreferences for onboarding completion flag
   - **Timeout:** 5 seconds with fallback to `false`
   - **Error Handling:** Catches all exceptions, defaults to not showing onboarding
   - **Why First:** Fast local operation, determines UI flow
   - **Thread:** Main isolate, async operation

2. **Post-Frame Service Initialization** (after first frame)
   ```dart
   WidgetsBinding.instance.addPostFrameCallback((_) async {
     await _initializeServices();
   })
   ```
   - Deferred until after first frame is painted
   - Ensures widget tree is stable and mounted
   - **Sequential Initialization:**

   a. **BackgroundDownloadService.initialize()**
      - Sets up method channel with Android native code
      - Starts periodic timers (500ms for status, 10s for retries)
      - **Timeout:** No explicit timeout, but operations are non-blocking
      - **Error Handling:** Logs errors, service may be partially functional
      - **Why First:** No dependencies on other services

   b. **DeepLinkService.initialize()**
      - Retrieves initial deep link if app opened via link
      - Sets up stream listener for future deep links
      - **Timeout:** 3 seconds for initial link fetch
      - **Error Handling:** Non-critical, app continues without deep linking
      - **Why Second:** Independent of other services

   c. **Set Deep Link Handler**
      ```dart
      deepLinkService.onArchiveLinkReceived = (identifier) { ... }
      ```
      - Handler validates IaGetService is initialized before use
      - **Safety Check:** Prevents calling uninitialized service
      - **Why Last:** Depends on services being initialized

   d. **Request Notification Permissions** (fire-and-forget)
      - Non-blocking async call
      - Checks if permission already granted
      - Silently ignored on Android < 13
      - **Error Handling:** Catches all exceptions, logs and continues
      - **Why Last:** Completely independent, non-critical

### Phase 4: FFI Initialization (On-Demand)

**File:** `mobile/flutter/lib/services/ia_get_service.dart` - `initialize()`

Called from `HomeScreen` when it mounts:

1. **Load Native Library** (if not already loaded)
   - Happens in Dart FFI `dylib` getter
   - Uses `DynamicLibrary.process()` for Android
   - Library was pre-loaded by MainActivity.kt
   - **Error Handling:** Throws if library not available

2. **Symbol Lookup** (lazy getters)
   - FFI function pointers are lazy getters
   - Lookup happens on first call to each function
   - **Error Handling:** Catches lookup failures, returns error codes

3. **Call `ia_get_init()`**
   - Validates global Rust state (mutexes, runtime)
   - Returns error code if validation fails
   - **Timeout:** 5 seconds via Future.timeout
   - **Error Handling:** Sets service error state, notifies listeners

### Android Native Initialization

**File:** `mobile/flutter/android/.../MainActivity.kt` - `configureFlutterEngine()`

1. **Load Native Library**
   ```kotlin
   System.loadLibrary("ia_get_mobile")
   ```
   - Called before Dart engine starts
   - Ensures library available for FFI
   - **Error Handling:** 
     - Catches `UnsatisfiedLinkError` (library not found)
     - Catches general exceptions
     - Logs error but allows app to continue
   - **Why Early:** Must happen before Dart FFI accesses it

2. **Setup Method Channel**
   - Creates bidirectional communication with Dart
   - Used by BackgroundDownloadService
   - **Error Handling:** None needed, synchronous operation

### Rust FFI Global State Initialization

**File:** `src/interface/ffi.rs` - Static lazy initialization

Happens on first access to any global via `lazy_static`:

1. **RUNTIME** - Tokio async runtime
   - Created on first access
   - **Error Handling:** Panics if creation fails (unrecoverable)

2. **SESSIONS** - Download session tracker
   - Arc<Mutex<HashMap>> for thread-safe access
   - **Error Handling:** Lock poisoning handled per-operation

3. **METADATA_CACHE** - Archive metadata cache
   - Arc<Mutex<HashMap>> for thread-safe access
   - **Error Handling:** Lock poisoning handled per-operation

4. **CIRCUIT_BREAKER** - Failure tracking
   - Initialized with threshold=3, timeout=30s
   - **Error Handling:** Lock poisoning handled per-operation

5. **REQUEST_TRACKER** - Request deduplication
   - Prevents duplicate concurrent requests
   - **Error Handling:** Lock poisoning handled per-operation

6. **PERFORMANCE_METRICS** - Telemetry
   - Tracks request counts and timing
   - **Error Handling:** Lock poisoning handled per-operation

## CLI Application Startup Sequence

### Phase 1: Tokio Runtime Creation

**File:** `src/main.rs` - `#[tokio::main]`

1. **Create Tokio Runtime**
   - Automatically created by `#[tokio::main]` macro
   - Multi-threaded runtime for async operations
   - **Error Handling:** Panics if runtime creation fails

### Phase 2: Main Function Execution

**File:** `src/main.rs` - `main()`

1. **Parse CLI Arguments**
   - Uses clap to parse command-line arguments
   - **Error Handling:** Clap handles errors, exits with usage message

2. **Detect GUI Availability** (if no identifier provided)
   ```rust
   if can_use_gui() { ... }
   ```
   - Checks environment variables for GUI availability
   - Platform-specific detection
   - **No errors:** Returns boolean

3. **Launch Mode** (GUI or CLI)

   **GUI Mode:**
   - Initialize logger with `env_logger::try_init()`
   - **Error Handling:** Logs warning if logger init fails
   - Create eframe window with configuration
   - **Error Handling:** Falls back to CLI if GUI launch fails
   - **Mutex Usage:** Safe error handling when checking mode switch state

   **CLI Mode:**
   - Launches interactive menu
   - Uses tokio async for I/O operations

### Error Handling Philosophy

1. **Critical Errors** (Must succeed):
   - Flutter binding initialization
   - Tokio runtime creation
   - Widget tree mounting

2. **Service Errors** (Recoverable):
   - FFI initialization - Shows error UI with retry
   - Service initialization - Logs error, continues
   - Deep link setup - Non-critical, continues without

3. **User Errors** (Informational):
   - Permission denials - Logged, app continues
   - Network failures - Handled by circuit breaker
   - Invalid input - User-facing error messages

### Race Condition Prevention

1. **Mounted Checks**: All `setState()` calls check `mounted` first
2. **Service Validation**: Deep link handler checks service initialization
3. **Sequential Initialization**: Critical services initialized in order
4. **Timeout Guards**: All network and storage operations have timeouts
5. **Mutex Error Handling**: All mutex lock operations handle poisoning

### Timeout Strategy

- **Local Operations**: 5 seconds (onboarding check, FFI init)
- **Network Operations**: 3 seconds (initial deep link fetch)
- **No Timeout**: Fire-and-forget operations (notification permissions)

### Memory Safety

1. **Widget Lifecycle**: Check `mounted` before `setState()`
2. **Stream Subscriptions**: Canceled in `dispose()`
3. **Timers**: Canceled in `dispose()`
4. **FFI Resources**: Freed in `finally` blocks
5. **Isolate Safety**: Platform channels handle thread safety

## Testing Recommendations

### Mobile App
1. Test cold start with airplane mode (network unavailable)
2. Test with missing native library (wrong architecture)
3. Test deep link handling before services ready
4. Test rapid app switching (background/foreground)
5. Test on Android 5.0 (API 21) for minimum support

### CLI App
1. Test in SSH session (headless)
2. Test GUI fallback when X11 unavailable
3. Test Ctrl+C signal handling
4. Test concurrent operations

## Future Improvements

1. **Health Monitoring**: Add startup telemetry to track initialization times
2. **Retry Logic**: Implement exponential backoff for FFI initialization
3. **Graceful Degradation**: Allow partial functionality when FFI unavailable
4. **Startup Profiling**: Measure and optimize cold start time
5. **Pre-warming**: Consider initializing services in background thread
