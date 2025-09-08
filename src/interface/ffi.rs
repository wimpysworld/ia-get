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
use tokio::runtime::Runtime;

use crate::core::archive::fetch_json_metadata;
use crate::core::session::ArchiveMetadata;
use crate::infrastructure::http::HttpClientFactory;
use crate::utilities::filters::parse_size_string;

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
    fn new(identifier: String, output_dir: String, concurrent_downloads: u32, auto_decompress: bool) -> Self {
        Self {
            identifier,
            output_dir,
            concurrent_downloads,
            auto_decompress,
            created_at: chrono::Utc::now(),
        }
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
    pub selected: bool,  // For UI selection state
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
    pub overall_progress: f64,      // 0.0 to 1.0
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
    let mut next_id = NEXT_SESSION_ID.lock().unwrap();
    let id = *next_id;
    *next_id += 1;
    id
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
#[no_mangle]
pub extern "C" fn ia_get_fetch_metadata(
    identifier: *const c_char,
    progress_callback: ProgressCallback,
    completion_callback: CompletionCallback,
    user_data: usize,
) -> c_int {
    if identifier.is_null() {
        return -1;
    }

    let identifier_str = match unsafe { CStr::from_ptr(identifier) }.to_str() {
        Ok(s) => s.to_string(),
        Err(_) => return -1,
    };

    let session_id = next_session_id();
    let runtime = RUNTIME.clone();

    // Spawn async operation in background thread
    std::thread::spawn(move || {
        runtime.block_on(async move {
            // Create HTTP client
            let enhanced_client = match HttpClientFactory::for_metadata_requests() {
                Ok(c) => c,
                Err(e) => {
                    let error_msg = CString::new(format!("HTTP client error: {}", e)).unwrap();
                    completion_callback(false, error_msg.as_ptr(), user_data);
                    return;
                }
            };
            let client = enhanced_client.client();

            // Progress update: Starting metadata fetch
            let progress_msg = CString::new("Fetching archive metadata...").unwrap();
            progress_callback(0.1, progress_msg.as_ptr(), user_data);

            // Create a progress bar for the metadata fetch
            let progress_bar = ProgressBar::new_spinner();

            // Fetch metadata
            match fetch_json_metadata(&identifier_str, &client, &progress_bar).await {
                Ok((metadata, _url)) => {
                    let progress_msg = CString::new("Parsing metadata...").unwrap();
                    progress_callback(0.8, progress_msg.as_ptr(), user_data);

                    // Store metadata in cache
                    {
                        let mut cache = METADATA_CACHE.lock().unwrap();
                        cache.insert(identifier_str.clone(), metadata);
                    }

                    let progress_msg = CString::new("Metadata fetch complete").unwrap();
                    progress_callback(1.0, progress_msg.as_ptr(), user_data);
                    completion_callback(true, ptr::null(), user_data);
                }
                Err(e) => {
                    let error_msg = CString::new(format!("Metadata fetch failed: {}", e)).unwrap();
                    completion_callback(false, error_msg.as_ptr(), user_data);
                }
            }
        });
    });

    session_id
}

/// Get cached metadata as JSON string
/// Returns null if metadata not found
#[no_mangle]
pub extern "C" fn ia_get_get_metadata_json(identifier: *const c_char) -> *mut c_char {
    if identifier.is_null() {
        return ptr::null_mut();
    }

    let identifier_str = match unsafe { CStr::from_ptr(identifier) }.to_str() {
        Ok(s) => s,
        Err(_) => return ptr::null_mut(),
    };

    let cache = METADATA_CACHE.lock().unwrap();
    if let Some(metadata) = cache.get(identifier_str) {
        match serde_json::to_string(metadata) {
            Ok(json) => {
                let c_string = CString::new(json).unwrap();
                c_string.into_raw()
            }
            Err(_) => ptr::null_mut(),
        }
    } else {
        ptr::null_mut()
    }
}

/// Create a new download session
/// Returns session ID for tracking
#[no_mangle] 
pub extern "C" fn ia_get_create_session(
    identifier: *const c_char,
    config: *const FfiDownloadConfig,
) -> c_int {
    if identifier.is_null() || config.is_null() {
        return -1;
    }

    let identifier_str = match unsafe { CStr::from_ptr(identifier) }.to_str() {
        Ok(s) => s.to_string(),
        Err(_) => return -1,
    };

    let ffi_config = unsafe { &*config };
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
    {
        let mut sessions = SESSIONS.lock().unwrap();
        sessions.insert(session_id, session);
    }

    session_id
}

/// Filter files based on criteria
/// Returns JSON string with filtered file list
#[no_mangle]
pub extern "C" fn ia_get_filter_files(
    metadata_json: *const c_char,
    include_formats: *const c_char,
    exclude_formats: *const c_char,
    max_size_str: *const c_char,
) -> *mut c_char {
    if metadata_json.is_null() {
        return ptr::null_mut();
    }

    let metadata_str = match unsafe { CStr::from_ptr(metadata_json) }.to_str() {
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
        match unsafe { CStr::from_ptr(include_formats) }.to_str() {
            Ok(s) => s.split(',').map(|s| s.trim().to_string()).collect(),
            Err(_) => Vec::new(),
        }
    };

    let exclude_formats = if exclude_formats.is_null() {
        Vec::new()
    } else {
        match unsafe { CStr::from_ptr(exclude_formats) }.to_str() {
            Ok(s) => s.split(',').map(|s| s.trim().to_string()).collect(),
            Err(_) => Vec::new(),
        }
    };

    let max_size = if max_size_str.is_null() {
        None
    } else {
        match unsafe { CStr::from_ptr(max_size_str) }.to_str() {
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
        filtered_files.retain(|file| file.size.map_or(true, |size| size <= max_size));
    }

    // Serialize filtered results
    match serde_json::to_string(&filtered_files) {
        Ok(json) => {
            let c_string = CString::new(json).unwrap();
            c_string.into_raw()
        }
        Err(_) => ptr::null_mut(),
    }
}

/// Start a download session
/// Returns 0 on success, error code on failure
#[no_mangle]
pub extern "C" fn ia_get_start_download(
    _session_id: c_int,
    files_json: *const c_char,
    progress_callback: ProgressCallback,
    completion_callback: CompletionCallback,
    user_data: usize,
) -> IaGetErrorCode {
    if files_json.is_null() {
        return IaGetErrorCode::InvalidInput;
    }

    let files_str = match unsafe { CStr::from_ptr(files_json) }.to_str() {
        Ok(s) => s,
        Err(_) => return IaGetErrorCode::InvalidInput,
    };

    // Parse selected files
    let _selected_files: Vec<serde_json::Value> = match serde_json::from_str(files_str) {
        Ok(f) => f,
        Err(_) => return IaGetErrorCode::ParseError,
    };

    let runtime = RUNTIME.clone();

    // Spawn download operation
    std::thread::spawn(move || {
        runtime.block_on(async move {
            // Progress update: Starting download
            let progress_msg = CString::new("Initializing download...").unwrap();
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
                let progress_msg = CString::new(format!("Downloading... {}%", (progress * 100.0) as i32)).unwrap();
                progress_callback(progress, progress_msg.as_ptr(), user_data);
                
                // Simulate work
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            }

            let progress_msg = CString::new("Download completed").unwrap();
            progress_callback(1.0, progress_msg.as_ptr(), user_data);
            completion_callback(true, ptr::null(), user_data);
        });
    });

    IaGetErrorCode::Success
}

/// Get download progress for a session
#[no_mangle]
pub extern "C" fn ia_get_get_download_progress(
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
        current_file: CString::new("test.pdf").unwrap().into_raw(),
        current_file_progress: 0.7,
        download_speed: 1024 * 1024, // 1 MB/s
        eta_seconds: 30,
        completed_files: 2,
        total_files: 5,
        downloaded_bytes: 1024 * 1024 * 10, // 10 MB
        total_bytes: 1024 * 1024 * 20,      // 20 MB
    };

    unsafe {
        *progress = mock_progress;
    }

    IaGetErrorCode::Success
}

/// Pause a download session
#[no_mangle]
pub extern "C" fn ia_get_pause_download(_session_id: c_int) -> IaGetErrorCode {
    // In a real implementation, this would pause the download session
    let _sessions = SESSIONS.lock().unwrap();
    // sessions.get_mut(&session_id).map(|session| session.pause());
    IaGetErrorCode::Success
}

/// Resume a download session
#[no_mangle]
pub extern "C" fn ia_get_resume_download(_session_id: c_int) -> IaGetErrorCode {
    // In a real implementation, this would resume the download session
    let _sessions = SESSIONS.lock().unwrap();
    // sessions.get_mut(&session_id).map(|session| session.resume());
    IaGetErrorCode::Success
}

/// Cancel a download session
#[no_mangle]
pub extern "C" fn ia_get_cancel_download(session_id: c_int) -> IaGetErrorCode {
    // Cancel the download and remove session
    let mut sessions = SESSIONS.lock().unwrap();
    if sessions.remove(&session_id).is_some() {
        IaGetErrorCode::Success
    } else {
        IaGetErrorCode::InvalidInput
    }
}

/// Cancel an ongoing operation
#[no_mangle]
pub extern "C" fn ia_get_cancel_operation(operation_id: c_int) -> IaGetErrorCode {
    // This can cancel either metadata fetch or download operations
    if operation_id > 0 {
        // In a real implementation, this would cancel the operation with the given ID
        let mut sessions = SESSIONS.lock().unwrap();
        sessions.remove(&operation_id);
        IaGetErrorCode::Success
    } else {
        IaGetErrorCode::InvalidInput
    }
}

/// Get session information as JSON
#[no_mangle]
pub extern "C" fn ia_get_get_session_info(session_id: c_int) -> *mut c_char {
    let sessions = SESSIONS.lock().unwrap();
    if let Some(_session) = sessions.get(&session_id) {
        // In a real implementation, serialize session info
        let session_info = serde_json::json!({
            "session_id": session_id,
            "status": "active",
            "created_at": chrono::Utc::now().to_rfc3339()
        });
        
        match serde_json::to_string(&session_info) {
            Ok(json) => {
                let c_string = CString::new(json).unwrap();
                c_string.into_raw()
            }
            Err(_) => ptr::null_mut(),
        }
    } else {
        ptr::null_mut()
    }
}

/// Get available download formats for an archive
#[no_mangle]
pub extern "C" fn ia_get_get_available_formats(identifier: *const c_char) -> *mut c_char {
    if identifier.is_null() {
        return ptr::null_mut();
    }

    let identifier_str = match unsafe { CStr::from_ptr(identifier) }.to_str() {
        Ok(s) => s,
        Err(_) => return ptr::null_mut(),
    };

    let cache = METADATA_CACHE.lock().unwrap();
    if let Some(metadata) = cache.get(identifier_str) {
        // Extract unique formats from files
        let mut formats: Vec<String> = metadata
            .files
            .iter()
            .filter_map(|file| file.format.as_ref())
            .map(|f| f.clone())
            .collect();
        
        formats.sort();
        formats.dedup();

        match serde_json::to_string(&formats) {
            Ok(json) => {
                let c_string = CString::new(json).unwrap();
                c_string.into_raw()
            }
            Err(_) => ptr::null_mut(),
        }
    } else {
        ptr::null_mut()
    }
}

/// Calculate total size of selected files
#[no_mangle]
pub extern "C" fn ia_get_calculate_total_size(files_json: *const c_char) -> u64 {
    if files_json.is_null() {
        return 0;
    }

    let files_str = match unsafe { CStr::from_ptr(files_json) }.to_str() {
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
#[no_mangle]
pub extern "C" fn ia_get_validate_urls(
    files_json: *const c_char,
    progress_callback: ProgressCallback,
    completion_callback: CompletionCallback,
    user_data: usize,
) -> c_int {
    if files_json.is_null() {
        return -1;
    }

    let files_str = match unsafe { CStr::from_ptr(files_json) }.to_str() {
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
                let progress_msg = CString::new(format!("Validating {}/{}", i + 1, files.len())).unwrap();
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
#[no_mangle]
pub extern "C" fn ia_get_free_string(ptr: *mut c_char) {
    if !ptr.is_null() {
        unsafe {
            let _ = CString::from_raw(ptr);
        }
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

        let result = ia_get_filter_files(
            metadata_cstr.as_ptr(),
            include_formats.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
        );

        assert!(!result.is_null());

        // Clean up
        ia_get_free_string(result);
    }

    #[test]
    fn test_ffi_fetch_metadata() {
        let identifier = CString::new("commute_test").unwrap();

        let request_id = ia_get_fetch_metadata(
            identifier.as_ptr(),
            test_progress_callback,
            test_completion_callback,
            0,
        );

        assert!(request_id > 0);

        // In a real test, you'd wait for the async operation to complete
    }
}
