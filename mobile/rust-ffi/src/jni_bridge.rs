//! JNI Bridge for Android Integration
//!
//! This module provides JNI bindings that bridge the Kotlin code to the Rust FFI implementation.
//! It allows the Android app to call Rust functions through JNI.

use jni::{
    objects::{JClass, JObject, JString, JValue},
    sys::{jint, jlong, jobject},
    JNIEnv,
};
use std::ffi::{CStr, CString};
use std::os::raw::c_char;

// Re-export the core FFI functions
use ia_get::interface::ffi::*;

/// Convert Java string to Rust String
fn jstring_to_string(
    env: &mut JNIEnv,
    jstr: &JString,
) -> Result<String, Box<dyn std::error::Error>> {
    let java_str = env.get_string(jstr)?;
    Ok(java_str.into())
}

/// Convert Rust String to Java string
fn string_to_jstring<'local>(
    env: &mut JNIEnv<'local>,
    string: &str,
) -> Result<JString<'local>, Box<dyn std::error::Error>> {
    Ok(env.new_string(string)?)
}

/// Initialize the ia-get library
#[no_mangle]
pub extern "system" fn Java_com_gameaday_ia_1get_1mobile_IaGetNativeWrapper_iaGetInit(
    _env: JNIEnv,
    _class: JClass,
) -> jint {
    println!("JNI: Initializing ia-get library");
    let result = ia_get_init() as jint;
    if result == 0 {
        println!("JNI: Library initialized successfully");
    } else {
        eprintln!("JNI: Library initialization failed with code: {}", result);
    }
    result
}

/// Cleanup the ia-get library
#[no_mangle]
pub extern "system" fn Java_com_gameaday_ia_1get_1mobile_IaGetNativeWrapper_iaGetCleanup(
    _env: JNIEnv,
    _class: JClass,
) {
    ia_get_cleanup();
}

/// Fetch metadata for an archive
#[no_mangle]
pub extern "system" fn Java_com_gameaday_ia_1get_1mobile_IaGetNativeWrapper_iaGetFetchMetadata(
    mut env: JNIEnv,
    _class: JClass,
    identifier: JString,
    _progress_callback: JObject,
    _completion_callback: JObject,
) -> jint {
    // Validate input
    if identifier.is_null() {
        eprintln!("JNI: identifier is null");
        return -1;
    }

    let identifier_str = match jstring_to_string(&mut env, &identifier) {
        Ok(s) => {
            if s.trim().is_empty() {
                eprintln!("JNI: identifier is empty");
                return -1;
            }
            s
        }
        Err(e) => {
            eprintln!("JNI: Failed to convert identifier: {:?}", e);
            return -1;
        }
    };

    let identifier_cstr = match CString::new(identifier_str.clone()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("JNI: Failed to create CString: {:?}", e);
            return -1;
        }
    };

    println!("JNI: Starting metadata fetch for '{}'", identifier_str);

    // Create callback wrappers that call back to Kotlin
    extern "C" fn progress_cb(progress: f64, message: *const c_char, _user_data: usize) {
        // TODO: Implement JNI callback to Kotlin
        let msg = unsafe {
            if message.is_null() {
                String::new()
            } else {
                match CStr::from_ptr(message).to_str() {
                    Ok(s) => s.to_string(),
                    Err(e) => {
                        eprintln!("JNI progress_cb: Invalid UTF-8 in message: {:?}", e);
                        String::new()
                    }
                }
            }
        };
        println!("Progress: {:.1}% - {}", progress * 100.0, msg);
    }

    extern "C" fn completion_cb(success: bool, error_message: *const c_char, _user_data: usize) {
        // TODO: Implement JNI callback to Kotlin
        if success {
            println!("Metadata fetch completed successfully");
        } else {
            let error = unsafe {
                if error_message.is_null() {
                    "Unknown error".to_string()
                } else {
                    match CStr::from_ptr(error_message).to_str() {
                        Ok(s) => s.to_string(),
                        Err(e) => {
                            eprintln!("JNI completion_cb: Invalid UTF-8 in error message: {:?}", e);
                            "Unknown error".to_string()
                        }
                    }
                }
            };
            eprintln!("Metadata fetch failed: {}", error);
        }
    }

    let result =
        unsafe { ia_get_fetch_metadata(identifier_cstr.as_ptr(), progress_cb, completion_cb, 0) };

    if result > 0 {
        println!("JNI: Metadata fetch initiated with session ID: {}", result);
    } else {
        eprintln!("JNI: Failed to initiate metadata fetch");
    }

    result
}

/// Get cached metadata as JSON
#[no_mangle]
pub extern "system" fn Java_com_gameaday_ia_1get_1mobile_IaGetNativeWrapper_iaGetGetMetadataJson(
    mut env: JNIEnv,
    _class: JClass,
    identifier: JString,
) -> jobject {
    // Validate input
    if identifier.is_null() {
        eprintln!("JNI: getMetadataJson - identifier is null");
        return JObject::null().as_raw();
    }

    let identifier_str = match jstring_to_string(&mut env, &identifier) {
        Ok(s) => {
            if s.trim().is_empty() {
                eprintln!("JNI: getMetadataJson - identifier is empty");
                return JObject::null().as_raw();
            }
            s
        }
        Err(e) => {
            eprintln!(
                "JNI: getMetadataJson - Failed to convert identifier: {:?}",
                e
            );
            return JObject::null().as_raw();
        }
    };

    let identifier_cstr = match CString::new(identifier_str.clone()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("JNI: getMetadataJson - Failed to create CString: {:?}", e);
            return JObject::null().as_raw();
        }
    };

    let result_ptr = unsafe { ia_get_get_metadata_json(identifier_cstr.as_ptr()) };

    if result_ptr.is_null() {
        println!("JNI: No metadata cached for '{}'", identifier_str);
        return JObject::null().as_raw();
    }

    let result_str = unsafe {
        let cstr = CStr::from_ptr(result_ptr);
        match cstr.to_str() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("JNI: getMetadataJson - Invalid UTF-8 in result: {:?}", e);
                ia_get_free_string(result_ptr as *mut c_char);
                return JObject::null().as_raw();
            }
        }
    };

    match string_to_jstring(&mut env, result_str) {
        Ok(jstr) => {
            // Free the C string
            unsafe { ia_get_free_string(result_ptr as *mut c_char) };
            println!(
                "JNI: Retrieved metadata for '{}' ({} bytes)",
                identifier_str,
                result_str.len()
            );
            jstr.as_raw()
        }
        Err(e) => {
            eprintln!("JNI: getMetadataJson - Failed to create JString: {:?}", e);
            unsafe { ia_get_free_string(result_ptr as *mut c_char) };
            JObject::null().as_raw()
        }
    }
}

/// Filter files based on criteria
#[no_mangle]
pub extern "system" fn Java_com_gameaday_ia_1get_1mobile_IaGetNativeWrapper_iaGetFilterFiles(
    mut env: JNIEnv,
    _class: JClass,
    metadata_json: JString,
    include_formats: JString,
    exclude_formats: JString,
    max_size_str: JString,
) -> jobject {
    let metadata_str = match jstring_to_string(&mut env, &metadata_json) {
        Ok(s) => s,
        Err(_) => return JObject::null().as_raw(),
    };

    let metadata_cstr = match CString::new(metadata_str) {
        Ok(s) => s,
        Err(_) => return JObject::null().as_raw(),
    };

    // Handle optional parameters
    let include_cstr = if include_formats.is_null() {
        std::ptr::null()
    } else {
        match jstring_to_string(&mut env, &include_formats)
            .and_then(|s| CString::new(s).map_err(|e| e.into()))
        {
            Ok(s) => s.as_ptr(),
            Err(_) => std::ptr::null(),
        }
    };

    let exclude_cstr = if exclude_formats.is_null() {
        std::ptr::null()
    } else {
        match jstring_to_string(&mut env, &exclude_formats)
            .and_then(|s| CString::new(s).map_err(|e| e.into()))
        {
            Ok(s) => s.as_ptr(),
            Err(_) => std::ptr::null(),
        }
    };

    let max_size_cstr = if max_size_str.is_null() {
        std::ptr::null()
    } else {
        match jstring_to_string(&mut env, &max_size_str)
            .and_then(|s| CString::new(s).map_err(|e| e.into()))
        {
            Ok(s) => s.as_ptr(),
            Err(_) => std::ptr::null(),
        }
    };

    let result_ptr = unsafe {
        ia_get_filter_files(
            metadata_cstr.as_ptr(),
            include_cstr,
            exclude_cstr,
            max_size_cstr,
        )
    };

    if result_ptr.is_null() {
        return JObject::null().as_raw();
    }

    let result_str = unsafe {
        let cstr = CStr::from_ptr(result_ptr);
        match cstr.to_str() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("JNI filterFiles: Invalid UTF-8 in result: {:?}", e);
                ia_get_free_string(result_ptr as *mut c_char);
                return JObject::null().as_raw();
            }
        }
    };

    match string_to_jstring(&mut env, result_str) {
        Ok(jstr) => {
            unsafe { ia_get_free_string(result_ptr as *mut c_char) };
            jstr.as_raw()
        }
        Err(_) => {
            unsafe { ia_get_free_string(result_ptr as *mut c_char) };
            JObject::null().as_raw()
        }
    }
}

/// Create a download session
#[no_mangle]
pub extern "system" fn Java_com_gameaday_ia_1get_1mobile_IaGetNativeWrapper_iaGetCreateSession(
    mut env: JNIEnv,
    _class: JClass,
    identifier: JString,
    config_json: JString,
) -> jint {
    let identifier_str = match jstring_to_string(&mut env, &identifier) {
        Ok(s) => s,
        Err(_) => return -1,
    };

    let _config_str = match jstring_to_string(&mut env, &config_json) {
        Ok(s) => s,
        Err(_) => return -1,
    };

    let identifier_cstr = match CString::new(identifier_str) {
        Ok(s) => s,
        Err(_) => return -1,
    };

    // Parse config JSON and create FfiDownloadConfig
    // For simplicity, using default config here
    let config = FfiDownloadConfig {
        concurrent_downloads: 4,
        max_file_size: 0,
        output_directory: std::ptr::null(),
        include_formats: std::ptr::null(),
        exclude_formats: std::ptr::null(),
        dry_run: false,
        verbose: false,
        auto_decompress: false,
        resume_downloads: true,
        verify_checksums: true,
    };

    unsafe { ia_get_create_session(identifier_cstr.as_ptr(), &config) }
}

/// Start a download
#[no_mangle]
pub extern "system" fn Java_com_gameaday_ia_1get_1mobile_IaGetNativeWrapper_iaGetStartDownload(
    mut env: JNIEnv,
    _class: JClass,
    session_id: jint,
    files_json: JString,
    _progress_callback: JObject,
    _completion_callback: JObject,
) -> jint {
    let files_str = match jstring_to_string(&mut env, &files_json) {
        Ok(s) => s,
        Err(_) => return IaGetErrorCode::InvalidInput as jint,
    };

    let files_cstr = match CString::new(files_str) {
        Ok(s) => s,
        Err(_) => return IaGetErrorCode::InvalidInput as jint,
    };

    extern "C" fn progress_cb(progress: f64, _message: *const c_char, _user_data: usize) {
        // TODO: Implement JNI callback to Kotlin
        println!("Download Progress: {:.1}%", progress * 100.0);
    }

    extern "C" fn completion_cb(success: bool, _error_message: *const c_char, _user_data: usize) {
        // TODO: Implement JNI callback to Kotlin
        if success {
            println!("Download completed successfully");
        } else {
            println!("Download failed");
        }
    }

    unsafe {
        ia_get_start_download(
            session_id,
            files_cstr.as_ptr(),
            progress_cb,
            completion_cb,
            0,
        ) as jint
    }
}

/// Pause a download
#[no_mangle]
pub extern "system" fn Java_com_gameaday_ia_1get_1mobile_IaGetNativeWrapper_iaGetPauseDownload(
    _env: JNIEnv,
    _class: JClass,
    session_id: jint,
) -> jint {
    ia_get_pause_download(session_id) as jint
}

/// Resume a download
#[no_mangle]
pub extern "system" fn Java_com_gameaday_ia_1get_1mobile_IaGetNativeWrapper_iaGetResumeDownload(
    _env: JNIEnv,
    _class: JClass,
    session_id: jint,
) -> jint {
    ia_get_resume_download(session_id) as jint
}

/// Cancel a download
#[no_mangle]
pub extern "system" fn Java_com_gameaday_ia_1get_1mobile_IaGetNativeWrapper_iaGetCancelDownload(
    _env: JNIEnv,
    _class: JClass,
    session_id: jint,
) -> jint {
    ia_get_cancel_download(session_id) as jint
}

/// Get download progress
#[no_mangle]
pub extern "system" fn Java_com_gameaday_ia_1get_1mobile_IaGetNativeWrapper_iaGetGetDownloadProgress(
    mut env: JNIEnv,
    _class: JClass,
    session_id: jint,
) -> jobject {
    let mut progress = FfiDownloadProgress {
        session_id: 0,
        overall_progress: 0.0,
        current_file: std::ptr::null(),
        current_file_progress: 0.0,
        download_speed: 0,
        eta_seconds: 0,
        completed_files: 0,
        total_files: 0,
        downloaded_bytes: 0,
        total_bytes: 0,
    };

    let result = unsafe { ia_get_get_download_progress(session_id, &mut progress) };

    if result as i32 != IaGetErrorCode::Success as i32 {
        return JObject::null().as_raw();
    }

    // Create DownloadProgressInfo object
    let class = match env
        .find_class("com/gameaday/ia_get_mobile/IaGetNativeWrapper$DownloadProgressInfo")
    {
        Ok(cls) => cls,
        Err(_) => return JObject::null().as_raw(),
    };

    let current_file_str = unsafe {
        if progress.current_file.is_null() {
            "".to_string()
        } else {
            match CStr::from_ptr(progress.current_file).to_str() {
                Ok(s) => s.to_string(),
                Err(e) => {
                    eprintln!("JNI getDownloadProgress: Invalid UTF-8 in current_file: {:?}", e);
                    "".to_string()
                }
            }
        }
    };

    let current_file_jstr = match string_to_jstring(&mut env, &current_file_str) {
        Ok(s) => s,
        Err(_) => return JObject::null().as_raw(),
    };

    let constructor_sig = "(IDLjava/lang/String;DJJIIJJ)V";

    // Create a binding to avoid temporary value issue
    let current_file_jobject = current_file_jstr.into();
    let args = [
        JValue::Int(progress.session_id),
        JValue::Double(progress.overall_progress),
        JValue::Object(&current_file_jobject),
        JValue::Double(progress.current_file_progress),
        JValue::Long(progress.download_speed as jlong),
        JValue::Long(progress.eta_seconds as jlong),
        JValue::Int(progress.completed_files as jint),
        JValue::Int(progress.total_files as jint),
        JValue::Long(progress.downloaded_bytes as jlong),
        JValue::Long(progress.total_bytes as jlong),
    ];

    match env.new_object(class, constructor_sig, &args) {
        Ok(obj) => obj.as_raw(),
        Err(_) => JObject::null().as_raw(),
    }
}

/// Calculate total size of files
#[no_mangle]
pub extern "system" fn Java_com_gameaday_ia_1get_1mobile_IaGetNativeWrapper_iaGetCalculateTotalSize(
    mut env: JNIEnv,
    _class: JClass,
    files_json: JString,
) -> jlong {
    let files_str = match jstring_to_string(&mut env, &files_json) {
        Ok(s) => s,
        Err(_) => return 0,
    };

    let files_cstr = match CString::new(files_str) {
        Ok(s) => s,
        Err(_) => return 0,
    };

    unsafe { ia_get_calculate_total_size(files_cstr.as_ptr()) as jlong }
}

/// Free a native string
#[no_mangle]
pub extern "system" fn Java_com_gameaday_ia_1get_1mobile_IaGetNativeWrapper_iaGetFreeString(
    _env: JNIEnv,
    _class: JClass,
    ptr: jlong,
) {
    if ptr != 0 {
        unsafe { ia_get_free_string(ptr as *mut c_char) };
    }
}
