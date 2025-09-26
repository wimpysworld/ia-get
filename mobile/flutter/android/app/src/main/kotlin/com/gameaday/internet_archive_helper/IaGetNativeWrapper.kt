package com.gameaday.internet_archive_helper

/**
 * JNI wrapper for the Rust FFI library
 * This class provides the interface between Kotlin and the native Rust implementation
 */
class IaGetNativeWrapper {
    
    companion object {
        init {
            System.loadLibrary("ia_get_mobile")
        }
    }
    
    // Core FFI functions
    external fun iaGetInit(): Int
    external fun iaGetCleanup()
    
    // Metadata operations
    external fun iaGetFetchMetadata(
        identifier: String,
        progressCallback: (Double, String) -> Unit,
        completionCallback: (Boolean, String?) -> Unit
    ): Int
    
    external fun iaGetGetMetadataJson(identifier: String): String?
    external fun iaGetFilterFiles(
        metadataJson: String,
        includeFormats: String?,
        excludeFormats: String?,
        maxSizeStr: String?
    ): String?
    
    external fun iaGetGetAvailableFormats(identifier: String): String?
    external fun iaGetCalculateTotalSize(filesJson: String): Long
    external fun iaGetValidateUrls(
        filesJson: String,
        progressCallback: (Double, String) -> Unit,
        completionCallback: (Boolean, String?) -> Unit
    ): Int
    
    // Session management
    external fun iaGetCreateSession(identifier: String, configJson: String): Int
    external fun iaGetGetSessionInfo(sessionId: Int): String?
    
    // Download operations
    external fun iaGetStartDownload(
        sessionId: Int,
        filesJson: String,
        progressCallback: (Double, String) -> Unit,
        completionCallback: (Boolean, String?) -> Unit
    ): Int
    
    external fun iaGetGetDownloadProgress(sessionId: Int): DownloadProgressInfo?
    external fun iaGetPauseDownload(sessionId: Int): Int
    external fun iaGetResumeDownload(sessionId: Int): Int
    external fun iaGetCancelDownload(sessionId: Int): Int
    external fun iaGetCancelOperation(operationId: Int): Int
    
    // Memory management
    external fun iaGetFreeString(ptr: Long)
    external fun iaGetLastError(): String?
    
    /**
     * Native download progress information
     */
    data class DownloadProgressInfo(
        val sessionId: Int,
        val overallProgress: Double,
        val currentFile: String,
        val currentFileProgress: Double,
        val downloadSpeed: Long,
        val etaSeconds: Long,
        val completedFiles: Int,
        val totalFiles: Int,
        val downloadedBytes: Long,
        val totalBytes: Long
    )
}