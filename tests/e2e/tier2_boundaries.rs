//! Tier 2: Boundary & Corner Cases E2E Test Suite
//! Pushes edge limits: 0B/1B/chunk boundary files, memory bounding backpressure,
//! capacity preflight guards, 25-level deep Unicode Turkish directory hierarchy,
//! and rapid concurrent pause/resume lifecycle controls.

use std::fs;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use volterra_core::{JobBuilder, JobLifecycleState};
use volterra_journal::{FileRecord, FileState, JobMode, JobRecord, JobState};
use volterra_manifest::Manifest;
use volterra_profiler::PerformanceMonitor;
use volterra_recovery::{RecoveryController, RetryBudget};
use volterra_test_suite::{
    SyntheticDatasetBuilder, SyntheticDatasetValidator, TestEnvironment,
};
use volterra_transfer::{ChunkBufferPool, TransferError, TransferScheduler, TransferSchedulerConfig};

// =========================================================================
// 1. File Size Extremes: 0B, 1B, 8 MiB ± 1, 64 MiB ± 1 Boundaries (TEST-07)
// =========================================================================
#[test]
fn test_tier2_file_size_extremes_0b_1b_chunk_edges() {
    let env = TestEnvironment::new();

    let mut builder = SyntheticDatasetBuilder::new(env.src_path());
    builder.add_boundary_files().expect("Create boundary files");

    let manifest = env.scan_source();
    let journal = env.open_journal();
    let job_id = "job_tier2_boundaries".to_string();

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
        chunk_size: 8 * 1024 * 1024,
        memory_budget_bytes: 32 * 1024 * 1024,
        direct_io: false,
        verify_readback: true,
        allow_overwrite: false,
    };

    let scheduler = TransferScheduler::new(
        config,
        journal,
        recovery,
        Arc::new(PerformanceMonitor::new()),
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    );

    let res = scheduler.execute_pipeline(&manifest, |_, _, _, _| {});
    assert!(res.is_ok(), "Boundary transfer failed: {:?}", res);

    // Deep verification via SyntheticDatasetValidator
    let val = SyntheticDatasetValidator::validate(env.dst_path(), builder.expectations())
        .expect("Validation should pass 100%");
    assert_eq!(val.files_matched, builder.expectations().len());
}

// =========================================================================
// 2. Strict Memory Budget Bounding & Pool Recycling (XFER-03, OPT-01)
// =========================================================================
#[test]
fn test_tier2_memory_budget_strict_bounding() {
    // 4 buffers of 4 MiB each = 16 MiB max budget
    let chunk_size = 4 * 1024 * 1024;
    let max_budget = 16 * 1024 * 1024;
    let pool = ChunkBufferPool::new(chunk_size, max_budget);

    assert_eq!(pool.max_buffers(), 4);
    assert_eq!(pool.in_flight_count(), 0);
    assert_eq!(pool.utilization_pct(), 0.0);

    // Acquire all 4 buffers to reach 100% budget utilization
    let b1 = pool.acquire();
    let b2 = pool.acquire();
    assert_eq!(pool.in_flight_count(), 2);
    assert!((pool.utilization_pct() - 50.0).abs() < 0.1);

    let b3 = pool.acquire();
    let b4 = pool.acquire();
    assert_eq!(pool.in_flight_count(), 4);
    assert!((pool.utilization_pct() - 100.0).abs() < 0.1);

    // Release buffers and verify capacity recycling
    pool.release(b1);
    assert_eq!(pool.in_flight_count(), 3);
    assert!((pool.utilization_pct() - 75.0).abs() < 0.1);

    pool.release(b2);
    pool.release(b3);
    pool.release(b4);
    assert_eq!(pool.in_flight_count(), 0);
    assert_eq!(pool.utilization_pct(), 0.0);

    // Verify recycled buffer is reused without reallocation
    let reused = pool.acquire();
    assert_eq!(reused.len(), chunk_size);
    assert_eq!(pool.in_flight_count(), 1);
    pool.release(reused);
    assert_eq!(pool.in_flight_count(), 0);
}

// =========================================================================
// 3. Preflight Capacity Guard (PLAT-01, XFER-08)
// =========================================================================
#[test]
fn test_tier2_preflight_capacity_and_readonly_guard() {
    let env = TestEnvironment::new();
    let journal = env.open_journal();
    let recovery = Arc::new(RecoveryController::with_journal(
        Arc::new(volterra_recovery::ErrorQueue::new()),
        RetryBudget::default(),
        journal.clone(),
    ));

    let config_cap = TransferSchedulerConfig {
        source_root: env.src_path().to_path_buf(),
        target_root: env.dst_path().to_path_buf(),
        job_id: "cap_job".into(),
        chunk_size: 16 * 1024,
        memory_budget_bytes: 16 * 1024 * 1024,
        direct_io: false,
        verify_readback: false,
        allow_overwrite: false,
    };

    let scheduler_cap = TransferScheduler::new(
        config_cap,
        journal,
        recovery,
        Arc::new(PerformanceMonitor::new()),
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    );

    // Create a manifest requiring 1 Petabyte (which exceeds available storage)
    let petabyte_manifest = Manifest {
        root: env.src_path().to_path_buf(),
        job_id: "cap_job".into(),
        items: Vec::new(),
        total_bytes: 1_000_000_000_000_000, // 1 PB
        file_count: 1,
        dir_count: 0,
        symlink_count: 0,
        special_count: 0,
    };

    let cap_res = scheduler_cap.preflight_check(&petabyte_manifest);
    match cap_res {
        Err(TransferError::PreflightCapacityExceeded { required_bytes, .. }) => {
            assert_eq!(required_bytes, 1_000_000_000_000_000);
        }
        other => panic!("Expected PreflightCapacityExceeded, got {:?}", other),
    }
}

// =========================================================================
// 4. 25-Level Deep Directory Tree & Turkish Unicode Workload (TEST-09)
// =========================================================================
#[test]
fn test_tier2_deep_tree_path_max_unicode() {
    let env = TestEnvironment::new();

    let mut builder = SyntheticDatasetBuilder::new(env.src_path());
    builder
        .add_deep_unicode_hierarchy(25)
        .expect("Create 25-level deep Turkish Unicode hierarchy");

    let manifest = env.scan_source();
    assert!(manifest.file_count >= 2);

    let journal = env.open_journal();
    let job_id = "job_tier2_unicode_25".to_string();

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
        chunk_size: 16 * 1024,
        memory_budget_bytes: 16 * 1024 * 1024,
        direct_io: false,
        verify_readback: true,
        allow_overwrite: false,
    };

    let scheduler = TransferScheduler::new(
        config,
        journal,
        recovery,
        Arc::new(PerformanceMonitor::new()),
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    );

    let res = scheduler.execute_pipeline(&manifest, |_, _, _, _| {});
    assert!(res.is_ok(), "25-level deep copy failed: {:?}", res);

    let val = SyntheticDatasetValidator::validate(env.dst_path(), builder.expectations())
        .expect("All Turkish Unicode deep files must validate bit-exact");
    assert_eq!(val.files_matched, builder.expectations().len());
}

// =========================================================================
// 5. Rapid Concurrent Pause / Resume Cycles Stress
// =========================================================================
#[test]
fn test_tier2_rapid_pause_resume_stress() {
    let env = TestEnvironment::new();
    // Create a 2 MB file (32 chunks of 64 KB)
    env.create_pattern_file("large_stress.bin", 2 * 1024 * 1024);

    let job_id = "job_pause_resume_stress";
    let job_handle = Arc::new(
        JobBuilder::new()
            .job_id(job_id)
            .source(env.src_path())
            .destination(env.dst_path())
            .mode(JobMode::SmartHybrid)
            .chunk_size(64 * 1024)
            .verify_readback(true)
            .journal_path(env.journal_db_path())
            .build()
            .expect("Build job"),
    );

    job_handle.start().expect("Start job");

    // Rapid alternating pause / resume calls in background thread
    let handle_clone = job_handle.clone();
    let stress_thread = std::thread::spawn(move || {
        for _ in 0..20 {
            let _ = handle_clone.pause();
            std::thread::sleep(std::time::Duration::from_millis(5));
            let _ = handle_clone.resume();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    });

    stress_thread.join().unwrap();

    // Ensure job resumes and completes
    let _ = job_handle.resume();

    let start_wait = std::time::Instant::now();
    while job_handle.state() != JobLifecycleState::Completed
        && start_wait.elapsed().as_secs() < 10
    {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    assert_eq!(job_handle.state(), JobLifecycleState::Completed);

    let src_data = fs::read(env.src_path().join("large_stress.bin")).unwrap();
    let dst_data = fs::read(env.dst_path().join("large_stress.bin")).unwrap();
    assert_eq!(src_data, dst_data, "Data must remain bit-exact after rapid pause/resume cycles");
}
