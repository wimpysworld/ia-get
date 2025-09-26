package com.gameaday.internet_archive_helper

import android.app.*
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.IBinder
import android.os.PowerManager
import androidx.annotation.RequiresApi
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import io.flutter.plugin.common.MethodChannel
import kotlinx.coroutines.*
import java.util.concurrent.ConcurrentHashMap
import kotlin.coroutines.CoroutineContext

/**
 * Background download service for Internet Archive downloads
 * Integrates with the Rust FFI library for actual download operations
 */
class DownloadService : Service(), CoroutineScope {

    companion object {
        const val CHANNEL_ID = "ia_get_downloads"
        const val NOTIFICATION_ID = 1
        
        // Intent actions
        const val ACTION_START_DOWNLOAD = "com.gameaday.ia_get_mobile.START_DOWNLOAD"
        const val ACTION_PAUSE_DOWNLOAD = "com.gameaday.ia_get_mobile.PAUSE_DOWNLOAD"
        const val ACTION_RESUME_DOWNLOAD = "com.gameaday.ia_get_mobile.RESUME_DOWNLOAD"
        const val ACTION_CANCEL_DOWNLOAD = "com.gameaday.ia_get_mobile.CANCEL_DOWNLOAD"
        
        // Intent extras
        const val EXTRA_IDENTIFIER = "identifier"
        const val EXTRA_OUTPUT_DIR = "output_dir"
        const val EXTRA_SESSION_ID = "session_id"
        const val EXTRA_CONFIG_JSON = "config_json"
        const val EXTRA_FILES_JSON = "files_json"
    }

    private lateinit var job: Job
    override val coroutineContext: CoroutineContext
        get() = Dispatchers.Main + job

    private var wakeLock: PowerManager.WakeLock? = null
    private var methodChannel: MethodChannel? = null
    private val activeSessions = ConcurrentHashMap<Int, DownloadSessionInfo>()
    
    // Native library interface
    private val iaGetFFI = IaGetNativeWrapper()

    data class DownloadSessionInfo(
        val sessionId: Int,
        val identifier: String,
        val outputDir: String,
        var isActive: Boolean = true,
        var isPaused: Boolean = false,
        var progress: Double = 0.0,
        var currentFile: String = "",
        var downloadSpeed: Long = 0,
        var completedFiles: Int = 0,
        var totalFiles: Int = 0
    )

    override fun onCreate() {
        super.onCreate()
        job = Job()
        
        // Initialize FFI library
        iaGetFFI.iaGetInit()
        
        // Create notification channel
        createNotificationChannel()
        
        // Acquire wake lock for background operations
        acquireWakeLock()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        intent?.let { processIntent(it) }
        return START_STICKY
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onDestroy() {
        super.onDestroy()
        
        // Cancel all active downloads
        activeSessions.keys.forEach { sessionId ->
            iaGetFFI.iaGetCancelDownload(sessionId)
        }
        activeSessions.clear()
        
        // Release wake lock
        wakeLock?.let {
            if (it.isHeld) {
                it.release()
            }
        }
        
        // Cancel coroutines
        job.cancel()
        
        // Cleanup FFI
        iaGetFFI.iaGetCleanup()
    }

    private fun processIntent(intent: Intent) {
        when (intent.action) {
            ACTION_START_DOWNLOAD -> {
                val identifier = intent.getStringExtra(EXTRA_IDENTIFIER) ?: return
                val outputDir = intent.getStringExtra(EXTRA_OUTPUT_DIR) ?: return
                val configJson = intent.getStringExtra(EXTRA_CONFIG_JSON) ?: return
                val filesJson = intent.getStringExtra(EXTRA_FILES_JSON) ?: return
                
                startDownload(identifier, outputDir, configJson, filesJson)
            }
            ACTION_PAUSE_DOWNLOAD -> {
                val sessionId = intent.getIntExtra(EXTRA_SESSION_ID, -1)
                if (sessionId > 0) pauseDownload(sessionId)
            }
            ACTION_RESUME_DOWNLOAD -> {
                val sessionId = intent.getIntExtra(EXTRA_SESSION_ID, -1)
                if (sessionId > 0) resumeDownload(sessionId)
            }
            ACTION_CANCEL_DOWNLOAD -> {
                val sessionId = intent.getIntExtra(EXTRA_SESSION_ID, -1)
                if (sessionId > 0) cancelDownload(sessionId)
            }
        }
    }

    private fun startDownload(identifier: String, outputDir: String, configJson: String, filesJson: String) {
        launch {
            try {
                // Create download session using FFI
                val sessionId = iaGetFFI.iaGetCreateSession(identifier, configJson)
                
                if (sessionId > 0) {
                    val sessionInfo = DownloadSessionInfo(
                        sessionId = sessionId,
                        identifier = identifier,
                        outputDir = outputDir
                    )
                    activeSessions[sessionId] = sessionInfo
                    
                    // Start foreground service with notification
                    startForeground(NOTIFICATION_ID, createDownloadNotification(sessionInfo))
                    
                    // Start the actual download using FFI
                    iaGetFFI.iaGetStartDownload(
                        sessionId = sessionId,
                        filesJson = filesJson,
                        progressCallback = { progress, message ->
                            updateDownloadProgress(sessionId, progress, message)
                        },
                        completionCallback = { success, errorMessage ->
                            onDownloadComplete(sessionId, success, errorMessage)
                        }
                    )
                    
                    // Start progress monitoring
                    startProgressMonitoring(sessionId)
                }
            } catch (e: Exception) {
                notifyError("Failed to start download: ${e.message}")
            }
        }
    }

    private fun pauseDownload(sessionId: Int) {
        activeSessions[sessionId]?.let { session ->
            session.isPaused = true
            iaGetFFI.iaGetPauseDownload(sessionId)
            updateNotification(session)
            notifyFlutter("download_paused", mapOf("sessionId" to sessionId))
        }
    }

    private fun resumeDownload(sessionId: Int) {
        activeSessions[sessionId]?.let { session ->
            session.isPaused = false
            iaGetFFI.iaGetResumeDownload(sessionId)
            updateNotification(session)
            notifyFlutter("download_resumed", mapOf("sessionId" to sessionId))
        }
    }

    private fun cancelDownload(sessionId: Int) {
        activeSessions[sessionId]?.let { session ->
            session.isActive = false
            iaGetFFI.iaGetCancelDownload(sessionId)
            activeSessions.remove(sessionId)
            
            notifyFlutter("download_cancelled", mapOf("sessionId" to sessionId))
            
            // Stop foreground service if no active downloads
            if (activeSessions.isEmpty()) {
                stopForeground(true)
                stopSelf()
            }
        }
    }

    private fun updateDownloadProgress(sessionId: Int, progress: Double, message: String) {
        activeSessions[sessionId]?.let { session ->
            session.progress = progress
            session.currentFile = message
            
            // Update notification
            updateNotification(session)
            
            // Notify Flutter app
            notifyFlutter("download_progress", mapOf(
                "sessionId" to sessionId,
                "progress" to progress,
                "message" to message,
                "currentFile" to session.currentFile,
                "downloadSpeed" to session.downloadSpeed,
                "completedFiles" to session.completedFiles,
                "totalFiles" to session.totalFiles
            ))
        }
    }

    private fun onDownloadComplete(sessionId: Int, success: Boolean, errorMessage: String?) {
        activeSessions[sessionId]?.let { session ->
            session.isActive = false
            activeSessions.remove(sessionId)
            
            if (success) {
                notifyFlutter("download_completed", mapOf(
                    "sessionId" to sessionId,
                    "identifier" to session.identifier
                ))
                showCompletionNotification(session.identifier, true)
            } else {
                notifyFlutter("download_failed", mapOf(
                    "sessionId" to sessionId,
                    "error" to (errorMessage ?: "Unknown error")
                ))
                showCompletionNotification(session.identifier, false, errorMessage)
            }
            
            // Stop foreground service if no active downloads
            if (activeSessions.isEmpty()) {
                stopForeground(true)
                stopSelf()
            }
        }
    }

    private fun startProgressMonitoring(sessionId: Int) {
        launch {
            while (activeSessions.containsKey(sessionId) && activeSessions[sessionId]?.isActive == true) {
                try {
                    val progressInfo = iaGetFFI.iaGetGetDownloadProgress(sessionId)
                    progressInfo?.let { info ->
                        activeSessions[sessionId]?.let { session ->
                            session.progress = info.overallProgress
                            session.currentFile = info.currentFile
                            session.downloadSpeed = info.downloadSpeed
                            session.completedFiles = info.completedFiles
                            session.totalFiles = info.totalFiles
                            
                            updateNotification(session)
                        }
                    }
                } catch (e: Exception) {
                    // Handle monitoring error
                    notifyError("Progress monitoring error: ${e.message}")
                }
                
                delay(1000) // Update every second
            }
        }
    }

    private fun createNotificationChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val channel = NotificationChannel(
                CHANNEL_ID,
                "Downloads",
                NotificationManager.IMPORTANCE_LOW
            ).apply {
                description = "Internet Archive download progress"
                setShowBadge(false)
                setSound(null, null)
            }
            
            val notificationManager = getSystemService(NotificationManager::class.java)
            notificationManager.createNotificationChannel(channel)
        }
    }

    private fun createDownloadNotification(session: DownloadSessionInfo): Notification {
        val cancelIntent = Intent(this, DownloadService::class.java).apply {
            action = ACTION_CANCEL_DOWNLOAD
            putExtra(EXTRA_SESSION_ID, session.sessionId)
        }
        val cancelPendingIntent = PendingIntent.getService(
            this, session.sessionId, cancelIntent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )

        val pauseResumeIntent = Intent(this, DownloadService::class.java).apply {
            action = if (session.isPaused) ACTION_RESUME_DOWNLOAD else ACTION_PAUSE_DOWNLOAD
            putExtra(EXTRA_SESSION_ID, session.sessionId)
        }
        val pauseResumePendingIntent = PendingIntent.getService(
            this, session.sessionId + 1000, pauseResumeIntent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )

        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle("Downloading ${session.identifier}")
            .setContentText(if (session.isPaused) "Paused" else "In progress...")
            .setSmallIcon(android.R.drawable.stat_sys_download)
            .setProgress(100, (session.progress * 100).toInt(), false)
            .setOngoing(true)
            .addAction(
                android.R.drawable.ic_media_pause,
                if (session.isPaused) "Resume" else "Pause",
                pauseResumePendingIntent
            )
            .addAction(
                android.R.drawable.ic_menu_close_clear_cancel,
                "Cancel",
                cancelPendingIntent
            )
            .build()
    }

    private fun updateNotification(session: DownloadSessionInfo) {
        try {
            val notification = createDownloadNotification(session)
            val notificationManager = NotificationManagerCompat.from(this)
            
            if (checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS) == 
                PackageManager.PERMISSION_GRANTED) {
                notificationManager.notify(NOTIFICATION_ID, notification)
            }
        } catch (e: Exception) {
            // Handle notification update error
        }
    }

    private fun showCompletionNotification(identifier: String, success: Boolean, errorMessage: String? = null) {
        try {
            val title = if (success) "Download completed" else "Download failed"
            val text = if (success) identifier else (errorMessage ?: "Unknown error")
            
            val notification = NotificationCompat.Builder(this, CHANNEL_ID)
                .setContentTitle(title)
                .setContentText(text)
                .setSmallIcon(if (success) android.R.drawable.stat_sys_download_done else android.R.drawable.stat_notify_error)
                .setAutoCancel(true)
                .build()
            
            val notificationManager = NotificationManagerCompat.from(this)
            if (checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS) == 
                PackageManager.PERMISSION_GRANTED) {
                notificationManager.notify(identifier.hashCode(), notification)
            }
        } catch (e: Exception) {
            // Handle completion notification error
        }
    }

    private fun acquireWakeLock() {
        val powerManager = getSystemService(Context.POWER_SERVICE) as PowerManager
        wakeLock = powerManager.newWakeLock(
            PowerManager.PARTIAL_WAKE_LOCK,
            "IaGetMobile::DownloadWakeLock"
        ).apply {
            acquire(10 * 60 * 1000L) // 10 minutes max
        }
    }

    private fun notifyFlutter(method: String, arguments: Map<String, Any>) {
        methodChannel?.invokeMethod(method, arguments)
    }

    private fun notifyError(message: String) {
        notifyFlutter("download_error", mapOf("error" to message))
    }

    // Set method channel for Flutter communication
    fun setMethodChannel(channel: MethodChannel) {
        this.methodChannel = channel
    }
}