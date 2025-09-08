package com.gameaday.ia_get_mobile

import android.content.Intent
import android.net.Uri
import android.os.Bundle
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel

/**
 * Main Activity for the Internet Archive Get mobile app
 * Handles deep links and integrates with the background download service
 */
class MainActivity: FlutterActivity() {
    
    private val methodChannelName = "com.gameaday.ia_get_mobile/platform"
    private lateinit var methodChannel: MethodChannel
    
    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        
        // Set up platform method channel for native integration
        methodChannel = MethodChannel(flutterEngine.dartExecutor.binaryMessenger, methodChannelName)
        methodChannel.setMethodCallHandler { call, result ->
            when (call.method) {
                "startDownloadService" -> {
                    val identifier = call.argument<String>("identifier")
                    val outputDir = call.argument<String>("outputDir")
                    val configJson = call.argument<String>("configJson")
                    val filesJson = call.argument<String>("filesJson")
                    
                    if (identifier != null && outputDir != null && configJson != null && filesJson != null) {
                        startDownloadService(identifier, outputDir, configJson, filesJson)
                        result.success(true)
                    } else {
                        result.error("INVALID_ARGUMENTS", "Missing required arguments", null)
                    }
                }
                "pauseDownload" -> {
                    val sessionId = call.argument<Int>("sessionId")
                    if (sessionId != null) {
                        pauseDownload(sessionId)
                        result.success(true)
                    } else {
                        result.error("INVALID_ARGUMENTS", "Missing sessionId", null)
                    }
                }
                "resumeDownload" -> {
                    val sessionId = call.argument<Int>("sessionId")
                    if (sessionId != null) {
                        resumeDownload(sessionId)
                        result.success(true)
                    } else {
                        result.error("INVALID_ARGUMENTS", "Missing sessionId", null)
                    }
                }
                "cancelDownload" -> {
                    val sessionId = call.argument<Int>("sessionId")
                    if (sessionId != null) {
                        cancelDownload(sessionId)
                        result.success(true)
                    } else {
                        result.error("INVALID_ARGUMENTS", "Missing sessionId", null)
                    }
                }
                "getAppVersion" -> {
                    try {
                        val packageInfo = packageManager.getPackageInfo(packageName, 0)
                        result.success(packageInfo.versionName)
                    } catch (e: Exception) {
                        result.error("VERSION_ERROR", "Failed to get app version", e.message)
                    }
                }
                "shareFile" -> {
                    val filePath = call.argument<String>("filePath")
                    val mimeType = call.argument<String>("mimeType")
                    if (filePath != null) {
                        shareFile(filePath, mimeType ?: "*/*")
                        result.success(true)
                    } else {
                        result.error("INVALID_ARGUMENTS", "Missing filePath", null)
                    }
                }
                "openFile" -> {
                    val filePath = call.argument<String>("filePath")
                    val mimeType = call.argument<String>("mimeType")
                    if (filePath != null) {
                        openFile(filePath, mimeType ?: "*/*")
                        result.success(true)
                    } else {
                        result.error("INVALID_ARGUMENTS", "Missing filePath", null)
                    }
                }
                else -> {
                    result.notImplemented()
                }
            }
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        
        // Handle deep link if present
        handleIntent(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        handleIntent(intent)
    }

    /**
     * Handle incoming intents, including deep links to Internet Archive URLs
     */
    private fun handleIntent(intent: Intent) {
        when (intent.action) {
            Intent.ACTION_VIEW -> {
                intent.data?.let { uri ->
                    handleDeepLink(uri)
                }
            }
            Intent.ACTION_SEND -> {
                if (intent.type == "text/plain") {
                    intent.getStringExtra(Intent.EXTRA_TEXT)?.let { sharedText ->
                        handleSharedText(sharedText)
                    }
                }
            }
        }
    }

    /**
     * Handle deep link URLs (archive.org links and custom iaget:// scheme)
     */
    private fun handleDeepLink(uri: Uri) {
        val identifier = extractIdentifierFromUri(uri)
        if (identifier != null) {
            // Send deep link data to Flutter app
            methodChannel.invokeMethod("deepLink", mapOf(
                "url" to uri.toString(),
                "identifier" to identifier,
                "host" to uri.host
            ))
        }
    }

    /**
     * Handle shared text that might contain Internet Archive URLs
     */
    private fun handleSharedText(text: String) {
        // Look for archive.org URLs in shared text
        val archiveRegex = Regex("https?://archive\\.org/(?:details|download)/([^\\s/?]+)")
        val match = archiveRegex.find(text)
        
        if (match != null) {
            val identifier = match.groupValues[1]
            methodChannel.invokeMethod("sharedText", mapOf(
                "text" to text,
                "identifier" to identifier
            ))
        } else {
            methodChannel.invokeMethod("sharedText", mapOf(
                "text" to text
            ))
        }
    }

    /**
     * Extract archive identifier from various URL formats
     */
    private fun extractIdentifierFromUri(uri: Uri): String? {
        return when (uri.scheme) {
            "iaget" -> {
                // Custom scheme: iaget://identifier
                uri.host ?: uri.path?.removePrefix("/")
            }
            "https", "http" -> {
                if (uri.host == "archive.org") {
                    // Extract from archive.org URLs
                    val pathSegments = uri.pathSegments
                    if (pathSegments.size >= 2 && 
                        (pathSegments[0] == "details" || pathSegments[0] == "download")) {
                        pathSegments[1]
                    } else null
                } else null
            }
            else -> null
        }
    }

    /**
     * Start the background download service
     */
    private fun startDownloadService(identifier: String, outputDir: String, configJson: String, filesJson: String) {
        val serviceIntent = Intent(this, DownloadService::class.java).apply {
            action = DownloadService.ACTION_START_DOWNLOAD
            putExtra(DownloadService.EXTRA_IDENTIFIER, identifier)
            putExtra(DownloadService.EXTRA_OUTPUT_DIR, outputDir)
            putExtra(DownloadService.EXTRA_CONFIG_JSON, configJson)
            putExtra(DownloadService.EXTRA_FILES_JSON, filesJson)
        }
        startForegroundService(serviceIntent)
    }

    /**
     * Pause a download session
     */
    private fun pauseDownload(sessionId: Int) {
        val serviceIntent = Intent(this, DownloadService::class.java).apply {
            action = DownloadService.ACTION_PAUSE_DOWNLOAD
            putExtra(DownloadService.EXTRA_SESSION_ID, sessionId)
        }
        startService(serviceIntent)
    }

    /**
     * Resume a download session
     */
    private fun resumeDownload(sessionId: Int) {
        val serviceIntent = Intent(this, DownloadService::class.java).apply {
            action = DownloadService.ACTION_RESUME_DOWNLOAD
            putExtra(DownloadService.EXTRA_SESSION_ID, sessionId)
        }
        startService(serviceIntent)
    }

    /**
     * Cancel a download session
     */
    private fun cancelDownload(sessionId: Int) {
        val serviceIntent = Intent(this, DownloadService::class.java).apply {
            action = DownloadService.ACTION_CANCEL_DOWNLOAD
            putExtra(DownloadService.EXTRA_SESSION_ID, sessionId)
        }
        startService(serviceIntent)
    }

    /**
     * Share a downloaded file using Android sharing
     */
    private fun shareFile(filePath: String, mimeType: String) {
        try {
            val file = java.io.File(filePath)
            if (file.exists()) {
                val uri = androidx.core.content.FileProvider.getUriForFile(
                    this,
                    "${packageName}.fileprovider",
                    file
                )
                
                val shareIntent = Intent(Intent.ACTION_SEND).apply {
                    type = mimeType
                    putExtra(Intent.EXTRA_STREAM, uri)
                    addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
                }
                
                startActivity(Intent.createChooser(shareIntent, "Share file"))
            }
        } catch (e: Exception) {
            // Handle sharing error
            methodChannel.invokeMethod("error", mapOf(
                "type" to "share_error",
                "message" to "Failed to share file: ${e.message}"
            ))
        }
    }

    /**
     * Open a downloaded file with appropriate app
     */
    private fun openFile(filePath: String, mimeType: String) {
        try {
            val file = java.io.File(filePath)
            if (file.exists()) {
                val uri = androidx.core.content.FileProvider.getUriForFile(
                    this,
                    "${packageName}.fileprovider",
                    file
                )
                
                val openIntent = Intent(Intent.ACTION_VIEW).apply {
                    setDataAndType(uri, mimeType)
                    addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
                    addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                }
                
                if (openIntent.resolveActivity(packageManager) != null) {
                    startActivity(openIntent)
                } else {
                    methodChannel.invokeMethod("error", mapOf(
                        "type" to "open_error",
                        "message" to "No app found to open this file type"
                    ))
                }
            }
        } catch (e: Exception) {
            // Handle opening error
            methodChannel.invokeMethod("error", mapOf(
                "type" to "open_error",
                "message" to "Failed to open file: ${e.message}"
            ))
        }
    }
}