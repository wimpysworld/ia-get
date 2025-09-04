use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use std::hint::black_box;
use std::time::Duration;
use tokio::runtime::Runtime;

use ia_get::{
    infrastructure::http::http_client::{EnhancedHttpClient, HttpClientFactory},
    utilities::common::performance::{AdaptiveBufferManager, PerformanceMonitor},
};

/// Benchmark HTTP client creation with different configurations
fn bench_http_client_creation(c: &mut Criterion) {
    let mut group = c.benchmark_group("http_client_creation");

    group.bench_function("default_client", |b| {
        b.iter(|| {
            let client = EnhancedHttpClient::new();
            let _ = black_box(client);
        });
    });

    group.bench_function("archive_optimized_client", |b| {
        b.iter(|| {
            let client = HttpClientFactory::for_archive_downloads();
            let _ = black_box(client);
        });
    });

    group.bench_function("metadata_optimized_client", |b| {
        b.iter(|| {
            let client = HttpClientFactory::for_metadata_requests();
            let _ = black_box(client);
        });
    });

    group.bench_function("connectivity_test_client", |b| {
        b.iter(|| {
            let client = HttpClientFactory::for_connectivity_tests();
            let _ = black_box(client);
        });
    });

    group.finish();
}

/// Benchmark performance monitoring operations
fn bench_performance_monitoring(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("performance_monitoring");

    group.bench_function("monitor_creation", |b| {
        b.iter(|| {
            let monitor = PerformanceMonitor::new();
            black_box(monitor);
        });
    });

    group.bench_function("record_download", |b| {
        let monitor = PerformanceMonitor::new();
        b.iter(|| {
            rt.block_on(async {
                monitor
                    .record_download(
                        black_box(1024 * 1024),
                        black_box(Duration::from_millis(100)),
                    )
                    .await;
            });
        });
    });

    group.bench_function("record_failure", |b| {
        let monitor = PerformanceMonitor::new();
        b.iter(|| {
            rt.block_on(async {
                monitor.record_failure().await;
            });
        });
    });

    group.bench_function("get_metrics", |b| {
        let monitor = PerformanceMonitor::new();
        b.iter(|| {
            rt.block_on(async {
                let metrics = monitor.get_metrics().await;
                black_box(metrics);
            });
        });
    });

    group.bench_function("generate_report", |b| {
        let monitor = PerformanceMonitor::new();
        b.iter(|| {
            rt.block_on(async {
                let report = monitor.generate_report().await;
                black_box(report);
            });
        });
    });

    group.finish();
}

/// Benchmark adaptive buffer management
fn bench_adaptive_buffer_management(c: &mut Criterion) {
    let mut group = c.benchmark_group("adaptive_buffer_management");

    group.bench_function("buffer_manager_creation", |b| {
        b.iter(|| {
            let manager = AdaptiveBufferManager::new();
            black_box(manager);
        });
    });

    group.bench_function("get_buffer_size", |b| {
        let manager = AdaptiveBufferManager::new();
        b.iter(|| {
            let size = manager.get_buffer_size();
            black_box(size);
        });
    });

    group.bench_function("update_performance_single", |b| {
        let mut manager = AdaptiveBufferManager::new();
        b.iter(|| {
            manager.update_performance(black_box(1024.0 * 1024.0)); // 1MB/s
        });
    });

    group.bench_function("update_performance_multiple", |b| {
        let mut manager = AdaptiveBufferManager::new();
        b.iter(|| {
            for speed in [100.0, 500.0, 1000.0, 2000.0, 1500.0] {
                manager.update_performance(black_box(speed * 1024.0));
            }
        });
    });

    group.bench_function("optimal_buffer_for_file_size", |b| {
        let manager = AdaptiveBufferManager::new();
        b.iter(|| {
            let size = manager.get_optimal_buffer_for_file_size(black_box(50 * 1024 * 1024)); // 50MB
            black_box(size);
        });
    });

    group.finish();
}

/// Benchmark timeout calculations
fn bench_timeout_calculations(c: &mut Criterion) {
    let mut group = c.benchmark_group("timeout_calculations");

    let client = EnhancedHttpClient::new().unwrap();

    let file_sizes = [
        None,                     // Unknown size
        Some(1024),               // 1KB
        Some(1024 * 1024),        // 1MB
        Some(10 * 1024 * 1024),   // 10MB
        Some(100 * 1024 * 1024),  // 100MB
        Some(1024 * 1024 * 1024), // 1GB
    ];

    for (i, size) in file_sizes.iter().enumerate() {
        group.bench_with_input(
            BenchmarkId::new("calculate_timeout", i),
            size,
            |b, &size| {
                b.iter(|| {
                    let timeout = client.calculate_timeout(black_box(size));
                    black_box(timeout);
                });
            },
        );
    }

    group.finish();
}

/// Benchmark concurrent operations
fn bench_concurrent_operations(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("concurrent_operations");
    group.measurement_time(Duration::from_secs(10));

    group.bench_function("concurrent_monitor_updates", |b| {
        b.iter(|| {
            rt.block_on(async {
                let monitor = std::sync::Arc::new(PerformanceMonitor::new());
                let handles: Vec<_> = (0..10)
                    .map(|i| {
                        let monitor_clone = monitor.clone();
                        tokio::spawn(async move {
                            monitor_clone
                                .record_download(
                                    black_box((i + 1) * 1024 * 1024),
                                    black_box(Duration::from_millis(100)),
                                )
                                .await;
                        })
                    })
                    .collect();

                for handle in handles {
                    handle.await.unwrap();
                }

                let metrics = monitor.get_metrics().await;
                black_box(metrics);
            });
        });
    });

    group.bench_function("concurrent_buffer_access", |b| {
        b.iter(|| {
            rt.block_on(async {
                let client = EnhancedHttpClient::new().unwrap();
                let handles: Vec<_> = (0..5)
                    .map(|i| {
                        let client_clone = &client;
                        async move {
                            let size = client_clone
                                .get_optimal_buffer_size(Some(black_box(
                                    (i + 1) * 10 * 1024 * 1024,
                                )))
                                .await;
                            black_box(size);
                        }
                    })
                    .collect();

                for handle in handles {
                    handle.await;
                }
            });
        });
    });

    group.finish();
}

/// Benchmark memory efficiency scenarios
fn bench_memory_efficiency(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_efficiency");
    group.measurement_time(Duration::from_secs(15));

    // Test creating and dropping many monitors
    group.bench_function("monitor_lifecycle", |b| {
        b.iter(|| {
            let monitors: Vec<_> = (0..100).map(|_| PerformanceMonitor::new()).collect();
            black_box(monitors);
            // Monitors are dropped here
        });
    });

    // Test creating and dropping many buffer managers
    group.bench_function("buffer_manager_lifecycle", |b| {
        b.iter(|| {
            let managers: Vec<_> = (0..100).map(|_| AdaptiveBufferManager::new()).collect();
            black_box(managers);
            // Managers are dropped here
        });
    });

    // Test large performance history
    group.bench_function("large_performance_history", |b| {
        b.iter(|| {
            let mut manager = AdaptiveBufferManager::new();
            // Simulate a large number of performance updates
            for i in 0..1000 {
                let speed = (i % 100 + 1) as f64 * 1024.0; // Varying speeds
                manager.update_performance(black_box(speed));
            }
            black_box(manager);
        });
    });

    group.finish();
}

criterion_group!(
    performance_benches,
    bench_http_client_creation,
    bench_performance_monitoring,
    bench_adaptive_buffer_management,
    bench_timeout_calculations,
    bench_concurrent_operations,
    bench_memory_efficiency
);
criterion_main!(performance_benches);
