//! Performance optimization utilities for ia-get
//!
//! This module provides performance monitoring, optimization strategies,
//! and adaptive algorithms to improve download speeds and resource usage.

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// Performance metrics collected during downloads
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceMetrics {
    /// Total bytes downloaded
    pub total_bytes: u64,
    /// Total time elapsed
    pub total_duration: Duration,
    /// Average download speed in bytes per second
    pub avg_speed: f64,
    /// Peak download speed in bytes per second
    pub peak_speed: f64,
    /// Number of successful downloads
    pub successful_downloads: u64,
    /// Number of failed downloads
    pub failed_downloads: u64,
    /// Number of retries performed
    pub retry_count: u64,
    /// Memory usage statistics
    pub memory_stats: MemoryStats,
    /// Connection statistics
    pub connection_stats: ConnectionStats,
}

/// Memory usage statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryStats {
    /// Peak memory usage in bytes
    pub peak_memory: u64,
    /// Current memory usage in bytes
    pub current_memory: u64,
    /// Number of allocations
    pub allocations: u64,
}

/// Connection statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionStats {
    /// Number of connections established
    pub connections_established: u64,
    /// Number of connections reused
    pub connections_reused: u64,
    /// Average connection establishment time
    pub avg_connection_time: Duration,
    /// Number of connection timeouts
    pub connection_timeouts: u64,
}

/// Performance monitor for tracking download metrics
pub struct PerformanceMonitor {
    metrics: Arc<Mutex<PerformanceMetrics>>,
    start_time: Instant,
}

impl PerformanceMonitor {
    /// Create a new performance monitor
    pub fn new() -> Self {
        Self {
            metrics: Arc::new(Mutex::new(PerformanceMetrics {
                total_bytes: 0,
                total_duration: Duration::ZERO,
                avg_speed: 0.0,
                peak_speed: 0.0,
                successful_downloads: 0,
                failed_downloads: 0,
                retry_count: 0,
                memory_stats: MemoryStats {
                    peak_memory: 0,
                    current_memory: 0,
                    allocations: 0,
                },
                connection_stats: ConnectionStats {
                    connections_established: 0,
                    connections_reused: 0,
                    avg_connection_time: Duration::ZERO,
                    connection_timeouts: 0,
                },
            })),
            start_time: Instant::now(),
        }
    }

    /// Record a successful download
    pub async fn record_download(&self, bytes: u64, duration: Duration) {
        let mut metrics = self.metrics.lock().await;
        metrics.total_bytes += bytes;
        metrics.successful_downloads += 1;

        let speed = bytes as f64 / duration.as_secs_f64();
        if speed > metrics.peak_speed {
            metrics.peak_speed = speed;
        }

        // Update average speed
        let total_time = self.start_time.elapsed();
        metrics.avg_speed = metrics.total_bytes as f64 / total_time.as_secs_f64();
        metrics.total_duration = total_time;
    }

    /// Record a failed download
    pub async fn record_failure(&self) {
        let mut metrics = self.metrics.lock().await;
        metrics.failed_downloads += 1;
    }

    /// Record a retry attempt
    pub async fn record_retry(&self) {
        let mut metrics = self.metrics.lock().await;
        metrics.retry_count += 1;
    }

    /// Record connection establishment
    pub async fn record_connection(&self, establishment_time: Duration, reused: bool) {
        let mut metrics = self.metrics.lock().await;

        if reused {
            metrics.connection_stats.connections_reused += 1;
        } else {
            metrics.connection_stats.connections_established += 1;
        }

        // Update average connection time (simple moving average for new connections)
        if !reused {
            let total_new_connections = metrics.connection_stats.connections_established;
            let current_avg = metrics.connection_stats.avg_connection_time;
            metrics.connection_stats.avg_connection_time =
                (current_avg * (total_new_connections - 1) as u32 + establishment_time)
                    / total_new_connections as u32;
        }
    }

    /// Record connection timeout
    pub async fn record_connection_timeout(&self) {
        let mut metrics = self.metrics.lock().await;
        metrics.connection_stats.connection_timeouts += 1;
    }

    /// Update memory usage statistics
    pub async fn update_memory_usage(&self, current: u64, peak: u64, allocations: u64) {
        let mut metrics = self.metrics.lock().await;
        metrics.memory_stats.current_memory = current;
        if peak > metrics.memory_stats.peak_memory {
            metrics.memory_stats.peak_memory = peak;
        }
        metrics.memory_stats.allocations = allocations;
    }

    /// Get current performance metrics
    pub async fn get_metrics(&self) -> PerformanceMetrics {
        let mut metrics = self.metrics.lock().await;
        metrics.total_duration = self.start_time.elapsed();
        metrics.clone()
    }

    /// Generate a performance report
    pub async fn generate_report(&self) -> String {
        let metrics = self.get_metrics().await;

        format!(
            "Performance Report:\n\
             ==================\n\
             Total bytes downloaded: {} ({:.2} MB)\n\
             Total time: {:.2}s\n\
             Average speed: {:.2} MB/s\n\
             Peak speed: {:.2} MB/s\n\
             Successful downloads: {}\n\
             Failed downloads: {}\n\
             Retry attempts: {}\n\
             Success rate: {:.1}%\n\
             \n\
             Connection Stats:\n\
             ================\n\
             New connections: {}\n\
             Reused connections: {}\n\
             Connection reuse rate: {:.1}%\n\
             Avg connection time: {:.2}ms\n\
             Connection timeouts: {}\n\
             \n\
             Memory Stats:\n\
             =============\n\
             Peak memory usage: {:.2} MB\n\
             Current memory usage: {:.2} MB\n\
             Total allocations: {}\n",
            metrics.total_bytes,
            metrics.total_bytes as f64 / 1_048_576.0,
            metrics.total_duration.as_secs_f64(),
            metrics.avg_speed / 1_048_576.0,
            metrics.peak_speed / 1_048_576.0,
            metrics.successful_downloads,
            metrics.failed_downloads,
            metrics.retry_count,
            if metrics.successful_downloads + metrics.failed_downloads > 0 {
                100.0 * metrics.successful_downloads as f64
                    / (metrics.successful_downloads + metrics.failed_downloads) as f64
            } else {
                0.0
            },
            metrics.connection_stats.connections_established,
            metrics.connection_stats.connections_reused,
            if metrics.connection_stats.connections_established
                + metrics.connection_stats.connections_reused
                > 0
            {
                100.0 * metrics.connection_stats.connections_reused as f64
                    / (metrics.connection_stats.connections_established
                        + metrics.connection_stats.connections_reused) as f64
            } else {
                0.0
            },
            metrics.connection_stats.avg_connection_time.as_millis(),
            metrics.connection_stats.connection_timeouts,
            metrics.memory_stats.peak_memory as f64 / 1_048_576.0,
            metrics.memory_stats.current_memory as f64 / 1_048_576.0,
            metrics.memory_stats.allocations,
        )
    }
}

impl Default for PerformanceMonitor {
    fn default() -> Self {
        Self::new()
    }
}

/// Adaptive buffer size calculator
pub struct AdaptiveBufferManager {
    current_size: usize,
    min_size: usize,
    max_size: usize,
    performance_history: Vec<(usize, f64)>, // (buffer_size, speed)
}

impl AdaptiveBufferManager {
    /// Create a new adaptive buffer manager
    pub fn new() -> Self {
        Self {
            current_size: 64 * 1024, // Start with 64KB
            min_size: 8 * 1024,      // Minimum 8KB
            max_size: 1024 * 1024,   // Maximum 1MB
            performance_history: Vec::new(),
        }
    }

    /// Get the current optimal buffer size
    pub fn get_buffer_size(&self) -> usize {
        self.current_size
    }

    /// Update buffer size based on performance feedback
    pub fn update_performance(&mut self, speed: f64) {
        self.performance_history.push((self.current_size, speed));

        // Keep only recent history (last 10 measurements)
        if self.performance_history.len() > 10 {
            self.performance_history.remove(0);
        }

        // Adjust buffer size based on performance trend
        if self.performance_history.len() >= 3 {
            let recent_speeds: Vec<f64> = self
                .performance_history
                .iter()
                .rev()
                .take(3)
                .map(|(_, speed)| *speed)
                .collect();

            let avg_recent_speed = recent_speeds.iter().sum::<f64>() / recent_speeds.len() as f64;

            // If performance is improving and we're not at max, try increasing
            if recent_speeds.windows(2).all(|w| w[0] < w[1]) && self.current_size < self.max_size {
                self.current_size = (self.current_size * 2).min(self.max_size);
            }
            // If performance is declining and we're not at min, try decreasing
            else if recent_speeds.windows(2).all(|w| w[0] > w[1])
                && self.current_size > self.min_size
            {
                self.current_size = (self.current_size / 2).max(self.min_size);
            }
            // If performance is inconsistent, find the best performing buffer size from history
            else if self.performance_history.len() >= 5 {
                let best_performer = self
                    .performance_history
                    .iter()
                    .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

                if let Some((best_size, best_speed)) = best_performer {
                    if *best_speed > avg_recent_speed * 1.1 {
                        // 10% improvement threshold
                        self.current_size = *best_size;
                    }
                }
            }
        }
    }

    /// Get optimal buffer size for a specific file size
    pub fn get_optimal_buffer_for_file_size(&self, file_size: u64) -> usize {
        if file_size < 1024 * 1024 {
            // Files < 1MB
            (self.current_size / 2).max(self.min_size)
        } else if file_size > 100 * 1024 * 1024 {
            // Files > 100MB
            self.max_size
        } else {
            self.current_size
        }
    }
}

impl Default for AdaptiveBufferManager {
    fn default() -> Self {
        Self::new()
    }
}
