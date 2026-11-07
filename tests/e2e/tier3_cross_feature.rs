//! Tier 3: Cross-Feature Integration Tests
//! Verifies cross-cutting interactions across subsystems:
//! 1. Error quarantine isolation & C ABI remediation actions
//! 2. Optimizer closed-loop fault reaction & bottleneck classification
//! 3. Mid-flight simulated crash, WAL reconciliation, and verified chunk reuse
//! 4. Destination file collision resolution (rejection vs. atomic replacement)
//! 5. Pre-publish readback integrity corruption barrier (preventing corrupt file publish)

use std::ffi::CString;
use std::fs;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use volterra_ffi::*;
use volterra_journal::{
    ChunkRecord, ChunkState, FileRecord, FileState, JobMode, JobRecord, JobState,
};
use volterra_optimizer::{
    BottleneckType, ControllerPhase, TransferOptimizer, TransferPolicy,
};
use volterra_platform_macos::PlatformAtomic;
use volterra_profiler::{DiskRelationship, PerformanceMonitor, TransferTelemetry};
use volterra_recovery::{ErrorQueue, RecoveryController, RetryBudget};
use volterra_test_suite::fault_injection::reader::MockFaultyReader;
use volterra_test_suite::TestEnvironment;
use volterra_transfer::{
    TransferError, TransferScheduler, TransferSchedulerConfig, WriteFaultInjector,
};

// =========================================================================
// 1. Error Quarantine Isolation & C ABI Remediation (RECO-01..04, FFI-09..14)
// =========================================================================
#[test]
fn test_tier3_error_quarantine_and_ffi_action() {
    let env = TestEnvironment::new();

    // Create 3 files: 2 healthy (32 KiB each), 1 faulty (128 KiB)
    let f1_data = vec![0xAA; 32 * 1024];
    let f2_data = vec![0xBB; 128 * 1024];
    let f3_data = vec![0xCC; 32 * 1024];

    fs::write(env.src_path().join("f1_healthy.bin"), &f1_data).unwrap();
    fs::write(env.src_path().join("f2_faulty.bin"), &f2_data).unwrap();
    fs::write(env.src_path().join("f3_healthy.bin"), &f3_data).unwrap();

    let manifest = env.scan_source();
    let journal = env.open_journal();
    let job_id = "job_tier3_quarantine".to_string();

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

    let error_queue = Arc::new(ErrorQueue::new());
    let recovery = Arc::new(RecoveryController::with_journal(
        error_queue.clone(),
        RetryBudget::default(),
        journal.clone(),
    ));

    // Inject bad sector at offset 64 KiB..68 KiB on f2_faulty.bin
    // f1 and f3 are only 32 KiB, so they never touch offset 64 KiB.
    let faulty_reader = Arc::new(MockFaultyReader::new().with_bad_sector(65536, 4096));

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
        journal.clone(),
        recovery.clone(),
        Arc::new(PerformanceMonitor::new()),
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    )
    .with_read_fault_injector(faulty_reader);

    // Execute pipeline: error quarantine must isolate the faulty file while healthy files finish
    let res = scheduler.execute_pipeline(&manifest, |_, _, _, _| {});
    assert!(res.is_ok(), "Healthy files must finish without whole job abort: {:?}", res);

    // Assert healthy files completed and published
    assert!(env.dst_path().join("f1_healthy.bin").exists());
    assert!(env.dst_path().join("f3_healthy.bin").exists());

    // Assert faulty file was quarantined and not published to final destination
    assert!(!env.dst_path().join("f2_faulty.bin").exists());

    // Assert error queue contains quarantined file with exact failed extent
    let quarantined = error_queue.list_all();
    assert_eq!(quarantined.len(), 1);
    let q = &quarantined[0];
    assert_eq!(q.relative_path, Path::new("f2_faulty.bin"));
    assert!(q.failed_extents.intervals().iter().any(|ext| ext.offset == 65536 && ext.length == 16 * 1024));

    // =====================================================================
    // C ABI Integration Verification
    // =====================================================================
    unsafe {
        let mut out_err: *mut volterra_error_t = std::ptr::null_mut();
        let src_c = CString::new(env.src_path().to_str().unwrap()).unwrap();
        let dst_c = CString::new(env.dst_path().to_str().unwrap()).unwrap();
        let jnl_c = CString::new(env.journal_db_path().to_str().unwrap()).unwrap();

        let job_config = volterra_config_t {
            source_path: src_c.as_ptr(),
            dest_path: dst_c.as_ptr(),
            mode: volterra_mode::SmartHybrid as u32,
            chunk_size: 16 * 1024,
            direct_io: false,
            verify_readback: true,
            memory_budget: 16 * 1024 * 1024,
            allow_overwrite: false,
            journal_path: jnl_c.as_ptr(),
        };

        let job = volterra_job_create(&job_config, &mut out_err);
        assert!(!job.is_null());
        assert!(out_err.is_null());

        // Inject quarantined file into job's recovery controller
        let _ = (*job).handle.recovery().queue().quarantine_file(q.clone());

        // Retrieve errors via C ABI
        let err_list = volterra_get_errors(job);
        assert!(!err_list.is_null());
        assert_eq!((*err_list).count, 1);
        let item = &*(*err_list).items;
        assert_eq!(item.file_id, q.file_id);
        assert_eq!(item.failed_extents_count, 1);

        // Test C ABI volterra_skip_file
        let skip_ret = volterra_skip_file(job, q.file_id);
        assert_eq!(skip_ret, volterra_error_code::Ok as libc::c_int);

        // Test C ABI volterra_retry_file
        let retry_ret = volterra_retry_file(job, q.file_id);
        assert_eq!(retry_ret, volterra_error_code::Ok as libc::c_int);

        // Free C ABI allocations
        volterra_error_list_free(err_list);
        volterra_job_free(job);
    }
}

// =========================================================================
// 2. Optimizer Closed-Loop Fault Reaction & Bottleneck (OPT-01, OPT-05)
// =========================================================================
#[test]
fn test_tier3_optimizer_fault_reaction_and_bottleneck() {
    let rel = DiskRelationship {
        source_device_id: "disk1".into(),
        dest_device_id: "disk2".into(),
        source_parent_id: "disk0".into(),
        dest_parent_id: "disk11".into(),
        is_same_physical_disk: false,
        is_same_filesystem_container: false,
        direction: volterra_profiler::TransferDirection::SsdToSsd,
        recommended_concurrency: 2,
        warning: None,
    };

    let mut optimizer = TransferOptimizer::new(
        rel,
        64 * 1024 * 1024,
        TransferPolicy::Automatic,
    );

    // Initial nominal telemetry during warmup
    let mut telemetry = TransferTelemetry {
        ewma_1s_mbps: 250.0,
        ewma_5s_mbps: 250.0,
        ewma_30s_mbps: 250.0,
        p50_read_latency_ms: 2.0,
        p95_read_latency_ms: 10.0,
        p50_write_latency_ms: 3.0,
        p95_write_latency_ms: 12.0,
        buffer_utilization_pct: 40.0,
        read_errors: 0,
        write_errors: 0,
        ..Default::default()
    };

    // Warmup cycles
    for _ in 0..5 {
        let dec = optimizer.evaluate_step(&telemetry);
        assert!(dec.chunk_size_bytes >= 8 * 1024 * 1024);
    }

    // Bottleneck detection under nominal conditions
    let nominal_bn = optimizer.detect_bottleneck(&telemetry);
    assert_eq!(nominal_bn.bottleneck_type, BottleneckType::Nominal);

    // Drive fault event: simulate unreadable bad sectors on drive
    telemetry.read_errors = 3;
    telemetry.write_errors = 1;

    // Bottleneck detector must prioritize ErrorDegradation as Critical
    let fault_bn = optimizer.detect_bottleneck(&telemetry);
    assert_eq!(fault_bn.bottleneck_type, BottleneckType::ErrorDegradation);
    assert_eq!(fault_bn.severity, volterra_optimizer::BottleneckSeverity::Critical);
    assert!(fault_bn.technical_reason.contains("Unreadable sectors"));

    // Optimizer step must execute immediate safe error fallback
    let fault_decision = optimizer.evaluate_step(&telemetry);
    assert_eq!(fault_decision.chunk_size_bytes, 1024 * 1024, "Must clamp to 1 MiB chunk");
    assert_eq!(fault_decision.concurrency, 1, "Must reduce concurrency to 1");
    assert_eq!(fault_decision.queue_depth, 1, "Must reduce queue depth to 1");
    assert_eq!(fault_decision.read_ahead_mb, 0, "Must disable readahead");
    assert_eq!(fault_decision.phase, ControllerPhase::Settled);
    assert!(fault_decision.action_description.contains("Error detected"));
}

// =========================================================================
// 3. Simulated SIGKILL / WAL Reconciliation & Resume (JOUR-04, TEST-05)
// =========================================================================
#[test]
fn test_tier3_simulated_sigkill_wal_reconciliation() {
    let env = TestEnvironment::new();

    // 256 KiB file (4 chunks of 64 KiB)
    let payload = vec![0x55; 256 * 1024];
    fs::write(env.src_path().join("resume_target.dat"), &payload).unwrap();

    let manifest = env.scan_source();
    let journal = env.open_journal();
    let job_id = "job_tier3_sigkill_resume".to_string();

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

    let target_file = env.dst_path().join("resume_target.dat");
    let partial_path = PlatformAtomic::make_partial_path(&target_file, &job_id);
    let rel_partial = partial_path.strip_prefix(env.dst_path()).unwrap().to_path_buf();

    let item = &manifest.items[0];
    let file_rec = FileRecord {
        id: item.id,
        job_id: job_id.clone(),
        relative_path: item.relative_path.clone(),
        source_size: item.size_bytes,
        source_mtime_ns: item.source_mtime_ns,
        source_btime_ns: item.source_btime_ns,
        posix_mode: item.posix_mode,
        source_file_id: item.source_file_id.clone(),
        temp_path: Some(rel_partial),
        state: FileState::Copying,
        verified_digest: None,
        error_code: None,
        recovered_bytes: 128 * 1024,
    };
    journal.record_files_batch(&job_id, &[file_rec]).unwrap();

    // Simulate partial transfer before crash: write first 2 chunks (128 KiB) to .partial
    if let Some(parent) = partial_path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&partial_path, &payload[0..128 * 1024]).unwrap();

    let c0_hash = blake3::hash(&payload[0..64 * 1024]).to_hex().to_string();
    let c1_hash = blake3::hash(&payload[64 * 1024..128 * 1024]).to_hex().to_string();

    journal
        .record_chunks_batch(&[
            ChunkRecord {
                file_id: item.id,
                chunk_index: 0,
                offset_bytes: 0,
                length_bytes: 64 * 1024,
                digest: Some(c0_hash),
                state: ChunkState::Verified,
                attempts: 1,
            },
            ChunkRecord {
                file_id: item.id,
                chunk_index: 1,
                offset_bytes: 64 * 1024,
                length_bytes: 64 * 1024,
                digest: Some(c1_hash),
                state: ChunkState::Verified,
                attempts: 1,
            },
        ])
        .unwrap();

    // Reconcile WAL after crash
    let recon_summary = journal
        .reconcile_job_after_crash(&job_id, env.dst_path(), true)
        .expect("WAL reconciliation must succeed");
    assert_eq!(recon_summary.reconciled_files_count, 1);

    // Resume transfer with bad sectors injected on chunks 0 and 1
    // If the pipeline attempts to re-read chunks 0 or 1 from source, it will fail!
    let faulty_reader = Arc::new(MockFaultyReader::new().with_bad_sector(0, 128 * 1024));

    let recovery = Arc::new(RecoveryController::with_journal(
        Arc::new(ErrorQueue::new()),
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
        journal,
        recovery,
        Arc::new(PerformanceMonitor::new()),
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    )
    .with_read_fault_injector(faulty_reader);

    let res = scheduler.execute_pipeline(&manifest, |_, _, _, _| {});
    assert!(res.is_ok(), "Resume must succeed by skipping verified chunks: {:?}", res);

    // Verify destination file exists and matches source exactly
    assert!(target_file.exists());
    let dest_data = fs::read(&target_file).unwrap();
    assert_eq!(dest_data.len(), payload.len());
    assert_eq!(blake3::hash(&dest_data), blake3::hash(&payload));
}

// =========================================================================
// 4. Destination Collision Resolution (XFER-09, PLAT-06)
// =========================================================================
#[test]
fn test_tier3_destination_collision_resolution() {
    let env = TestEnvironment::new();

    let new_content = b"NEW SOURCE PAYLOAD CONTENT";
    let old_content = b"EXISTING DESTINATION PRESERVED CONTENT";

    fs::write(env.src_path().join("collision.txt"), new_content).unwrap();
    // Pre-create conflicting file at target root
    fs::write(env.dst_path().join("collision.txt"), old_content).unwrap();

    let manifest = env.scan_source();
    let journal = env.open_journal();
    let job_id = "job_tier3_collision".to_string();

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

    let recovery = Arc::new(RecoveryController::with_journal(
        Arc::new(ErrorQueue::new()),
        RetryBudget::default(),
        journal.clone(),
    ));

    // Phase 1: allow_overwrite = false -> must fail with DestinationCollision
    let config_no_overwrite = TransferSchedulerConfig {
        source_root: env.src_path().to_path_buf(),
        target_root: env.dst_path().to_path_buf(),
        job_id: job_id.clone(),
        chunk_size: 64 * 1024,
        memory_budget_bytes: 16 * 1024 * 1024,
        direct_io: false,
        verify_readback: true,
        allow_overwrite: false,
    };

    let scheduler_blocked = TransferScheduler::new(
        config_no_overwrite,
        journal.clone(),
        recovery.clone(),
        Arc::new(PerformanceMonitor::new()),
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    );

    let res_blocked = scheduler_blocked.execute_pipeline(&manifest, |_, _, _, _| {});
    match res_blocked {
        Err(TransferError::DestinationCollision(p)) => {
            assert_eq!(p, env.dst_path().join("collision.txt"));
        }
        other => panic!("Expected DestinationCollision error, got {:?}", other),
    }

    // Destination file content must NOT have been overwritten
    let current_content = fs::read(env.dst_path().join("collision.txt")).unwrap();
    assert_eq!(current_content, old_content);

    // Phase 2: allow_overwrite = true -> must atomically replace file
    let config_overwrite = TransferSchedulerConfig {
        source_root: env.src_path().to_path_buf(),
        target_root: env.dst_path().to_path_buf(),
        job_id: "job_tier3_collision_allowed".to_string(),
        chunk_size: 64 * 1024,
        memory_budget_bytes: 16 * 1024 * 1024,
        direct_io: false,
        verify_readback: true,
        allow_overwrite: true,
    };

    let scheduler_allowed = TransferScheduler::new(
        config_overwrite,
        journal,
        recovery,
        Arc::new(PerformanceMonitor::new()),
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    );

    let res_allowed = scheduler_allowed.execute_pipeline(&manifest, |_, _, _, _| {});
    assert!(res_allowed.is_ok(), "Overwrite allowed must succeed: {:?}", res_allowed);

    // Destination file must now atomically hold the new content
    let final_content = fs::read(env.dst_path().join("collision.txt")).unwrap();
    assert_eq!(final_content, new_content);
}

// =========================================================================
// 5. Readback Integrity Corruption Barrier (INTEG-03, INTEG-04, XFER-06)
// =========================================================================
struct CorruptingWriter {
    corrupt_offset: u64,
}

impl WriteFaultInjector for CorruptingWriter {
    fn transform_data(&self, offset: u64, data: &[u8]) -> Option<Vec<u8>> {
        // Corrupt 1 byte at the target offset
        if offset <= self.corrupt_offset && (self.corrupt_offset < offset + data.len() as u64) {
            let mut corrupted = data.to_vec();
            let idx = (self.corrupt_offset - offset) as usize;
            corrupted[idx] ^= 0xFF; // Flip all bits of the target byte
            Some(corrupted)
        } else {
            None
        }
    }
}

#[test]
fn test_tier3_readback_integrity_corruption_barrier() {
    let env = TestEnvironment::new();

    // 64 KiB file with distinct pseudo-random pattern
    let mut payload = vec![0u8; 64 * 1024];
    for (i, b) in payload.iter_mut().enumerate() {
        *b = ((i * 37 + 11) % 251) as u8;
    }
    fs::write(env.src_path().join("sensitive_record.db"), &payload).unwrap();

    let manifest = env.scan_source();
    let journal = env.open_journal();
    let job_id = "job_tier3_corruption_barrier".to_string();

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

    let recovery = Arc::new(RecoveryController::with_journal(
        Arc::new(ErrorQueue::new()),
        RetryBudget::default(),
        journal.clone(),
    ));

    // Inject 1 corrupted byte at offset 500 during write
    let corruptor = Arc::new(CorruptingWriter { corrupt_offset: 500 });

    let config = TransferSchedulerConfig {
        source_root: env.src_path().to_path_buf(),
        target_root: env.dst_path().to_path_buf(),
        job_id: job_id.clone(),
        chunk_size: 64 * 1024,
        memory_budget_bytes: 16 * 1024 * 1024,
        direct_io: false,
        verify_readback: true, // Strict readback digest validation active
        allow_overwrite: false,
    };

    let scheduler = TransferScheduler::new(
        config,
        journal,
        recovery,
        Arc::new(PerformanceMonitor::new()),
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    )
    .with_write_fault_injector(corruptor);

    // Execute transfer: readback verification must catch the corrupted byte and fail publication!
    let res = scheduler.execute_pipeline(&manifest, |_, _, _, _| {});

    match res {
        Err(TransferError::IntegrityFailed(e)) => {
            // Success: IntegrityFailed caught
            let err_str = e.to_string();
            assert!(
                err_str.contains("Checksum mismatch") || err_str.contains("mismatch"),
                "Expected checksum mismatch error, got: {}",
                err_str
            );
        }
        other => panic!("Expected IntegrityFailed from readback verification, got: {:?}", other),
    }

    // Crucial barrier assertion: The corrupted destination file must NEVER have been published!
    let target_dest = env.dst_path().join("sensitive_record.db");
    assert!(!target_dest.exists(), "Corrupted file must NEVER be published to final destination!");

    // The .partial file must remain on disk for diagnostics
    let partial_path = PlatformAtomic::make_partial_path(&target_dest, &job_id);
    assert!(partial_path.exists(), ".partial file must be preserved for analysis");
}
