use std::path::Path;
use tempfile::tempdir;
use volterra_profiler::{
    BenchmarkOptions, DiskDiscoveryEngine, HardwareHealth, PerformanceMonitor, ProfilerError,
    SafeBenchmarker, TransferDirection,
};

#[test]
fn test_disk_discovery_root_volume() {
    let profile = DiskDiscoveryEngine::profile_path(Path::new("/")).expect("Profile root volume");
    assert!(!profile.device_id.is_empty(), "Device ID must not be empty");
    assert!(
        !profile.physical_parent_id.is_empty(),
        "Parent ID must not be empty"
    );
    assert!(profile.capacity_bytes > 0, "Capacity must be > 0");
    assert!(profile.available_bytes > 0, "Available bytes must be > 0");
    assert!(profile.block_size > 0, "Block size must be > 0");
}

#[test]
fn test_disk_discovery_multi_volume() {
    let disks = DiskDiscoveryEngine::discover_disks().expect("Discover all disks");
    assert!(!disks.is_empty(), "Should discover at least root disk");
    for d in &disks {
        assert!(!d.device_id.is_empty());
        assert!(!d.physical_parent_id.is_empty());
    }
}

#[test]
fn test_same_physical_disk_detection() {
    let dir1 = tempdir().unwrap();
    let dir2 = tempdir().unwrap();

    let rel = DiskDiscoveryEngine::evaluate_relationship(dir1.path(), dir2.path())
        .expect("Evaluate relationship");

    // Both temp dirs are on the same volume/container on the host
    assert!(rel.is_same_physical_disk);
    assert_eq!(rel.direction, TransferDirection::SameDiskTransfer);
    assert!(rel.warning.is_some());
}

#[test]
fn test_telemetry_ewma_recording() {
    let monitor = PerformanceMonitor::new();
    monitor.update_concurrency(2);
    monitor.update_chunk_size(4 * 1024 * 1024);
    monitor.update_buffer_usage(16 * 1024 * 1024, 64 * 1024 * 1024);

    // Simulate 10 read operations of 1 MB each (10 MB total)
    for _ in 0..10 {
        monitor.record_read(1024 * 1024, 5_000_000); // 5ms latency
    }

    // Simulate 10 write operations of 1 MB each (10 MB total)
    for _ in 0..10 {
        monitor.record_write(1024 * 1024, 8_000_000); // 8ms latency
    }

    let tel = monitor.get_telemetry();
    assert_eq!(tel.total_bytes_read, 10 * 1024 * 1024);
    assert_eq!(tel.total_bytes_written, 10 * 1024 * 1024);
    assert_eq!(tel.total_read_ops, 10);
    assert_eq!(tel.total_write_ops, 10);
    assert!(tel.p50_read_latency_ms >= 4.0 && tel.p50_read_latency_ms <= 6.0);
    assert!(tel.p50_write_latency_ms >= 7.0 && tel.p50_write_latency_ms <= 9.0);
    assert_eq!(tel.buffer_bytes_used, 16 * 1024 * 1024);
    assert_eq!(tel.buffer_capacity_bytes, 64 * 1024 * 1024);
    assert!((tel.buffer_utilization_pct - 25.0).abs() < 0.1);
    assert_eq!(tel.active_concurrency, 2);
    assert_eq!(tel.current_chunk_size, 4 * 1024 * 1024);
}

#[test]
fn test_cache_burst_discriminator() {
    let monitor = PerformanceMonitor::new();
    // Simulate sub-25us RAM cache burst write
    monitor.record_write(1024 * 1024, 15_000); // 15 us latency

    let tel = monitor.get_telemetry();
    assert!(
        tel.is_cache_burst,
        "Should flag cache burst for sub-25us writes"
    );
}

#[test]
fn test_safe_benchmark_active_roundtrip_and_cleanup() {
    let dir = tempdir().unwrap();
    let opts = BenchmarkOptions {
        test_size_bytes: 1024 * 1024, // 1 MiB small test for speed
        chunk_size_bytes: 256 * 1024,
        direct_io: false, // In test tempdir
        is_failing_disk: false,
        hardware_health: HardwareHealth::Clean,
    };

    let res = SafeBenchmarker::run_active_benchmark(dir.path(), opts).expect("Run safe benchmark");
    assert!(res.write_speed_mbps > 0.0);
    assert!(res.read_speed_mbps > 0.0);
    assert_eq!(res.test_size_bytes, 1024 * 1024);

    // Verify RAII cleanup: No .volterra.bench files left behind!
    let entries: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert!(entries.is_empty(), "Temp benchmark file must be cleaned up");
}

#[test]
fn test_safe_benchmark_failing_disk_prohibited() {
    let dir = tempdir().unwrap();
    let opts = BenchmarkOptions {
        test_size_bytes: 1024 * 1024,
        chunk_size_bytes: 256 * 1024,
        direct_io: false,
        is_failing_disk: true,
        hardware_health: HardwareHealth::Failing,
    };

    let res = SafeBenchmarker::run_active_benchmark(dir.path(), opts);
    match res {
        Err(ProfilerError::FailingDiskBenchmarkProhibited { .. }) => {}
        other => panic!("Expected FailingDiskBenchmarkProhibited, got {:?}", other),
    }
}

#[test]
fn test_safe_benchmark_insufficient_space_rejected() {
    let dir = tempdir().unwrap();
    let opts = BenchmarkOptions {
        test_size_bytes: 10 * 1024 * 1024 * 1024 * 1024, // 10 TB!
        chunk_size_bytes: 1024 * 1024,
        direct_io: false,
        is_failing_disk: false,
        hardware_health: HardwareHealth::Clean,
    };

    let res = SafeBenchmarker::run_active_benchmark(dir.path(), opts);
    match res {
        Err(ProfilerError::InsufficientSpaceForBenchmark { .. }) => {}
        other => panic!("Expected InsufficientSpaceForBenchmark, got {:?}", other),
    }
}
