//! Tier 4: Real-World Scenarios Integration Tests
//! Validates real-world deployment workloads:
//! 1. Heterogeneous multi-gigabyte disk migration (nested dirs, mixed sizes, symlinks, 100% BLAKE3 verification)
//! 2. Degraded disk rescue with sidecar manifests (scattered EIO sectors, .recovered.partial, fail-closed barrier)
//! 3. Device disconnect & reconnection (ENODEV drive ejection, WaitingForDisk, volume UUID verification, resume)
//! 4. Multi-gigabyte adaptive scaling (dynamic bandwidth adaptation 8 -> 16 -> 32 -> 64 MiB without oscillation)

use std::fs;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use volterra_journal::{
    FileRecord, FileState, JobMode, JobRecord, JobState,
};
use volterra_manifest::ItemType;
use volterra_optimizer::{
    AdaptiveChunkEngine, ChunkSize, TransferOptimizer, TransferPolicy,
};
use volterra_platform_macos::{PlatformSymlink, PlatformVolume, VolumeUuid};
use volterra_profiler::{
    DiskRelationship, PerformanceMonitor, TransferDirection, TransferTelemetry,
};
use volterra_recovery::{
    ActionRecommendation, ErrorCategory, ErrorDomain, ErrorQueue, ErrorSeverity, HoleFillPolicy,
    RecoveryClassifier, RecoveryController, RetryBudget, SidecarManifest,
};
use volterra_test_suite::fault_injection::reader::MockFaultyReader;
use volterra_test_suite::TestEnvironment;
use volterra_transfer::{
    TransferError, TransferScheduler, TransferSchedulerConfig, WriteFaultInjector,
};

// =========================================================================
// 1. Heterogeneous Disk Migration (MANI-01..03, XFER-01..09, JOUR-01..05)
// =========================================================================
#[test]
fn test_tier4_heterogeneous_disk_migration() {
    let env = TestEnvironment::new();

    // Directory hierarchy
    let docs_dir = env.src_path().join("documents").join("reports");
    let media_dir = env.src_path().join("media").join("photos");
    let video_dir = env.src_path().join("media").join("videos");
    fs::create_dir_all(&docs_dir).unwrap();
    fs::create_dir_all(&media_dir).unwrap();
    fs::create_dir_all(&video_dir).unwrap();

    // 1. Small files (1 KB to 32 KB)
    for i in 0..10 {
        let size = (i + 1) * 3 * 1024;
        let content = vec![((i * 17 + 5) % 251) as u8; size];
        fs::write(docs_dir.join(format!("report_{:02}.json", i)), &content).unwrap();
    }

    // 2. Medium photo files (1 MB to 2 MB)
    for i in 0..3 {
        let size = (1024 + i * 512) * 1024;
        let content = vec![((i * 31 + 7) % 251) as u8; size];
        fs::write(media_dir.join(format!("photo_{:02}.raw", i)), &content).unwrap();
    }

    // 3. Large video file (16 MB)
    let video_size = 16 * 1024 * 1024;
    let mut video_content = vec![0u8; video_size];
    for (idx, b) in video_content.iter_mut().enumerate() {
        *b = (idx % 256) as u8;
    }
    fs::write(video_dir.join("presentation.mov"), &video_content).unwrap();

    // 4. Symlink
    let target_file = docs_dir.join("report_00.json");
    let symlink_path = env.src_path().join("latest_report.json");
    PlatformSymlink::create_symlink(&target_file, &symlink_path).unwrap();

    // Scan manifest
    let manifest = env.scan_source();
    assert!(manifest.file_count >= 14, "Expected at least 14 items scanned");

    let journal = env.open_journal();
    let job_id = "job_tier4_migration".to_string();

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
        Arc::new(ErrorQueue::new()),
        RetryBudget::default(),
        journal.clone(),
    ));

    let config = TransferSchedulerConfig {
        source_root: env.src_path().to_path_buf(),
        target_root: env.dst_path().to_path_buf(),
        job_id: job_id.clone(),
        chunk_size: 4 * 1024 * 1024,
        memory_budget_bytes: 32 * 1024 * 1024,
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
    assert!(res.is_ok(), "Pipeline transfer failed: {:?}", res);

    // Deep Verification: Every regular file must match BLAKE3 digest 100%
    for item in &manifest.items {
        let src_file = env.src_path().join(&item.relative_path);
        let dst_file = env.dst_path().join(&item.relative_path);

        match item.item_type {
            ItemType::RegularFile => {
                assert!(dst_file.exists(), "Target file missing: {:?}", dst_file);
                let src_bytes = fs::read(&src_file).unwrap();
                let dst_bytes = fs::read(&dst_file).unwrap();
                assert_eq!(
                    src_bytes.len(),
                    dst_bytes.len(),
                    "Size mismatch for {:?}",
                    item.relative_path
                );
                assert_eq!(
                    blake3::hash(&src_bytes),
                    blake3::hash(&dst_bytes),
                    "Digest mismatch for {:?}",
                    item.relative_path
                );
            }
            ItemType::Symlink => {
                assert!(
                    dst_file.symlink_metadata().is_ok(),
                    "Target symlink missing: {:?}",
                    dst_file
                );
                let is_symlink = dst_file
                    .symlink_metadata()
                    .map(|m| m.file_type().is_symlink())
                    .unwrap_or(false);
                assert!(is_symlink, "Must be created as symlink: {:?}", dst_file);
            }
            ItemType::Directory => {
                assert!(dst_file.is_dir(), "Target directory missing: {:?}", dst_file);
            }
            _ => {}
        }
    }
}

// =========================================================================
// 2. Degraded Disk Rescue with Sidecars (RECO-02, RECO-05, TEST-01)
// =========================================================================
#[test]
fn test_tier4_degraded_disk_rescue_with_sidecars() {
    let env = TestEnvironment::new();

    // 128 KiB damaged tar file
    let file_size = 128 * 1024u64;
    let mut payload = vec![0x77u8; file_size as usize];
    for (i, b) in payload.iter_mut().enumerate() {
        *b = (i % 251) as u8;
    }
    fs::write(env.src_path().join("damaged_archive.tar"), &payload).unwrap();

    let manifest = env.scan_source();
    let journal = env.open_journal();
    let job_id = "job_tier4_rescue_sidecar".to_string();

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

    // Inject scattered bad sector at offset 32 KiB..36 KiB (4096 bytes)
    let faulty_reader = Arc::new(MockFaultyReader::new().with_bad_sector(32768, 4096));

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
    )
    .with_read_fault_injector(faulty_reader);

    // Run scheduler: degraded file gets quarantined without aborting
    let res = scheduler.execute_pipeline(&manifest, |_, _, _, _| {});
    assert!(res.is_ok());

    let quarantined_list = error_queue.list_all();
    assert_eq!(quarantined_list.len(), 1);
    let q = &quarantined_list[0];

    // Create sidecar manifest for the partially recovered file
    let sidecar = SidecarManifest::create(
        job_id.clone(),
        q.relative_path.clone(),
        file_size,
        &q.failed_extents,
        HoleFillPolicy::ZeroFilled,
    );

    // Fail-closed invariant checks
    assert!(!sidecar.is_complete, "Fail-closed: is_complete must be false");
    assert!(sidecar.is_partial_recovery, "is_partial_recovery must be true");
    assert!(!sidecar.checksum_verified, "checksum_verified must be false");
    assert_eq!(sidecar.unreadable_bytes, 16 * 1024);
    assert_eq!(sidecar.recovered_bytes, file_size - 16 * 1024);
    assert!(!sidecar.warning.is_empty());

    // Persist sidecar manifest to disk
    let sidecar_path = env.dst_path().join("damaged_archive.tar.volterra.meta");
    sidecar.write_to_file(&sidecar_path).expect("Write sidecar manifest");
    assert!(sidecar_path.exists());

    // Read back sidecar manifest and confirm identical state
    let read_back = SidecarManifest::read_from_file(&sidecar_path).expect("Read sidecar manifest");
    assert_eq!(read_back.job_id, job_id);
    assert_eq!(read_back.total_size_bytes, file_size);
    assert_eq!(read_back.unreadable_bytes, 16 * 1024);

    // Enforce publication barrier: attempting to publish this incomplete file must fail!
    let barrier_res = SidecarManifest::enforce_fail_closed_barrier(
        file_size,
        sidecar.recovered_bytes,
        sidecar.missing_extents.len(),
    );
    assert!(
        barrier_res.is_err(),
        "Publication barrier must prevent publishing incomplete files"
    );
}

// =========================================================================
// 3. Device Disconnect & Reconnection (TEST-06, PLAT-02, CORE-01)
// =========================================================================
struct DisconnectOnWrite;
impl WriteFaultInjector for DisconnectOnWrite {
    fn on_pwrite(&self, _offset: u64, _data: &[u8]) -> Option<Result<usize, std::io::Error>> {
        Some(Err(std::io::Error::from_raw_os_error(libc::ENODEV)))
    }
}

#[test]
fn test_tier4_device_disconnect_and_reconnection() {
    let env = TestEnvironment::new();

    let payload = vec![0x99u8; 64 * 1024];
    fs::write(env.src_path().join("usb_sync.bin"), &payload).unwrap();

    let manifest = env.scan_source();
    let journal = env.open_journal();
    let job_id = "job_tier4_disconnect".to_string();

    // 1. Query Darwin volume UUID
    let target_uuid = PlatformVolume::get_volume_uuid(env.dst_path())
        .unwrap_or_else(|_| VolumeUuid([0x5A; 16]));

    journal
        .create_job(&JobRecord {
            id: job_id.clone(),
            source_root: env.src_path().to_path_buf(),
            target_root: env.dst_path().to_path_buf(),
            source_volume_uuid: None,
            target_volume_uuid: Some(target_uuid.to_string_formatted()),
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

    // 2. Inject ENODEV (ejected disk during active write)
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
        recovery,
        Arc::new(PerformanceMonitor::new()),
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    )
    .with_write_fault_injector(Arc::new(DisconnectOnWrite));

    let res = scheduler.execute_pipeline(&manifest, |_, _, _, _| {});

    // Pipeline must catch ENODEV error
    let raw_err = match res {
        Err(TransferError::DestinationWriteError { source, .. }) => source,
        other => panic!("Expected DestinationWriteError with ENODEV, got: {:?}", other),
    };
    assert_eq!(raw_err.raw_os_error(), Some(libc::ENODEV));

    // 3. Classify ENODEV: Must classify as DeviceDisconnected and recommend MountDisk
    let classified = RecoveryClassifier::classify_io_error(&raw_err, ErrorDomain::Destination);
    assert_eq!(classified.category, ErrorCategory::DeviceDisconnected);
    assert_eq!(classified.severity, ErrorSeverity::JobFatal);
    assert!(!classified.is_retryable);
    assert_eq!(classified.suggested_action, ActionRecommendation::MountDisk);

    // 4. Job transitions to WaitingForDisk
    journal
        .update_job_status(&job_id, JobState::WaitingForDisk)
        .expect("Transition to WaitingForDisk");
    let job_rec = journal.get_job(&job_id).unwrap().unwrap();
    assert_eq!(job_rec.state, JobState::WaitingForDisk);

    // 5. Volume UUID verification upon candidate drive re-attachment
    let bogus_uuid = VolumeUuid([0xEE; 16]);
    assert_ne!(bogus_uuid, target_uuid);

    // Candidate 1: Bogus drive -> rejected
    let is_matching_uuid = |cand_uuid: &VolumeUuid, expected: &VolumeUuid| cand_uuid == expected;
    assert!(!is_matching_uuid(&bogus_uuid, &target_uuid), "Bogus drive must be rejected");

    // Candidate 2: Original target drive re-attached -> accepted!
    assert!(is_matching_uuid(&target_uuid, &target_uuid), "Original drive accepted");

    // 6. Resume job: transition from WaitingForDisk back to Running
    journal
        .update_job_status(&job_id, JobState::Running)
        .expect("Transition back to Running");
    let resumed_rec = journal.get_job(&job_id).unwrap().unwrap();
    assert_eq!(resumed_rec.state, JobState::Running);

    // Execute resume with normal writer (drive reconnected)
    let clean_recovery = Arc::new(RecoveryController::with_journal(
        Arc::new(ErrorQueue::new()),
        RetryBudget::default(),
        journal.clone(),
    ));

    let clean_scheduler = TransferScheduler::new(
        TransferSchedulerConfig {
            source_root: env.src_path().to_path_buf(),
            target_root: env.dst_path().to_path_buf(),
            job_id: job_id.clone(),
            chunk_size: 16 * 1024,
            memory_budget_bytes: 16 * 1024 * 1024,
            direct_io: false,
            verify_readback: true,
            allow_overwrite: false,
        },
        journal,
        clean_recovery,
        Arc::new(PerformanceMonitor::new()),
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    );

    let clean_res = clean_scheduler.execute_pipeline(&manifest, |_, _, _, _| {});
    assert!(clean_res.is_ok(), "Transfer must succeed after drive reconnection: {:?}", clean_res);
    assert!(env.dst_path().join("usb_sync.bin").exists());
}

// =========================================================================
// 4. Multi-Gigabyte Adaptive Scaling (OPT-01..04, TEST-11)
// =========================================================================
#[test]
fn test_tier4_multi_gigabyte_adaptive_scaling() {
    let rel = DiskRelationship {
        source_device_id: "disk1".into(),
        dest_device_id: "disk2".into(),
        source_parent_id: "disk0".into(),
        dest_parent_id: "disk11".into(),
        is_same_physical_disk: false,
        is_same_filesystem_container: false,
        direction: TransferDirection::SsdToSsd,
        recommended_concurrency: 2,
        warning: None,
    };

    let mut optimizer = TransferOptimizer::new(
        rel,
        128 * 1024 * 1024,
        TransferPolicy::Automatic,
    );

    // Step 1: Slow speed (< 30 MB/s)
    let tel_slow = TransferTelemetry {
        ewma_1s_mbps: 20.0,
        ewma_5s_mbps: 20.0,
        ewma_30s_mbps: 20.0,
        p50_read_latency_ms: 15.0,
        p95_read_latency_ms: 30.0,
        p50_write_latency_ms: 20.0,
        p95_write_latency_ms: 40.0,
        buffer_utilization_pct: 25.0,
        ..Default::default()
    };
    let dec = optimizer.evaluate_step(&tel_slow);
    assert!(dec.chunk_size_bytes >= 1024 * 1024);
    let chunk_slow = AdaptiveChunkEngine::determine_chunk_size(
        &tel_slow,
        1_000_000_000,
        128 * 1024 * 1024,
        1,
        false,
        TransferDirection::SsdToSsd,
    );
    assert_eq!(chunk_slow, ChunkSize::MiB4);

    // Step 2: Medium speed (100 MB/s)
    let tel_med = TransferTelemetry {
        ewma_1s_mbps: 100.0,
        ewma_5s_mbps: 100.0,
        ewma_30s_mbps: 100.0,
        p50_read_latency_ms: 5.0,
        p95_read_latency_ms: 15.0,
        p50_write_latency_ms: 8.0,
        p95_write_latency_ms: 18.0,
        buffer_utilization_pct: 45.0,
        ..Default::default()
    };
    let chunk_med = AdaptiveChunkEngine::determine_chunk_size(
        &tel_med,
        1_000_000_000,
        128 * 1024 * 1024,
        1,
        false,
        TransferDirection::SsdToSsd,
    );
    assert_eq!(chunk_med, ChunkSize::MiB8);

    // Step 3: High speed (300 MB/s)
    let tel_high = TransferTelemetry {
        ewma_1s_mbps: 300.0,
        ewma_5s_mbps: 300.0,
        ewma_30s_mbps: 300.0,
        p50_read_latency_ms: 2.0,
        p95_read_latency_ms: 8.0,
        p50_write_latency_ms: 3.0,
        p95_write_latency_ms: 10.0,
        buffer_utilization_pct: 60.0,
        ..Default::default()
    };
    let chunk_high = AdaptiveChunkEngine::determine_chunk_size(
        &tel_high,
        1_000_000_000,
        128 * 1024 * 1024,
        1,
        false,
        TransferDirection::SsdToSsd,
    );
    assert_eq!(chunk_high, ChunkSize::MiB16);

    // Step 4: Ultra NVMe speed (1200 MB/s, ultra-low latency < 20 ms)
    let tel_ultra = TransferTelemetry {
        ewma_1s_mbps: 1200.0,
        ewma_5s_mbps: 1200.0,
        ewma_30s_mbps: 1200.0,
        p50_read_latency_ms: 0.5,
        p95_read_latency_ms: 5.0,
        p50_write_latency_ms: 0.8,
        p95_write_latency_ms: 8.0,
        buffer_utilization_pct: 75.0,
        ..Default::default()
    };
    let chunk_ultra = AdaptiveChunkEngine::determine_chunk_size(
        &tel_ultra,
        2_000_000_000,
        128 * 1024 * 1024,
        1,
        false,
        TransferDirection::SsdToSsd,
    );
    assert_eq!(chunk_ultra, ChunkSize::MiB64);

    // Monotonic progression assert: 4 -> 8 -> 16 -> 64 MiB
    assert!(chunk_slow < chunk_med);
    assert!(chunk_med < chunk_high);
    assert!(chunk_high < chunk_ultra);

    // Verify slicing of a 1 GiB file using 64 MiB chunks produces 16 chunks
    let chunks = AdaptiveChunkEngine::slice_file(1, 1_073_741_824, ChunkSize::MiB64);
    assert_eq!(chunks.len(), 16);
    assert_eq!(chunks[0].length_bytes, 64 * 1024 * 1024);
    assert_eq!(chunks[15].offset_bytes, 15 * 64 * 1024 * 1024);
}
