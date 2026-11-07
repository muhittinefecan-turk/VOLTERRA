use std::fs::File;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tempfile::tempdir;
use volterra_journal::{FileRecord, FileState, JournalEngine};
use volterra_manifest::{ManifestScanner, ScanOptions};
use volterra_profiler::PerformanceMonitor;
use volterra_recovery::RecoveryController;
use volterra_transfer::{
    AtomicPublisher, ChunkBufferPool, ChunkReader, ChunkWriter, TransferError, TransferScheduler,
    TransferSchedulerConfig,
};

#[test]
fn test_chunk_reader_writer_posix_roundtrip() {
    let dir = tempdir().unwrap();
    let partial_path = dir.path().join(".volterra.job1.test.partial");

    let writer = ChunkWriter::open_or_create_partial(&partial_path, false).expect("Open writer");
    let payload = vec![0xABu8; 128 * 1024]; // 128 KiB
    writer.write_chunk(0, &payload).expect("Write chunk");
    writer.sync_all().expect("Sync write");

    let reader = ChunkReader::open(&partial_path, false).expect("Open reader");
    let mut buf = vec![0u8; 128 * 1024];
    let digest = reader
        .read_chunk(0, 128 * 1024, &mut buf)
        .expect("Read chunk");

    assert_eq!(buf, payload);
    let expected_digest = volterra_integrity::Blake3Hasher::hash_chunk(&payload);
    assert_eq!(digest, expected_digest);
}

#[test]
fn test_buffer_pool_concurrency_and_backpressure() {
    let pool = ChunkBufferPool::new(4 * 1024 * 1024, 16 * 1024 * 1024); // 4 buffers max
    assert_eq!(pool.max_buffers(), 4);
    assert_eq!(pool.in_flight_count(), 0);

    let b1 = pool.acquire();
    let b2 = pool.acquire();
    assert_eq!(pool.in_flight_count(), 2);
    assert!((pool.utilization_pct() - 50.0).abs() < 0.1);

    pool.release(b1);
    assert_eq!(pool.in_flight_count(), 1);
    pool.release(b2);
    assert_eq!(pool.in_flight_count(), 0);
}

#[test]
fn test_atomic_publisher_collision_prevention() {
    let dir = tempdir().unwrap();
    let src_file = dir.path().join("source.dat");
    std::fs::write(&src_file, b"original source data").unwrap();

    let dest_file = dir.path().join("existing_dest.dat");
    std::fs::write(&dest_file, b"existing destination data").unwrap();

    let temp_partial = dir.path().join(".volterra.job1.dest.partial");
    std::fs::write(&temp_partial, b"new partial data").unwrap();

    let manifest = ManifestScanner::scan_tree(dir.path(), &ScanOptions::default()).unwrap();
    let item = manifest
        .items
        .iter()
        .find(|i| i.relative_path.to_string_lossy() == "source.dat")
        .expect("Find source item");

    let file = File::open(&temp_partial).unwrap();

    // Publication with allow_overwrite = false must fail with collision!
    let res = AtomicPublisher::publish_file(
        &file,
        &temp_partial,
        &dest_file,
        dir.path(),
        item,
        20,
        None,
        false,
        false,
    );

    assert!(res.is_err());
    // Destination file must remain unchanged
    let dest_content = std::fs::read(&dest_file).unwrap();
    assert_eq!(dest_content, b"existing destination data");
}

#[test]
fn test_transfer_scheduler_pipeline_end_to_end() {
    let src_dir = tempdir().unwrap();
    let dst_dir = tempdir().unwrap();
    let journal_dir = tempdir().unwrap();

    // Create synthetic source tree
    std::fs::create_dir_all(src_dir.path().join("nested")).unwrap();
    std::fs::write(src_dir.path().join("file1.txt"), b"Hello Volterra World 1").unwrap();
    std::fs::write(
        src_dir.path().join("nested/file2.bin"),
        vec![0x42; 64 * 1024],
    )
    .unwrap();

    let manifest =
        ManifestScanner::scan_tree(src_dir.path(), &ScanOptions::default()).expect("Scan manifest");
    assert_eq!(manifest.items.len(), 3); // 2 files + 1 dir

    let db_path = journal_dir.path().join("journal.db");
    let journal = Arc::new(JournalEngine::open(&db_path).expect("Open journal"));
    journal
        .create_job(&volterra_journal::JobRecord {
            id: "job_test_001".into(),
            source_root: src_dir.path().to_path_buf(),
            target_root: dst_dir.path().to_path_buf(),
            source_volume_uuid: None,
            target_volume_uuid: None,
            mode: volterra_journal::JobMode::SmartHybrid,
            state: volterra_journal::JobState::Running,
            total_files: manifest.items.len() as u64,
            total_bytes: manifest.total_bytes,
            copied_bytes: 0,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            completed_at: None,
        })
        .unwrap();

    // Record files into journal
    let file_records: Vec<_> = manifest
        .items
        .iter()
        .map(|item| FileRecord {
            id: item.id,
            job_id: "job_test_001".into(),
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
    journal
        .record_files_batch("job_test_001", &file_records)
        .unwrap();

    let recovery = Arc::new(RecoveryController::with_journal(
        Arc::new(volterra_recovery::ErrorQueue::new()),
        volterra_recovery::RetryBudget::default(),
        journal.clone(),
    ));
    let monitor = Arc::new(PerformanceMonitor::new());
    let paused = Arc::new(AtomicBool::new(false));
    let cancelled = Arc::new(AtomicBool::new(false));

    let config = TransferSchedulerConfig {
        source_root: src_dir.path().to_path_buf(),
        target_root: dst_dir.path().to_path_buf(),
        job_id: "job_test_001".into(),
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
        monitor,
        paused,
        cancelled,
    );
    let mut progress_count = 0;
    scheduler
        .execute_pipeline(&manifest, |_file_id, _chunk_idx, _copied, _total| {
            progress_count += 1;
        })
        .expect("Pipeline execution");

    assert!(progress_count > 0);
    // Verify target files exist and content matches
    let f1_dst = dst_dir.path().join("file1.txt");
    let f2_dst = dst_dir.path().join("nested/file2.bin");
    assert!(f1_dst.exists());
    assert!(f2_dst.exists());
    assert_eq!(std::fs::read(f1_dst).unwrap(), b"Hello Volterra World 1");
    assert_eq!(std::fs::read(f2_dst).unwrap(), vec![0x42; 64 * 1024]);
}

#[test]
fn test_error_isolation_unreadable_file_continues_healthy_files() {
    let src_dir = tempdir().unwrap();
    let dst_dir = tempdir().unwrap();
    let journal_dir = tempdir().unwrap();

    // Create file 1 (unreadable: chmod 000) and file 2 (healthy)
    let f1 = src_dir.path().join("unreadable.txt");
    std::fs::write(&f1, b"Secret unreadable data").unwrap();

    let f2 = src_dir.path().join("healthy.txt");
    std::fs::write(&f2, b"Healthy readable data").unwrap();

    let manifest =
        ManifestScanner::scan_tree(src_dir.path(), &ScanOptions::default()).expect("Scan");

    // Make file 1 unreadable via POSIX chmod 000
    #[cfg(unix)]
    unsafe {
        let c_path = std::ffi::CString::new(f1.to_string_lossy().as_bytes()).unwrap();
        libc::chmod(c_path.as_ptr(), 0o000);
    }

    let db_path = journal_dir.path().join("journal.db");
    let journal = Arc::new(JournalEngine::open(&db_path).unwrap());
    journal
        .create_job(&volterra_journal::JobRecord {
            id: "job_err_iso".into(),
            source_root: src_dir.path().to_path_buf(),
            target_root: dst_dir.path().to_path_buf(),
            source_volume_uuid: None,
            target_volume_uuid: None,
            mode: volterra_journal::JobMode::SmartHybrid,
            state: volterra_journal::JobState::Running,
            total_files: manifest.items.len() as u64,
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
            job_id: "job_err_iso".into(),
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
    journal
        .record_files_batch("job_err_iso", &file_records)
        .unwrap();

    let recovery = Arc::new(RecoveryController::with_journal(
        Arc::new(volterra_recovery::ErrorQueue::new()),
        volterra_recovery::RetryBudget::default(),
        journal.clone(),
    ));
    let monitor = Arc::new(PerformanceMonitor::new());
    let paused = Arc::new(AtomicBool::new(false));
    let cancelled = Arc::new(AtomicBool::new(false));

    let config = TransferSchedulerConfig {
        source_root: src_dir.path().to_path_buf(),
        target_root: dst_dir.path().to_path_buf(),
        job_id: "job_err_iso".into(),
        chunk_size: 16 * 1024,
        memory_budget_bytes: 16 * 1024 * 1024,
        direct_io: false,
        verify_readback: false,
        allow_overwrite: false,
    };

    let scheduler = TransferScheduler::new(
        config,
        journal.clone(),
        recovery.clone(),
        monitor,
        paused,
        cancelled,
    );
    let res = scheduler.execute_pipeline(&manifest, |_, _, _, _| {});

    // Pipeline should succeed because unreadable file is isolated and quarantined!
    assert!(
        res.is_ok(),
        "Pipeline should continue healthy files despite read error"
    );

    // Restore permissions so tempdir can clean up cleanly
    #[cfg(unix)]
    unsafe {
        let c_path = std::ffi::CString::new(f1.to_string_lossy().as_bytes()).unwrap();
        libc::chmod(c_path.as_ptr(), 0o644);
    }

    // Healthy file must be completed and present in destination!
    let healthy_dst = dst_dir.path().join("healthy.txt");
    assert!(healthy_dst.exists());
    assert_eq!(
        std::fs::read(healthy_dst).unwrap(),
        b"Healthy readable data"
    );

    // Unreadable file must be recorded in quarantine queue
    assert_eq!(recovery.queue().list_all().len(), 1);
}

#[test]
fn test_preflight_capacity_exceeded_rejection() {
    let src_dir = tempdir().unwrap();
    let dst_dir = tempdir().unwrap();
    let journal_dir = tempdir().unwrap();

    let db_path = journal_dir.path().join("journal.db");
    let journal = Arc::new(JournalEngine::open(&db_path).unwrap());
    let recovery = Arc::new(RecoveryController::with_journal(
        Arc::new(volterra_recovery::ErrorQueue::new()),
        volterra_recovery::RetryBudget::default(),
        journal.clone(),
    ));
    let monitor = Arc::new(PerformanceMonitor::new());
    let paused = Arc::new(AtomicBool::new(false));
    let cancelled = Arc::new(AtomicBool::new(false));

    let config = TransferSchedulerConfig {
        source_root: src_dir.path().to_path_buf(),
        target_root: dst_dir.path().to_path_buf(),
        job_id: "job_cap".into(),
        chunk_size: 16 * 1024,
        memory_budget_bytes: 16 * 1024 * 1024,
        direct_io: false,
        verify_readback: false,
        allow_overwrite: false,
    };

    let scheduler = TransferScheduler::new(config, journal, recovery, monitor, paused, cancelled);

    // Manifest with 10 Petabytes
    let manifest = volterra_manifest::Manifest {
        root: src_dir.path().to_path_buf(),
        job_id: "job_cap".into(),
        items: Vec::new(),
        total_bytes: 10 * 1024 * 1024 * 1024 * 1024 * 1024, // 10 PB
        file_count: 0,
        dir_count: 0,
        symlink_count: 0,
        special_count: 0,
    };

    let res = scheduler.preflight_check(&manifest);
    match res {
        Err(TransferError::PreflightCapacityExceeded { required_bytes, .. }) => {
            assert_eq!(required_bytes, 10 * 1024 * 1024 * 1024 * 1024 * 1024);
        }
        other => panic!("Expected PreflightCapacityExceeded, got {:?}", other),
    }
}
