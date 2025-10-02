//! FFI Interface for Mobile Platforms
//!
//! This module provides C-compatible bindings for the core ia-get functionality
//! to enable integration with mobile applications (Flutter, React Native, etc.)

use indicatif::ProgressBar;
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::ptr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::runtime::Runtime;

use crate::core::archive::fetch_json_metadata;
use crate::core::session::ArchiveMetadata;
use crate::infrastructure::http::HttpClientFactory;
use crate::utilities::filters::parse_size_string;

/// Circuit breaker state for resilience
#[derive(Debug, Clone, Copy, PartialEq)]
enum CircuitBreakerState {
    Closed,   // Normal operation
    Open,     // Failing, rejecting requests
    HalfOpen, // Testing if service recovered
}

/// Circuit breaker for handling repeated failures
#[derive(Debug)]
struct CircuitBreaker {
    state: CircuitBreakerState,
    failure_count: u32,
    last_failure_time: Option<Instant>,
    failure_threshold: u32,
    timeout: Duration,
}

impl CircuitBreaker {
    fn new(failure_threshold: u32, timeout_secs: u64) -> Self {
        Self {
            state: CircuitBreakerState::Closed,
            failure_count: 0,
            last_failure_time: None,
            failure_threshold,
            timeout: Duration::from_secs(timeout_secs),
        }
    }

    fn can_attempt(&mut self) -> bool {
        match self.state {
            CircuitBreakerState::Closed => true,
            CircuitBreakerState::Open => {
                // Check if enough time has passed to try again
                if let Some(last_failure) = self.last_failure_time {
                    if last_failure.elapsed() > self.timeout {
                        self.state = CircuitBreakerState::HalfOpen;
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            CircuitBreakerState::HalfOpen => true,
        }
    }

    fn record_success(&mut self) {
        self.failure_count = 0;
        self.state = CircuitBreakerState::Closed;
        self.last_failure_time = None;
    }

    fn record_failure(&mut self) {
        self.failure_count += 1;
        self.last_failure_time = Some(Instant::now());

        if self.failure_count >= self.failure_threshold {
            self.state = CircuitBreakerState::Open;
        }
    }
}

/// Request deduplication tracker to prevent duplicate requests
#[derive(Debug)]
struct RequestTracker {
    identifier: String,
    in_progress: bool,
    last_request_time: Instant,
}

/// Performance metrics for monitoring
#[derive(Debug, Clone)]
struct PerformanceMetrics {
    total_requests: u64,
    successful_requests: u64,
    failed_requests: u64,
    average_response_time_ms: u64,
    cache_hits: u64,
    cache_misses: u64,
}

// Simple session structure for FFI
#[derive(Debug)]
struct FfiSession {
    identifier: String,
    output_dir: String,
    concurrent_downloads: u32,
    auto_decompress: bool,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl FfiSession {
    fn new(
        identifier: String,
        output_dir: String,
        concurrent_downloads: u32,
        auto_decompress: bool,
    ) -> Self {
        Self {
            identifier,
            output_dir,
            concurrent_downloads,
            auto_decompress,
            created_at: chrono::Utc::now(),
        }
    }

    /// Get the archive identifier for this session
    fn get_identifier(&self) -> &str {
        &self.identifier
    }

    /// Get the output directory for this session
    fn get_output_dir(&self) -> &str {
        &self.output_dir
    }

    /// Get the number of concurrent downloads for this session
    fn get_concurrent_downloads(&self) -> u32 {
        self.concurrent_downloads
    }

    /// Check if auto-decompression is enabled for this session
    fn is_auto_decompress_enabled(&self) -> bool {
        self.auto_decompress
    }

    /// Get the creation timestamp of this session
    fn get_created_at(&self) -> &chrono::DateTime<chrono::Utc> {
        &self.created_at
    }
}

// Global state management for mobile platforms
lazy_static::lazy_static! {
    static ref RUNTIME: Arc<Runtime> = Arc::new(
        Runtime::new().expect("Failed to create Tokio runtime")
    );
    static ref SESSIONS: Arc<Mutex<HashMap<i32, FfiSession>>> = Arc::new(Mutex::new(HashMap::new()));
    static ref METADATA_CACHE: Arc<Mutex<HashMap<String, ArchiveMetadata>>> = Arc::new(Mutex::new(HashMap::new()));
    static ref NEXT_SESSION_ID: Arc<Mutex<i32>> = Arc::new(Mutex::new(1));
    static ref CIRCUIT_BREAKER: Arc<Mutex<CircuitBreaker>> = Arc::new(Mutex::new(CircuitBreaker::new(3, 30)));
    static ref REQUEST_TRACKER: Arc<Mutex<HashMap<String, RequestTracker>>> = Arc::new(Mutex::new(HashMap::new()));
    static ref PERFORMANCE_METRICS: Arc<Mutex<PerformanceMetrics>> = Arc::new(Mutex::new(PerformanceMetrics {
        total_requests: 0,
        successful_requests: 0,
        failed_requests: 0,
        average_response_time_ms: 0,
        cache_hits: 0,
        cache_misses: 0,
    }));
}

/// Callback function type for progress updates
pub type ProgressCallback = extern "C" fn(progress: f64, message: *const c_char, user_data: usize);

/// Callback function type for completion
pub type CompletionCallback =
    extern "C" fn(success: bool, error_message: *const c_char, user_data: usize);

/// FFI-compatible error codes
#[repr(C)]
pub enum IaGetErrorCode {
    Success = 0,
    InvalidInput = 1,
    NetworkError = 2,
    FileSystemError = 3,
    ParseError = 4,
    UnknownError = 5,
}

/// FFI-compatible download configuration
#[repr(C)]
pub struct FfiDownloadConfig {
    pub concurrent_downloads: u32,
    pub max_file_size: u64, // 0 = no limit
    pub output_directory: *const c_char,
    pub include_formats: *const c_char, // comma-separated
    pub exclude_formats: *const c_char, // comma-separated
    pub dry_run: bool,
    pub verbose: bool,
    pub auto_decompress: bool,
    pub resume_downloads: bool,
    pub verify_checksums: bool,
}

/// FFI-compatible file information
#[repr(C)]
pub struct FfiFileInfo {
    pub name: *const c_char,
    pub size: u64,
    pub format: *const c_char,
    pub download_url: *const c_char,
    pub md5: *const c_char,
    pub sha1: *const c_char,
    pub selected: bool, // For UI selection state
}

/// FFI-compatible archive metadata
#[repr(C)]
pub struct FfiArchiveMetadata {
    pub identifier: *const c_char,
    pub title: *const c_char,
    pub description: *const c_char,
    pub creator: *const c_char,
    pub date: *const c_char,
    pub total_files: u32,
    pub total_size: u64,
    pub files: *const FfiFileInfo,
    pub files_count: u32,
}

/// FFI-compatible download progress information
#[repr(C)]
pub struct FfiDownloadProgress {
    pub session_id: i32,
    pub overall_progress: f64, // 0.0 to 1.0
    pub current_file: *const c_char,
    pub current_file_progress: f64, // 0.0 to 1.0
    pub download_speed: u64,        // bytes per second
    pub eta_seconds: u64,           // estimated time remaining
    pub completed_files: u32,
    pub total_files: u32,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
}

/// Generate next session ID
fn next_session_id() -> i32 {
    match NEXT_SESSION_ID.lock() {
        Ok(mut next_id) => {
            let id = *next_id;
            *next_id += 1;
            id
        }
        Err(e) => {
            eprintln!("Failed to acquire session ID lock: {}", e);
            // Return a fallback ID based on timestamp to avoid crashes
            use std::time::{SystemTime, UNIX_EPOCH};
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i32;
            timestamp.wrapping_rem(1000000) // Use timestamp as fallback
        }
    }
}

/// Initialize the FFI interface
/// Must be called once before using any other functions
#[no_mangle]
pub extern "C" fn ia_get_init() -> IaGetErrorCode {
    // Initialize logging for mobile debugging (optional)
    #[cfg(debug_assertions)]
    {
        // Simple logging initialization without external dependencies
        println!("ia-get FFI initialized");
    }

    IaGetErrorCode::Success
}

/// Cleanup the FFI interface
/// Should be called when the application is shutting down
#[no_mangle]
pub extern "C" fn ia_get_cleanup() {
    // Currently no cleanup needed, but provides future extensibility
}

/// Fetch archive metadata asynchronously
/// Returns a session ID that can be used to cancel the operation
///
/// # Safety
/// The `identifier` parameter must be a valid null-terminated C string pointer.
/// The caller must ensure the string remains valid for the duration of the call.
#[no_mangle]
pub unsafe extern "C" fn ia_get_fetch_metadata(
    identifier: *const c_char,
    progress_callback: ProgressCallback,
    completion_callback: CompletionCallback,
    user_data: usize,
) -> c_int {
    if identifier.is_null() {
        eprintln!("ia_get_fetch_metadata: identifier is null");
        return -1;
    }

    let identifier_str = match CStr::from_ptr(identifier).to_str() {
        Ok(s) => {
            if s.is_empty() {
                eprintln!("ia_get_fetch_metadata: identifier is empty");
                return -1;
            }
            s.to_string()
        }
        Err(e) => {
            eprintln!("ia_get_fetch_metadata: invalid UTF-8 in identifier: {}", e);
            return -1;
        }
    };

    // Update metrics
    if let Ok(mut metrics) = PERFORMANCE_METRICS.lock() {
        metrics.total_requests += 1;
    }

    // Check circuit breaker
    let can_proceed = match CIRCUIT_BREAKER.lock() {
        Ok(mut breaker) => breaker.can_attempt(),
        Err(e) => {
            eprintln!(
                "ia_get_fetch_metadata: failed to check circuit breaker: {}",
                e
            );
            false
        }
    };

    if !can_proceed {
        eprintln!("ia_get_fetch_metadata: circuit breaker is open, rejecting request");
        if let Ok(mut metrics) = PERFORMANCE_METRICS.lock() {
            metrics.failed_requests += 1;
        }
        return -2; // Special code indicating circuit breaker rejection
    }

    // Check for duplicate requests
    let should_proceed = match REQUEST_TRACKER.lock() {
        Ok(mut tracker) => {
            if let Some(request) = tracker.get(&identifier_str) {
                if request.in_progress
                    && request.last_request_time.elapsed() < Duration::from_secs(60)
                {
                    eprintln!(
                        "ia_get_fetch_metadata: request already in progress for '{}'",
                        identifier_str
                    );
                    false
                } else {
                    // Update existing request
                    tracker.insert(
                        identifier_str.clone(),
                        RequestTracker {
                            identifier: identifier_str.clone(),
                            in_progress: true,
                            last_request_time: Instant::now(),
                        },
                    );
                    true
                }
            } else {
                // New request
                tracker.insert(
                    identifier_str.clone(),
                    RequestTracker {
                        identifier: identifier_str.clone(),
                        in_progress: true,
                        last_request_time: Instant::now(),
                    },
                );
                true
            }
        }
        Err(e) => {
            eprintln!(
                "ia_get_fetch_metadata: failed to check request tracker: {}",
                e
            );
            true // Proceed anyway if tracker unavailable
        }
    };

    if !should_proceed {
        return -3; // Special code indicating duplicate request
    }

    // Check cache first
    let cached_result = match METADATA_CACHE.lock() {
        Ok(cache) => {
            if cache.contains_key(&identifier_str) {
                if let Ok(mut metrics) = PERFORMANCE_METRICS.lock() {
                    metrics.cache_hits += 1;
                    metrics.successful_requests += 1;
                }
                true
            } else {
                if let Ok(mut metrics) = PERFORMANCE_METRICS.lock() {
                    metrics.cache_misses += 1;
                }
                false
            }
        }
        Err(_) => false,
    };

    if cached_result {
        #[cfg(debug_assertions)]
        println!("Metadata cache hit for '{}'", identifier_str);

        // Mark request as complete
        if let Ok(mut tracker) = REQUEST_TRACKER.lock() {
            if let Some(request) = tracker.get_mut(&identifier_str) {
                request.in_progress = false;
            }
        }

        // Return positive session ID to indicate cached success
        return next_session_id();
    }

    let session_id = next_session_id();
    let runtime = RUNTIME.clone();
    let request_start_time = Instant::now();

    #[cfg(debug_assertions)]
    println!(
        "Starting metadata fetch for '{}' with session ID {}",
        identifier_str, session_id
    );

    // Spawn async operation in background thread
    std::thread::spawn(move || {
        runtime.block_on(async move {
            // Create HTTP client with retry capability
            let enhanced_client = match HttpClientFactory::for_metadata_requests() {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("Failed to create HTTP client: {}", e);
                    // NOTE: Callback NOT called to avoid thread safety issues on Android
                    return;
                }
            };
            let client = enhanced_client.client();

            // Progress update: Starting metadata fetch
            // NOTE: Callbacks are NOT called to avoid thread safety issues on Android.
            // The Dart side uses polling via ia_get_get_metadata_json() instead.

            // Create a progress bar for the metadata fetch
            let progress_bar = ProgressBar::new_spinner();

            // Fetch metadata with timeout and retry
            match fetch_json_metadata(&identifier_str, client, &progress_bar).await {
                Ok((metadata, _url)) => {
                    // NOTE: Progress callback NOT called to avoid thread safety issues on Android

                    // Store metadata in cache with error handling
                    match METADATA_CACHE.lock() {
                        Ok(mut cache) => {
                            cache.insert(identifier_str.clone(), metadata);
                            #[cfg(debug_assertions)]
                            println!("Metadata cached for '{}'", identifier_str);
                        }
                        Err(e) => {
                            eprintln!("Failed to acquire metadata cache lock: {}", e);

                            // Mark request as complete and record failure
                            if let Ok(mut tracker) = REQUEST_TRACKER.lock() {
                                if let Some(request) = tracker.get_mut(&identifier_str) {
                                    request.in_progress = false;
                                }
                            }
                            if let Ok(mut breaker) = CIRCUIT_BREAKER.lock() {
                                breaker.record_failure();
                            }
                            if let Ok(mut metrics) = PERFORMANCE_METRICS.lock() {
                                metrics.failed_requests += 1;
                            }

                            let error_msg = CString::new("Failed to cache metadata")
                                .unwrap_or_else(|_| {
                                    CString::new("Cache error")
                                        .expect("Failed to create fallback error message")
                                });
                            let _error_ptr = error_msg.as_ptr();
                            // NOTE: Callback NOT called to avoid thread safety issues on Android
                            return;
                        }
                    }

                    // Record success metrics
                    let elapsed = request_start_time.elapsed().as_millis() as u64;
                    if let Ok(mut metrics) = PERFORMANCE_METRICS.lock() {
                        metrics.successful_requests += 1;
                        // Update rolling average
                        if metrics.average_response_time_ms == 0 {
                            metrics.average_response_time_ms = elapsed;
                        } else {
                            metrics.average_response_time_ms =
                                (metrics.average_response_time_ms + elapsed) / 2;
                        }
                    }

                    // Record success in circuit breaker
                    if let Ok(mut breaker) = CIRCUIT_BREAKER.lock() {
                        breaker.record_success();
                    }

                    // Mark request as complete
                    if let Ok(mut tracker) = REQUEST_TRACKER.lock() {
                        if let Some(request) = tracker.get_mut(&identifier_str) {
                            request.in_progress = false;
                        }
                    }

                    // NOTE: Callbacks NOT called to avoid thread safety issues on Android.
                    // The Dart side polls for completion using ia_get_get_metadata_json().
                }
                Err(e) => {
                    eprintln!("Metadata fetch error for '{}': {}", identifier_str, e);

                    // Record failure metrics
                    if let Ok(mut metrics) = PERFORMANCE_METRICS.lock() {
                        metrics.failed_requests += 1;
                    }

                    // Record failure in circuit breaker
                    if let Ok(mut breaker) = CIRCUIT_BREAKER.lock() {
                        breaker.record_failure();
                    }

                    // Mark request as complete
                    if let Ok(mut tracker) = REQUEST_TRACKER.lock() {
                        if let Some(request) = tracker.get_mut(&identifier_str) {
                            request.in_progress = false;
                        }
                    }

                    let error_msg = CString::new(format!("Metadata fetch failed: {}", e))
                        .unwrap_or_else(|_| {
                            CString::new("Metadata fetch failed")
                                .expect("Failed to create fallback error message")
                        });
                    let _error_ptr = error_msg.as_ptr();
                    // NOTE: Callback NOT called to avoid thread safety issues on Android
                }
            }
        });
    });

    session_id
}

/// Get cached metadata as JSON string
/// Returns null if metadata not found
///
/// # Safety
/// The `identifier` parameter must be a valid null-terminated C string pointer.
/// The returned pointer must be freed using `ia_get_free_string`.
#[no_mangle]
pub unsafe extern "C" fn ia_get_get_metadata_json(identifier: *const c_char) -> *mut c_char {
    if identifier.is_null() {
        eprintln!("ia_get_get_metadata_json: identifier is null");
        return ptr::null_mut();
    }

    let identifier_str = match CStr::from_ptr(identifier).to_str() {
        Ok(s) => {
            if s.is_empty() {
                eprintln!("ia_get_get_metadata_json: identifier is empty");
                return ptr::null_mut();
            }
            s
        }
        Err(e) => {
            eprintln!("ia_get_get_metadata_json: invalid UTF-8: {}", e);
            return ptr::null_mut();
        }
    };

    let cache = match METADATA_CACHE.lock() {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "ia_get_get_metadata_json: failed to acquire cache lock: {}",
                e
            );
            return ptr::null_mut();
        }
    };

    if let Some(metadata) = cache.get(identifier_str) {
        match serde_json::to_string(metadata) {
            Ok(json) => match CString::new(json) {
                Ok(c_string) => {
                    #[cfg(debug_assertions)]
                    println!("Returning metadata JSON for '{}'", identifier_str);
                    c_string.into_raw()
                }
                Err(e) => {
                    eprintln!("ia_get_get_metadata_json: failed to create CString: {}", e);
                    ptr::null_mut()
                }
            },
            Err(e) => {
                eprintln!("ia_get_get_metadata_json: JSON serialization failed: {}", e);
                ptr::null_mut()
            }
        }
    } else {
        #[cfg(debug_assertions)]
        println!("No metadata cached for '{}'", identifier_str);
        ptr::null_mut()
    }
}

/// Create a new download session
/// Returns session ID for tracking
///
/// # Safety
/// Both `identifier` and `config` must be valid pointers.
/// The `identifier` must be a valid null-terminated C string.
/// The `config` must point to a valid `FfiDownloadConfig` structure.
#[no_mangle]
pub unsafe extern "C" fn ia_get_create_session(
    identifier: *const c_char,
    config: *const FfiDownloadConfig,
) -> c_int {
    if identifier.is_null() || config.is_null() {
        return -1;
    }

    let identifier_str = match CStr::from_ptr(identifier).to_str() {
        Ok(s) => s.to_string(),
        Err(_) => return -1,
    };

    let ffi_config = &*config;
    let session_id = next_session_id();

    // Convert FFI config to internal config
    let output_dir = if ffi_config.output_directory.is_null() {
        "./downloads".to_string()
    } else {
        match unsafe { CStr::from_ptr(ffi_config.output_directory) }.to_str() {
            Ok(s) => s.to_string(),
            Err(_) => "./downloads".to_string(),
        }
    };

    // Create session object (simplified for FFI demonstration)
    let session = FfiSession::new(
        identifier_str,
        output_dir,
        ffi_config.concurrent_downloads,
        ffi_config.auto_decompress,
    );

    // Store session
    match SESSIONS.lock() {
        Ok(mut sessions) => {
            sessions.insert(session_id, session);
            session_id
        }
        Err(e) => {
            eprintln!(
                "ia_get_create_session: failed to acquire sessions lock: {}",
                e
            );
            -1
        }
    }
}

/// Filter files based on criteria
/// Returns JSON string with filtered file list
///
/// # Safety
/// The `metadata_json` parameter must be a valid null-terminated C string pointer.
/// The optional parameters (`include_formats`, `exclude_formats`, `max_size_str`) can be null
/// or must be valid null-terminated C string pointers.
/// The returned pointer must be freed using `ia_get_free_string`.
#[no_mangle]
pub unsafe extern "C" fn ia_get_filter_files(
    metadata_json: *const c_char,
    include_formats: *const c_char,
    exclude_formats: *const c_char,
    max_size_str: *const c_char,
) -> *mut c_char {
    if metadata_json.is_null() {
        return ptr::null_mut();
    }

    let metadata_str = match CStr::from_ptr(metadata_json).to_str() {
        Ok(s) => s,
        Err(_) => return ptr::null_mut(),
    };

    // Parse metadata
    let metadata: ArchiveMetadata = match serde_json::from_str(metadata_str) {
        Ok(m) => m,
        Err(_) => return ptr::null_mut(),
    };

    // Parse filter criteria
    let include_formats = if include_formats.is_null() {
        Vec::new()
    } else {
        match CStr::from_ptr(include_formats).to_str() {
            Ok(s) => s.split(',').map(|s| s.trim().to_string()).collect(),
            Err(_) => Vec::new(),
        }
    };

    let exclude_formats = if exclude_formats.is_null() {
        Vec::new()
    } else {
        match CStr::from_ptr(exclude_formats).to_str() {
            Ok(s) => s.split(',').map(|s| s.trim().to_string()).collect(),
            Err(_) => Vec::new(),
        }
    };

    let max_size = if max_size_str.is_null() {
        None
    } else {
        match CStr::from_ptr(max_size_str).to_str() {
            Ok(s) => parse_size_string(s).ok(),
            Err(_) => None,
        }
    };

    // Apply filtering logic manually for FFI compatibility
    let mut filtered_files = metadata.files.clone();

    // Filter by include formats
    if !include_formats.is_empty() {
        filtered_files.retain(|file| {
            if let Some(ref format) = file.format {
                include_formats
                    .iter()
                    .any(|f| format.to_lowercase().contains(&f.to_lowercase()))
            } else {
                false
            }
        });
    }

    // Filter by exclude formats
    if !exclude_formats.is_empty() {
        filtered_files.retain(|file| {
            if let Some(ref format) = file.format {
                !exclude_formats
                    .iter()
                    .any(|f| format.to_lowercase().contains(&f.to_lowercase()))
            } else {
                true
            }
        });
    }

    // Filter by max size
    if let Some(max_size) = max_size {
        filtered_files.retain(|file| file.size.is_none_or(|size| size <= max_size));
    }

    // Serialize filtered results
    match serde_json::to_string(&filtered_files) {
        Ok(json) => match CString::new(json) {
            Ok(c_string) => c_string.into_raw(),
            Err(e) => {
                eprintln!("ia_get_filter_files: failed to create CString: {}", e);
                ptr::null_mut()
            }
        },
        Err(e) => {
            eprintln!("ia_get_filter_files: JSON serialization failed: {}", e);
            ptr::null_mut()
        }
    }
}

/// Start a download session
/// Returns 0 on success, error code on failure
/// Start downloading selected files
///
/// # Safety
/// The `files_json` parameter must be a valid null-terminated C string pointer
/// containing valid JSON data.
#[no_mangle]
pub unsafe extern "C" fn ia_get_start_download(
    session_id: c_int,
    files_json: *const c_char,
    progress_callback: ProgressCallback,
    completion_callback: CompletionCallback,
    user_data: usize,
) -> IaGetErrorCode {
    if files_json.is_null() {
        return IaGetErrorCode::InvalidInput;
    }

    let files_str = match CStr::from_ptr(files_json).to_str() {
        Ok(s) => s,
        Err(_) => return IaGetErrorCode::InvalidInput,
    };

    // Parse selected files
    let _selected_files: Vec<serde_json::Value> = match serde_json::from_str(files_str) {
        Ok(f) => f,
        Err(_) => return IaGetErrorCode::ParseError,
    };

    // Get session information to use in download
    let session_info = match SESSIONS.lock() {
        Ok(sessions) => sessions.get(&session_id).map(|session| {
            (
                session.get_identifier().to_string(),
                session.get_output_dir().to_string(),
                session.get_concurrent_downloads(),
                session.is_auto_decompress_enabled(),
            )
        }),
        Err(e) => {
            eprintln!(
                "ia_get_start_download: failed to acquire sessions lock: {}",
                e
            );
            return IaGetErrorCode::UnknownError;
        }
    };

    let Some((identifier, output_dir, concurrent_downloads, auto_decompress)) = session_info else {
        return IaGetErrorCode::InvalidInput;
    };

    let runtime = RUNTIME.clone();

    // Spawn download operation using session configuration
    std::thread::spawn(move || {
        runtime.block_on(async move {
            // Progress update: Starting download with session info
            let progress_msg = match CString::new(format!(
                "Initializing download for '{}' to '{}' (concurrency: {}, decompress: {})",
                identifier, output_dir, concurrent_downloads, auto_decompress
            )) {
                Ok(msg) => msg,
                Err(e) => {
                    eprintln!("Failed to create progress message: {}", e);
                    match CString::new("Initializing download...") {
                        Ok(fallback) => fallback,
                        Err(_) => {
                            eprintln!("Failed to create fallback progress message");
                            completion_callback(false, ptr::null(), user_data);
                            return;
                        }
                    }
                }
            };
            progress_callback(0.0, progress_msg.as_ptr(), user_data);

            // In a real implementation, this would:
            // 1. Get session from SESSIONS map
            // 2. Create DownloadService with the configuration
            // 3. Set up progress reporting callbacks with proper tracking
            // 4. Start the download with proper session management
            // 5. Handle errors and completion with detailed progress info

            // Simulate download progress
            for i in 1..=10 {
                let progress = i as f64 / 10.0;
                let progress_msg =
                    match CString::new(format!("Downloading... {}%", (progress * 100.0) as i32)) {
                        Ok(msg) => msg,
                        Err(_) => {
                            eprintln!(
                                "Failed to create progress message at {}%",
                                (progress * 100.0) as i32
                            );
                            continue; // Skip this update but continue download
                        }
                    };
                progress_callback(progress, progress_msg.as_ptr(), user_data);

                // Simulate work
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            }

            let progress_msg = match CString::new("Download completed") {
                Ok(msg) => msg,
                Err(_) => {
                    eprintln!("Failed to create completion message");
                    completion_callback(true, ptr::null(), user_data);
                    return;
                }
            };
            progress_callback(1.0, progress_msg.as_ptr(), user_data);
            completion_callback(true, ptr::null(), user_data);
        });
    });

    IaGetErrorCode::Success
}

/// Get download progress for a session
/// Get current download progress for a session
///
/// # Safety
/// The `progress` parameter must be a valid pointer to an allocated `FfiDownloadProgress` structure.
#[no_mangle]
pub unsafe extern "C" fn ia_get_get_download_progress(
    session_id: c_int,
    progress: *mut FfiDownloadProgress,
) -> IaGetErrorCode {
    if progress.is_null() {
        return IaGetErrorCode::InvalidInput;
    }

    // In a real implementation, this would:
    // 1. Look up session in SESSIONS map
    // 2. Get actual progress from the download service
    // 3. Fill in the progress structure

    // Mock progress data for demonstration
    let mock_progress = FfiDownloadProgress {
        session_id,
        overall_progress: 0.5,
        current_file: match CString::new("test.pdf") {
            Ok(cstr) => cstr.into_raw(),
            Err(e) => {
                eprintln!("Failed to create mock filename: {}", e);
                ptr::null_mut() // Use null pointer on failure
            }
        },
        current_file_progress: 0.7,
        download_speed: 1024 * 1024, // 1 MB/s
        eta_seconds: 30,
        completed_files: 2,
        total_files: 5,
        downloaded_bytes: 1024 * 1024 * 10, // 10 MB
        total_bytes: 1024 * 1024 * 20,      // 20 MB
    };

    *progress = mock_progress;

    IaGetErrorCode::Success
}

/// Pause a download session
#[no_mangle]
pub extern "C" fn ia_get_pause_download(session_id: c_int) -> IaGetErrorCode {
    // In a real implementation, this would pause the download session
    match SESSIONS.lock() {
        Ok(sessions) => {
            if sessions.contains_key(&session_id) {
                // Session exists, would pause it here
                IaGetErrorCode::Success
            } else {
                IaGetErrorCode::InvalidInput
            }
        }
        Err(e) => {
            eprintln!(
                "ia_get_pause_download: failed to acquire sessions lock: {}",
                e
            );
            IaGetErrorCode::UnknownError
        }
    }
}

/// Resume a download session
#[no_mangle]
pub extern "C" fn ia_get_resume_download(session_id: c_int) -> IaGetErrorCode {
    // In a real implementation, this would resume the download session
    match SESSIONS.lock() {
        Ok(sessions) => {
            if sessions.contains_key(&session_id) {
                // Session exists, would resume it here
                IaGetErrorCode::Success
            } else {
                IaGetErrorCode::InvalidInput
            }
        }
        Err(e) => {
            eprintln!(
                "ia_get_resume_download: failed to acquire sessions lock: {}",
                e
            );
            IaGetErrorCode::UnknownError
        }
    }
}

/// Cancel a download session
#[no_mangle]
pub extern "C" fn ia_get_cancel_download(session_id: c_int) -> IaGetErrorCode {
    // Cancel the download and remove session
    match SESSIONS.lock() {
        Ok(mut sessions) => {
            if sessions.remove(&session_id).is_some() {
                IaGetErrorCode::Success
            } else {
                IaGetErrorCode::InvalidInput
            }
        }
        Err(e) => {
            eprintln!(
                "ia_get_cancel_download: failed to acquire sessions lock: {}",
                e
            );
            IaGetErrorCode::UnknownError
        }
    }
}

/// Cancel an ongoing operation
#[no_mangle]
pub extern "C" fn ia_get_cancel_operation(operation_id: c_int) -> IaGetErrorCode {
    // This can cancel either metadata fetch or download operations
    if operation_id > 0 {
        // In a real implementation, this would cancel the operation with the given ID
        match SESSIONS.lock() {
            Ok(mut sessions) => {
                sessions.remove(&operation_id);
                IaGetErrorCode::Success
            }
            Err(e) => {
                eprintln!(
                    "ia_get_cancel_operation: failed to acquire sessions lock: {}",
                    e
                );
                IaGetErrorCode::UnknownError
            }
        }
    } else {
        IaGetErrorCode::InvalidInput
    }
}

/// Get session information as JSON
#[no_mangle]
pub extern "C" fn ia_get_get_session_info(session_id: c_int) -> *mut c_char {
    match SESSIONS.lock() {
        Ok(sessions) => {
            if let Some(session) = sessions.get(&session_id) {
                // Use the actual session data
                let session_info = serde_json::json!({
                    "session_id": session_id,
                    "identifier": session.get_identifier(),
                    "output_dir": session.get_output_dir(),
                    "concurrent_downloads": session.get_concurrent_downloads(),
                    "auto_decompress": session.is_auto_decompress_enabled(),
                    "status": "active",
                    "created_at": session.get_created_at().to_rfc3339()
                });

                match serde_json::to_string(&session_info) {
                    Ok(json) => match CString::new(json) {
                        Ok(c_string) => c_string.into_raw(),
                        Err(e) => {
                            eprintln!("ia_get_get_session_info: failed to create CString: {}", e);
                            ptr::null_mut()
                        }
                    },
                    Err(e) => {
                        eprintln!("ia_get_get_session_info: JSON serialization failed: {}", e);
                        ptr::null_mut()
                    }
                }
            } else {
                ptr::null_mut()
            }
        }
        Err(e) => {
            eprintln!(
                "ia_get_get_session_info: failed to acquire sessions lock: {}",
                e
            );
            ptr::null_mut()
        }
    }
}

/// Get available download formats for an archive
///
/// # Safety
/// The `identifier` parameter must be a valid null-terminated C string pointer.
/// The returned pointer must be freed using `ia_get_free_string`.
#[no_mangle]
pub unsafe extern "C" fn ia_get_get_available_formats(identifier: *const c_char) -> *mut c_char {
    if identifier.is_null() {
        return ptr::null_mut();
    }

    let identifier_str = match CStr::from_ptr(identifier).to_str() {
        Ok(s) => s,
        Err(_) => return ptr::null_mut(),
    };

    let cache = match METADATA_CACHE.lock() {
        Ok(cache) => cache,
        Err(e) => {
            eprintln!(
                "ia_get_get_available_formats: failed to acquire cache lock: {}",
                e
            );
            return ptr::null_mut();
        }
    };

    if let Some(metadata) = cache.get(identifier_str) {
        // Extract unique formats from files
        let mut formats: Vec<String> = metadata
            .files
            .iter()
            .filter_map(|file| file.format.as_ref())
            .cloned()
            .collect();

        formats.sort();
        formats.dedup();

        match serde_json::to_string(&formats) {
            Ok(json) => match CString::new(json) {
                Ok(c_string) => c_string.into_raw(),
                Err(e) => {
                    eprintln!(
                        "ia_get_get_available_formats: failed to create CString: {}",
                        e
                    );
                    ptr::null_mut()
                }
            },
            Err(_) => ptr::null_mut(),
        }
    } else {
        ptr::null_mut()
    }
}

/// Calculate total size of selected files
///
/// # Safety
/// The `files_json` parameter must be a valid null-terminated C string pointer
/// containing valid JSON data.
#[no_mangle]
pub unsafe extern "C" fn ia_get_calculate_total_size(files_json: *const c_char) -> u64 {
    if files_json.is_null() {
        return 0;
    }

    let files_str = match CStr::from_ptr(files_json).to_str() {
        Ok(s) => s,
        Err(_) => return 0,
    };

    let files: Vec<serde_json::Value> = match serde_json::from_str(files_str) {
        Ok(f) => f,
        Err(_) => return 0,
    };

    files
        .iter()
        .filter_map(|file| file.get("size")?.as_u64())
        .sum()
}

/// Validate download URL accessibility
///
/// # Safety
/// The `files_json` parameter must be a valid null-terminated C string pointer
/// containing valid JSON data.
#[no_mangle]
pub unsafe extern "C" fn ia_get_validate_urls(
    files_json: *const c_char,
    progress_callback: ProgressCallback,
    completion_callback: CompletionCallback,
    user_data: usize,
) -> c_int {
    if files_json.is_null() {
        return -1;
    }

    let files_str = match CStr::from_ptr(files_json).to_str() {
        Ok(s) => s.to_string(),
        Err(_) => return -1,
    };

    let runtime = RUNTIME.clone();
    let validation_id = next_session_id();

    std::thread::spawn(move || {
        runtime.block_on(async move {
            let progress_msg = CString::new("Validating download URLs...").unwrap();
            progress_callback(0.0, progress_msg.as_ptr(), user_data);

            // Parse files
            let files: Vec<serde_json::Value> = match serde_json::from_str(&files_str) {
                Ok(f) => f,
                Err(_) => {
                    let error_msg = CString::new("Failed to parse files JSON").unwrap();
                    completion_callback(false, error_msg.as_ptr(), user_data);
                    return;
                }
            };

            // In a real implementation, this would check each URL's accessibility
            // For now, simulate validation
            for (i, _file) in files.iter().enumerate() {
                let progress = (i + 1) as f64 / files.len() as f64;
                let progress_msg =
                    CString::new(format!("Validating {}/{}", i + 1, files.len())).unwrap();
                progress_callback(progress, progress_msg.as_ptr(), user_data);

                tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            }

            let progress_msg = CString::new("URL validation complete").unwrap();
            progress_callback(1.0, progress_msg.as_ptr(), user_data);
            completion_callback(true, ptr::null(), user_data);
        });
    });

    validation_id
}

/// Free memory allocated by FFI functions
///
/// # Safety
/// The `ptr` parameter must be a valid pointer previously returned by an FFI function
/// that allocates memory, or null. Calling this function with an invalid pointer
/// or calling it more than once with the same pointer will cause undefined behavior.
#[no_mangle]
pub unsafe extern "C" fn ia_get_free_string(ptr: *mut c_char) {
    if !ptr.is_null() {
        let _ = CString::from_raw(ptr);
    }
}

/// Get the last error message (thread-local)
#[no_mangle]
pub extern "C" fn ia_get_last_error() -> *const c_char {
    // In a real implementation, this would return thread-local error state
    ptr::null()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    extern "C" fn test_progress_callback(progress: f64, message: *const c_char, _user_data: usize) {
        if !message.is_null() {
            let msg = unsafe { CStr::from_ptr(message) }.to_str().unwrap();
            println!("Progress: {:.1}% - {}", progress * 100.0, msg);
        }
    }

    extern "C" fn test_completion_callback(
        success: bool,
        error_message: *const c_char,
        _user_data: usize,
    ) {
        if success {
            println!("Operation completed successfully");
        } else if !error_message.is_null() {
            let msg = unsafe { CStr::from_ptr(error_message) }.to_str().unwrap();
            println!("Operation failed: {}", msg);
        }
    }

    #[test]
    fn test_ffi_init() {
        let result = ia_get_init();
        assert_eq!(result as i32, IaGetErrorCode::Success as i32);
    }

    #[test]
    fn test_ffi_filter_files() {
        let metadata_json = r#"{
            "created": 1640995200,
            "d1": "ia800100.us.archive.org",
            "d2": "ia600100.us.archive.org", 
            "dir": "/test",
            "files": [{
                "name": "test.pdf",
                "source": "original",
                "mtime": "1640995200",
                "size": 1024,
                "format": "pdf",
                "crc32": "12345678",
                "md5": "abcdef123456789",
                "sha1": "fedcba987654321",
                "btih": "999999999999999",
                "summation": "5555"
            }],
            "files_count": 1,
            "item_last_updated": 1640995200,
            "item_size": 1024,
            "metadata": {"title": "Test Archive"},
            "server": "ia800100.us.archive.org",
            "uniq": 123456,
            "workable_servers": ["ia800100.us.archive.org"]
        }"#;
        let metadata_cstr = CString::new(metadata_json).unwrap();
        let include_formats = CString::new("pdf").unwrap();

        let result = unsafe {
            ia_get_filter_files(
                metadata_cstr.as_ptr(),
                include_formats.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
            )
        };

        assert!(!result.is_null());

        // Clean up
        unsafe {
            ia_get_free_string(result);
        }
    }

    #[test]
    fn test_ffi_fetch_metadata() {
        let identifier = CString::new("commute_test").unwrap();

        let request_id = unsafe {
            ia_get_fetch_metadata(
                identifier.as_ptr(),
                test_progress_callback,
                test_completion_callback,
                0,
            )
        };

        assert!(request_id > 0);

        // In a real test, you'd wait for the async operation to complete
    }
}

/// Check if a request is already in progress for an identifier
/// Returns true if request is in progress, false otherwise
///
/// # Safety
/// The `identifier` parameter must be a valid null-terminated C string pointer.
#[no_mangle]
pub unsafe extern "C" fn ia_get_is_request_in_progress(identifier: *const c_char) -> bool {
    if identifier.is_null() {
        return false;
    }

    let identifier_str = match CStr::from_ptr(identifier).to_str() {
        Ok(s) => s,
        Err(_) => return false,
    };

    match REQUEST_TRACKER.lock() {
        Ok(tracker) => {
            if let Some(request) = tracker.get(identifier_str) {
                // Consider in progress if less than 60 seconds old
                request.in_progress && request.last_request_time.elapsed() < Duration::from_secs(60)
            } else {
                false
            }
        }
        Err(_) => false,
    }
}

/// Get performance metrics as JSON string
/// Returns JSON with metrics or null on error
///
/// # Safety
/// The returned pointer must be freed using `ia_get_free_string`.
#[no_mangle]
pub extern "C" fn ia_get_get_performance_metrics() -> *mut c_char {
    match PERFORMANCE_METRICS.lock() {
        Ok(metrics) => {
            let metrics_json = serde_json::json!({
                "total_requests": metrics.total_requests,
                "successful_requests": metrics.successful_requests,
                "failed_requests": metrics.failed_requests,
                "success_rate": if metrics.total_requests > 0 {
                    (metrics.successful_requests as f64 / metrics.total_requests as f64) * 100.0
                } else {
                    0.0
                },
                "average_response_time_ms": metrics.average_response_time_ms,
                "cache_hits": metrics.cache_hits,
                "cache_misses": metrics.cache_misses,
                "cache_hit_rate": if (metrics.cache_hits + metrics.cache_misses) > 0 {
                    (metrics.cache_hits as f64 / (metrics.cache_hits + metrics.cache_misses) as f64) * 100.0
                } else {
                    0.0
                }
            });

            match serde_json::to_string(&metrics_json) {
                Ok(json) => match CString::new(json) {
                    Ok(c_string) => c_string.into_raw(),
                    Err(e) => {
                        eprintln!(
                            "ia_get_get_performance_metrics: failed to create CString: {}",
                            e
                        );
                        ptr::null_mut()
                    }
                },
                Err(e) => {
                    eprintln!(
                        "ia_get_get_performance_metrics: JSON serialization failed: {}",
                        e
                    );
                    ptr::null_mut()
                }
            }
        }
        Err(e) => {
            eprintln!(
                "ia_get_get_performance_metrics: failed to acquire metrics lock: {}",
                e
            );
            ptr::null_mut()
        }
    }
}

/// Reset performance metrics
#[no_mangle]
pub extern "C" fn ia_get_reset_performance_metrics() -> IaGetErrorCode {
    match PERFORMANCE_METRICS.lock() {
        Ok(mut metrics) => {
            *metrics = PerformanceMetrics {
                total_requests: 0,
                successful_requests: 0,
                failed_requests: 0,
                average_response_time_ms: 0,
                cache_hits: 0,
                cache_misses: 0,
            };
            IaGetErrorCode::Success
        }
        Err(e) => {
            eprintln!(
                "ia_get_reset_performance_metrics: failed to acquire metrics lock: {}",
                e
            );
            IaGetErrorCode::UnknownError
        }
    }
}

/// Check health of the FFI system
/// Returns 0 for healthy, non-zero for issues
#[no_mangle]
pub extern "C" fn ia_get_health_check() -> c_int {
    let mut health_score = 0;

    // Check circuit breaker state
    if let Ok(breaker) = CIRCUIT_BREAKER.lock() {
        if breaker.state == CircuitBreakerState::Open {
            health_score += 10; // Circuit breaker is open - major issue
        } else if breaker.state == CircuitBreakerState::HalfOpen {
            health_score += 5; // Circuit breaker is recovering
        }
    } else {
        health_score += 20; // Can't acquire circuit breaker lock - critical
    }

    // Check if we can acquire all necessary locks
    if SESSIONS.lock().is_err() {
        health_score += 15;
    }
    if METADATA_CACHE.lock().is_err() {
        health_score += 15;
    }
    if REQUEST_TRACKER.lock().is_err() {
        health_score += 10;
    }

    // Check metrics for high failure rate
    if let Ok(metrics) = PERFORMANCE_METRICS.lock() {
        if metrics.total_requests > 10 {
            let failure_rate =
                (metrics.failed_requests as f64 / metrics.total_requests as f64) * 100.0;
            if failure_rate > 50.0 {
                health_score += 20; // High failure rate
            } else if failure_rate > 25.0 {
                health_score += 10; // Moderate failure rate
            }
        }
    }

    health_score
}

/// Clear stale entries from request tracker and metadata cache
/// Helps prevent memory leaks and keeps caches fresh
#[no_mangle]
pub extern "C" fn ia_get_clear_stale_cache() -> IaGetErrorCode {
    // Clear stale request tracker entries (older than 5 minutes)
    if let Ok(mut tracker) = REQUEST_TRACKER.lock() {
        tracker.retain(|_, request| {
            request.in_progress && request.last_request_time.elapsed() < Duration::from_secs(300)
        });
    }

    // Clear old metadata cache entries if cache is too large
    if let Ok(mut cache) = METADATA_CACHE.lock() {
        if cache.len() > 100 {
            // Keep only the 50 most recently used entries
            // In a production system, you'd track access times
            let keys_to_remove: Vec<String> = cache.keys().skip(50).cloned().collect();

            for key in keys_to_remove {
                cache.remove(&key);
            }
        }
    }

    IaGetErrorCode::Success
}

/// Get circuit breaker status
/// Returns: 0 = Closed (healthy), 1 = HalfOpen (recovering), 2 = Open (failing)
#[no_mangle]
pub extern "C" fn ia_get_get_circuit_breaker_status() -> c_int {
    match CIRCUIT_BREAKER.lock() {
        Ok(breaker) => match breaker.state {
            CircuitBreakerState::Closed => 0,
            CircuitBreakerState::HalfOpen => 1,
            CircuitBreakerState::Open => 2,
        },
        Err(_) => -1, // Error acquiring lock
    }
}

/// Manually reset circuit breaker (use with caution)
#[no_mangle]
pub extern "C" fn ia_get_reset_circuit_breaker() -> IaGetErrorCode {
    match CIRCUIT_BREAKER.lock() {
        Ok(mut breaker) => {
            breaker.record_success();
            IaGetErrorCode::Success
        }
        Err(e) => {
            eprintln!(
                "ia_get_reset_circuit_breaker: failed to acquire breaker lock: {}",
                e
            );
            IaGetErrorCode::UnknownError
        }
    }
}
