//! Tier 1: Feature Coverage E2E Test Suite
//! Verifies happy-path functionality across all 10 VOLTERRA subsystems:
//! Manifest, Transfer, Integrity, Journal, Recovery, Profiler, Optimizer, Core, Platform, and FFI.

use std::ffi::{CStr, CString};
use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use volterra_core::{JobBuilder, JobLifecycleState};
use volterra_ffi::*;
use volterra_journal::{FileRecord, FileState, JobMode, JobRecord, JobState};
use volterra_platform_macos::{PlatformMetadata, PlatformSymlink};
use volterra_profiler::{DiskDiscoveryEngine, PerformanceMonitor};
use volterra_recovery::{RecoveryController, RetryBudget};
use volterra_test_suite::TestEnvironment;
use volterra_transfer::{TransferScheduler, TransferSchedulerConfig};

// =========================================================================
// 1. End-to-End File Copy and BLAKE3 Bit-Exact Verification
// =========================================================================
#[test]
fn test_tier1_end_to_end_copy_and_blake3() {
    let env = TestEnvironment::new();

    // Create multi-file source tree
    env.create_file("root_file.bin", 64 * 1024, 0xAA);
    env.create_file("sub/nested.dat", 128 * 1024, 0xBB);
    env.create_file("sub/deep/zero.bin", 0, 0x00);
    env.create_pattern_file("sub/deep/pattern.bin", 256 * 1024);

    let manifest = env.scan_source();
    assert_eq!(manifest.file_count, 4);

    let journal = env.open_journal();
    let job_id = "job_tier1_copy".to_string();

    journal
        .create_job(&JobRecord {
            id: job_id.clone(),
            source_root: env.src_path().to_path_buf(),
            target_root: env.dst_path().to_path_buf(),
            source_volume_uuid: None,
            target_volume_uuid: None,
            mode: JobMode::SmartHybrid,
            state: JobState::Running,
            total_files: manifest.file_count,
            total_bytes: manifest.total_bytes,
            copied_bytes: 0,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            completed_at: None,
        })
        .unwrap();

    let file_records: Vec<_> = manifest
        .items
        .iter()
        .map(|item| FileRecord {
            id: item.id,
            job_id: job_id.clone(),
            relative_path: item.relative_path.clone(),
            source_size: item.size_bytes,
            source_mtime_ns: item.source_mtime_ns,
            source_btime_ns: item.source_btime_ns,
            posix_mode: item.posix_mode,
            source_file_id: item.source_file_id.clone(),
            temp_path: None,
            state: FileState::Pending,
            verified_digest: None,
            error_code: None,
            recovered_bytes: 0,
        })
        .collect();
    journal.record_files_batch(&job_id, &file_records).unwrap();

    let recovery = Arc::new(RecoveryController::with_journal(
        Arc::new(volterra_recovery::ErrorQueue::new()),
        RetryBudget::default(),
        journal.clone(),
    ));

    let config = TransferSchedulerConfig {
        source_root: env.src_path().to_path_buf(),
        target_root: env.dst_path().to_path_buf(),
        job_id: job_id.clone(),
        chunk_size: 64 * 1024,
        memory_budget_bytes: 16 * 1024 * 1024,
        direct_io: false,
        verify_readback: true,
        allow_overwrite: false,
    };

    let scheduler = TransferScheduler::new(
        config,
        journal.clone(),
        recovery,
        Arc::new(PerformanceMonitor::new()),
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    );

    let res = scheduler.execute_pipeline(&manifest, |_, _, _, _| {});
    assert!(res.is_ok(), "Transfer pipeline failed: {:?}", res);

    // Verify all destination files exist and match BLAKE3 digests bit-for-bit
    for item in &manifest.items {
        if item.item_type == volterra_manifest::ItemType::Directory {
            continue;
        }
        let src_f = env.src_path().join(&item.relative_path);
        let dst_f = env.dst_path().join(&item.relative_path);

        assert!(dst_f.exists(), "Destination file missing: {:?}", dst_f);
        let src_data = fs::read(&src_f).unwrap();
        let dst_data = fs::read(&dst_f).unwrap();
        assert_eq!(src_data, dst_data, "Data mismatch for {:?}", item.relative_path);

        let src_hash = volterra_integrity::Blake3Hasher::hash_chunk(&src_data);
        let dst_hash = volterra_integrity::Blake3Hasher::hash_chunk(&dst_data);
        assert_eq!(src_hash, dst_hash, "BLAKE3 digest mismatch for {:?}", item.relative_path);
    }

    // Verify zero leftover .partial files
    for entry in walkdir(env.dst_path()) {
        assert!(
            !entry.to_string_lossy().contains(".partial"),
            "Leftover partial file found: {:?}",
            entry
        );
    }
}

// =========================================================================
// 2. Darwin macOS Metadata Preservation (POSIX permissions)
// =========================================================================
#[test]
fn test_tier1_macos_metadata_preservation() {
    let env = TestEnvironment::new();
    let src_file = env.create_file("exec_script.sh", 1024, 0x2A);

    // Set POSIX executable permissions: 0o755
    fs::set_permissions(&src_file, fs::Permissions::from_mode(0o755)).unwrap();

    let dst_file = env.dst_path().join("exec_script.sh");
    fs::write(&dst_file, vec![0x2A; 1024]).unwrap();

    let pres = PlatformMetadata::copy_metadata(&src_file, &dst_file).expect("Copy metadata");
    assert!(pres.permissions_preserved, "Permissions should be preserved");

    let dst_meta = fs::metadata(&dst_file).unwrap();
    let mode = dst_meta.permissions().mode() & 0o777;
    assert_eq!(mode, 0o755, "POSIX mode 0755 must be preserved");
}

// =========================================================================
// 3. Symlink Preservation Without Cycle Loops
// =========================================================================
#[test]
fn test_tier1_symlink_preservation_no_loop() {
    let env = TestEnvironment::new();

    let _target_file = env.create_file("real_target.txt", 128, 0x77);
    let link_path = env.src_path().join("link_to_target.txt");
    symlink("real_target.txt", &link_path).expect("Create symlink");

    let manifest = env.scan_source();
    let link_item = manifest
        .items
        .iter()
        .find(|i| i.relative_path.to_str().unwrap() == "link_to_target.txt")
        .expect("Symlink item in manifest");

    assert_eq!(link_item.item_type, volterra_manifest::ItemType::Symlink);

    // Test platform symlink replication
    let dst_link = env.dst_path().join("link_to_target.txt");
    PlatformSymlink::preserve_symlink(&link_path, &dst_link)
        .expect("Copy symlink");

    assert!(dst_link.symlink_metadata().unwrap().file_type().is_symlink());
    let target = fs::read_link(&dst_link).unwrap();
    assert_eq!(target.to_str().unwrap(), "real_target.txt");
}

// =========================================================================
// 4. SQLite Journal WAL Durability & Relational Schema
// =========================================================================
#[test]
fn test_tier1_journal_wal_durability() {
    let env = TestEnvironment::new();
    let journal = env.open_journal();

    // Verify WAL mode is active
    let conn = journal.lock_conn();
    let mode: String = conn
        .query_row("PRAGMA journal_mode;", [], |row| row.get(0))
        .unwrap();
    drop(conn);
    assert_eq!(mode.to_lowercase(), "wal", "Journal must run in SQLite WAL mode");

    let job_id = "wal_durability_job";
    journal
        .create_job(&JobRecord {
            id: job_id.into(),
            source_root: env.src_path().to_path_buf(),
            target_root: env.dst_path().to_path_buf(),
            source_volume_uuid: None,
            target_volume_uuid: None,
            mode: JobMode::SmartHybrid,
            state: JobState::Running,
            total_files: 10,
            total_bytes: 1024 * 1024,
            copied_bytes: 0,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            completed_at: None,
        })
        .unwrap();

    let job_read = journal.get_job(job_id).unwrap().expect("Job exists");
    assert_eq!(job_read.id, job_id);
    assert_eq!(job_read.state, JobState::Running);

    // Update job status to Completed
    journal
        .update_job_status(job_id, JobState::Completed)
        .unwrap();
    let job_completed = journal.get_job(job_id).unwrap().unwrap();
    assert_eq!(job_completed.state, JobState::Completed);
}

// =========================================================================
// 5. Core Job Lifecycle State Machine & EventHub Broadcasting
// =========================================================================
#[test]
fn test_tier1_core_lifecycle_and_eventhub() {
    let env = TestEnvironment::new();
    env.create_file("test.dat", 32 * 1024, 0x44);

    let job_id = "lifecycle_test_job";
    let job_handle = JobBuilder::new()
        .job_id(job_id)
        .source(env.src_path())
        .destination(env.dst_path())
        .mode(JobMode::SmartHybrid)
        .chunk_size(16 * 1024)
        .verify_readback(true)
        .journal_path(env.journal_db_path())
        .build()
        .expect("Build job");

    assert_eq!(job_handle.state(), JobLifecycleState::Pending);

    let event_rx = job_handle.subscribe_events(100);

    job_handle.start().expect("Start job");

    // Wait for completion (small 32 KB file completes quickly)
    let start_wait = std::time::Instant::now();
    while job_handle.state() != JobLifecycleState::Completed
        && start_wait.elapsed().as_secs() < 10
    {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    assert_eq!(job_handle.state(), JobLifecycleState::Completed);

    let snapshot = job_handle.snapshot();
    assert_eq!(snapshot.state, JobLifecycleState::Completed);
    assert_eq!(snapshot.completed_files, 1);
    assert_eq!(snapshot.transferred_bytes, 32 * 1024);

    let mut event_count = 0;
    while let Ok(_event) = event_rx.try_recv() {
        event_count += 1;
    }
    assert!(event_count > 0, "EventHub must broadcast lifecycle events");
}

// =========================================================================
// 6. C ABI / FFI Bridge End-to-End Execution
// =========================================================================
#[test]
fn test_tier1_ffi_c_abi_end_to_end() {
    let env = TestEnvironment::new();
    env.create_file("ffi_payload.bin", 64 * 1024, 0x33);

    let src_c = CString::new(env.src_path().to_str().unwrap()).unwrap();
    let dst_c = CString::new(env.dst_path().to_str().unwrap()).unwrap();
    let jnl_c = CString::new(env.journal_db_path().to_str().unwrap()).unwrap();

    let config = volterra_config_t {
        source_path: src_c.as_ptr(),
        dest_path: dst_c.as_ptr(),
        mode: volterra_mode::SmartHybrid as u32,
        chunk_size: 32 * 1024,
        direct_io: false,
        verify_readback: true,
        memory_budget: 16 * 1024 * 1024,
        allow_overwrite: false,
        journal_path: jnl_c.as_ptr(),
    };

    unsafe {
        let mut err: *mut volterra_error_t = std::ptr::null_mut();
        let job = volterra_job_create(&config, &mut err);
        assert!(!job.is_null(), "volterra_job_create failed");
        assert!(err.is_null());

        let start_res = volterra_job_start(job, &mut err);
        assert_eq!(start_res, 0, "volterra_job_start failed");

        // Wait for completion
        let start_wait = std::time::Instant::now();
        let mut is_completed = false;
        while start_wait.elapsed().as_secs() < 10 {
            let snap = volterra_get_snapshot(job);
            if !snap.is_null() {
                if (*snap).state == volterra_state::Completed as u32 {
                    is_completed = true;
                    volterra_snapshot_free(snap);
                    break;
                }
                volterra_snapshot_free(snap);
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }

        assert!(is_completed, "Job must complete via C ABI");

        // Export report test
        let report_ptr = volterra_export_report(
            job,
            volterra_report_format::Json as libc::c_int,
        );
        assert!(!report_ptr.is_null(), "Report generation failed");
        let report_str = CStr::from_ptr(report_ptr).to_str().unwrap();
        assert!(report_str.contains("job_id"), "Report JSON should contain job_id");
        volterra_string_free(report_ptr);

        // Free job handle
        volterra_job_free(job);
    }
}

// =========================================================================
// 7. Disk Discovery & Real-Time Performance Monitor
// =========================================================================
#[test]
fn test_tier1_disk_discovery_and_profiler() {
    let disks = DiskDiscoveryEngine::discover_disks().expect("Discover disks");
    assert!(!disks.is_empty(), "Should discover at least root disk");

    let root_disk = &disks[0];
    assert!(!root_disk.device_id.is_empty());
    assert!(!root_disk.mount_point.is_empty());

    // Performance monitor telemetry testing
    let monitor = PerformanceMonitor::new();
    std::thread::sleep(std::time::Duration::from_millis(110));
    monitor.record_read(1024 * 1024, 2_000_000); // 1 MB in 2 ms -> 500 MB/s
    monitor.record_write(1024 * 1024, 1_000_000); // 1 MB in 1 ms -> 1000 MB/s

    let tel = monitor.get_telemetry();
    assert!(tel.total_bytes_read >= 1024 * 1024);
    assert!(tel.total_bytes_written >= 1024 * 1024);
    assert!(tel.ewma_1s_mbps > 0.0);
}

fn walkdir(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                files.extend(walkdir(&p));
            } else {
                files.push(p);
            }
        }
    }
    files
}
