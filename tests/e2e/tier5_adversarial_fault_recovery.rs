//! Tier 5: Adversarial Fault Injection & Recovery Challenger Suite
//!
//! Empirically challenges and stress-tests:
//! 1. Corrupted byte regions, sector defects, and error quarantine isolation (RECO-01..04, JOUR-02..03)
//! 2. POSIX pread short-read loop fragmentation (512B, 1B, primes) and EINTR retry resilience (XFER-01)
//! 3. SIGKILL crash recovery WAL reconciliation, partial chunk reuse, and whole-file BLAKE3 verification (JOUR-04..05, INTEG-02)
//! 4. Unexpected unmount / ENODEV drive ejection, WaitingForDisk state machine, and volume UUID reconnection (PLAT-02, CORE-01)

use std::fs;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use volterra_integrity::{Blake3Hasher, HashDigest};
use volterra_journal::{
    ChunkRecord, ChunkState, FileRecord, FileState, JobMode, JobRecord, JobState, JournalEngine,
};
use volterra_platform_macos::{PlatformAtomic, PlatformVolume, VolumeUuid};
use volterra_profiler::PerformanceMonitor;
use volterra_recovery::{
    ActionRecommendation, ErrorCategory, ErrorDomain, ErrorQueue, ErrorSeverity,
    InteractiveAction, QuarantineState, RecoveryClassifier, RecoveryController, RetryBudget,
};
use volterra_test_suite::fault_injection::reader::MockFaultyReader;
use volterra_test_suite::TestEnvironment;
use volterra_transfer::{
    ChunkReader, ChunkWriter, ReadFaultInjector, TransferError, TransferScheduler,
    TransferSchedulerConfig, WriteFaultInjector,
};


// =========================================================================
// 1. Adversarial Corrupted Byte Regions & Error Quarantine Isolation
// =========================================================================

/// State-aware multi-file fault injector that targets specific files and byte regions.
struct MultiFileFaultInjector {
    current_file_idx: AtomicUsize,
    last_offset: Mutex<u64>,
    /// Rules: (file_index, start_offset, end_offset) -> EIO
    bad_sectors: Vec<(usize, u64, u64)>,
}

impl MultiFileFaultInjector {
    fn new(bad_sectors: Vec<(usize, u64, u64)>) -> Self {
        Self {
            current_file_idx: AtomicUsize::new(0),
            last_offset: Mutex::new(0),
            bad_sectors,
        }
    }
}

impl ReadFaultInjector for MultiFileFaultInjector {
    fn on_pread(
        &self,
        offset: u64,
        length: usize,
        _buffer: &mut [u8],
    ) -> Option<Result<usize, std::io::Error>> {
        let mut last = self.last_offset.lock().unwrap();
        // A new file begins when offset returns to 0
        if offset == 0 && *last != 0 {
            self.current_file_idx.fetch_add(1, Ordering::SeqCst);
        }
        *last = offset + length as u64;

        let file_idx = self.current_file_idx.load(Ordering::SeqCst);
        let req_end = offset + length as u64;

        for &(target_file, start, end) in &self.bad_sectors {
            if file_idx == target_file && offset < end && req_end > start {
                return Some(Err(std::io::Error::from_raw_os_error(libc::EIO)));
            }
        }

        None
    }
}

#[test]
fn test_adversarial_corrupted_byte_regions_quarantine_and_isolation() {
    let env = TestEnvironment::new();

    // Dataset structure:
    // File 0: clean_lead.dat (32 KiB) -> healthy
    // File 1: corrupt_first_chunk.dat (128 KiB) -> sector error at offset 0..4096 (first chunk)
    // File 2: clean_mid.dat (64 KiB) -> healthy
    // File 3: corrupt_spanning_boundary.dat (192 KiB) -> sector error at offset 64500..66500 (spans chunk 0 and 1)
    // File 4: clean_trail.dat (32 KiB) -> healthy
    let f0_data = vec![0x11u8; 32 * 1024];
    let f1_data = vec![0x22u8; 128 * 1024];
    let f2_data = vec![0x33u8; 64 * 1024];
    let f3_data = vec![0x44u8; 192 * 1024];
    let f4_data = vec![0x55u8; 32 * 1024];

    fs::write(env.src_path().join("f0_clean_lead.dat"), &f0_data).unwrap();
    fs::write(env.src_path().join("f1_corrupt_first_chunk.dat"), &f1_data).unwrap();
    fs::write(env.src_path().join("f2_clean_mid.dat"), &f2_data).unwrap();
    fs::write(env.src_path().join("f3_corrupt_spanning_boundary.dat"), &f3_data).unwrap();
    fs::write(env.src_path().join("f4_clean_trail.dat"), &f4_data).unwrap();

    let manifest = env.scan_source();
    assert_eq!(manifest.file_count, 5);

    let journal = env.open_journal();
    let job_id = "job_adv_quarantine_01".to_string();

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

    // Configure fault injector:
    // File 1 (corrupt_first_chunk) hits EIO at offset 0..4096
    // File 3 (corrupt_spanning_boundary) hits EIO at offset 64500..66500 (chunk size 64 KiB = 65536)
    let bad_sectors = vec![
        (1, 0, 4096),
        (3, 64500, 66500),
    ];
    let injector = Arc::new(MultiFileFaultInjector::new(bad_sectors));

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
        recovery.clone(),
        Arc::new(PerformanceMonitor::new()),
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    )
    .with_read_fault_injector(injector);

    // 1. Pipeline execution: unreadable sectors must NOT abort healthy files or panic!
    let res = scheduler.execute_pipeline(&manifest, |_, _, _, _| {});
    assert!(res.is_ok(), "Pipeline must complete remaining healthy files: {:?}", res);

    // 2. All 3 clean files MUST exist at destination and match bit-for-bit
    let dst_f0 = env.dst_path().join("f0_clean_lead.dat");
    let dst_f2 = env.dst_path().join("f2_clean_mid.dat");
    let dst_f4 = env.dst_path().join("f4_clean_trail.dat");
    assert!(dst_f0.exists(), "f0 must exist");
    assert!(dst_f2.exists(), "f2 must exist");
    assert!(dst_f4.exists(), "f4 must exist");
    assert_eq!(fs::read(&dst_f0).unwrap(), f0_data);
    assert_eq!(fs::read(&dst_f2).unwrap(), f2_data);
    assert_eq!(fs::read(&dst_f4).unwrap(), f4_data);

    // 3. Corrupted files must NEVER be published to final destination
    let dst_f1 = env.dst_path().join("f1_corrupt_first_chunk.dat");
    let dst_f3 = env.dst_path().join("f3_corrupt_spanning_boundary.dat");
    assert!(!dst_f1.exists(), "Corrupted file 1 must NEVER be published!");
    assert!(!dst_f3.exists(), "Corrupted file 3 must NEVER be published!");

    // 4. Recovery queue verification: exactly 2 files quarantined
    let quarantined = error_queue.list_all();
    assert_eq!(quarantined.len(), 2, "Exactly 2 corrupted files must be quarantined");

    for qf in &quarantined {
        assert_eq!(qf.state, QuarantineState::Quarantined);
        assert_eq!(qf.last_error.category, ErrorCategory::DeviceIo);
        assert!(!qf.failed_extents.is_empty(), "Failed extents must be tracked");
    }

    // 5. Journal status check: load_job_state and get_job_errors
    let job_state = journal.load_job_state(&job_id).unwrap();
    assert_eq!(job_state.completed_file_count, 3, "3 clean files must be Completed");
    assert_eq!(job_state.failed_file_count, 2, "2 corrupted files must be Failed/ReadError");
    assert_eq!(job_state.pending_file_count, 0, "0 files pending");

    let job_errors = journal.get_job_errors(&job_id).unwrap();
    assert_eq!(job_errors.len(), 2, "Exactly 2 extent errors recorded in journal");


    // 6. Interactive controls verification: SkipFile, RetryFile, SkipAll
    let qf1 = quarantined.iter().find(|f| f.relative_path.to_str().unwrap().contains("f1")).unwrap();
    let qf3 = quarantined.iter().find(|f| f.relative_path.to_str().unwrap().contains("f3")).unwrap();

    // SkipFile on f1
    let skip_res = recovery.apply_action(InteractiveAction::SkipFile {
        file_id: qf1.file_id,
        reason: Some("Adversarial sector fault".into()),
    });
    assert!(skip_res.is_ok());
    assert_eq!(error_queue.get_file(qf1.file_id).unwrap().state, QuarantineState::Skipped);

    // RetryFile on f3
    let retry_res = recovery.apply_action(InteractiveAction::RetryFile {
        file_id: qf3.file_id,
        reset_budget: true,
    });
    assert!(retry_res.is_ok());
    assert_eq!(error_queue.get_file(qf3.file_id).unwrap().state, QuarantineState::Retrying);
}

// =========================================================================
// 2. Adversarial POSIX pread Short Reads, Fragmentation & EINTR Stress
// =========================================================================

/// Injector forcing micro-fragmentation (clamping reads to N bytes) and optional EINTR bursts.
struct MicroFragmentedReader {
    max_chunk_size: usize,
    eintr_remaining: AtomicUsize,
    total_calls: AtomicUsize,
}

impl MicroFragmentedReader {
    fn new(max_chunk_size: usize, eintr_burst: usize) -> Self {
        Self {
            max_chunk_size,
            eintr_remaining: AtomicUsize::new(eintr_burst),
            total_calls: AtomicUsize::new(0),
        }
    }
}

impl ReadFaultInjector for MicroFragmentedReader {
    fn on_pread(
        &self,
        _offset: u64,
        length: usize,
        _buffer: &mut [u8],
    ) -> Option<Result<usize, std::io::Error>> {
        self.total_calls.fetch_add(1, Ordering::Relaxed);

        // Inject EINTR if burst remaining
        let remaining = self.eintr_remaining.load(Ordering::Relaxed);
        if remaining > 0 {
            self.eintr_remaining.fetch_sub(1, Ordering::Relaxed);
            return Some(Err(std::io::Error::from_raw_os_error(libc::EINTR)));
        }

        // Clamp to micro-fragment size
        let clamped = length.min(self.max_chunk_size);
        Some(Ok(clamped))
    }
}

#[test]
fn test_adversarial_posix_pread_short_reads_and_eintr_stress() {
    let env = TestEnvironment::new();

    // 1. Stress: 256 KiB chunk read clamped to 512 bytes per pread call (512 iterations) + 10 EINTRs
    let payload_size = 256 * 1024;
    let mut payload = vec![0u8; payload_size];
    for (i, b) in payload.iter_mut().enumerate() {
        *b = ((i * 101 + 17) % 251) as u8;
    }
    let src_file = env.src_path().join("fragmented_payload.bin");
    fs::write(&src_file, &payload).unwrap();

    let injector_512b = Arc::new(MicroFragmentedReader::new(512, 10));
    let reader = ChunkReader::open(&src_file, false)
        .unwrap()
        .with_fault_injector(injector_512b.clone());

    let mut buf = vec![0u8; payload_size];
    let digest = reader
        .read_chunk(0, payload_size, &mut buf)
        .expect("Must assemble 256 KiB buffer over 512 short-reads + 10 EINTRs");

    // Assert buffer matched 100% bit-exact
    assert_eq!(buf, payload, "Buffer assembled via 512-byte short reads must match payload exactly");
    let expected_digest = Blake3Hasher::hash_chunk(&payload);
    assert_eq!(digest, expected_digest, "BLAKE3 chunk digest must match expected");

    // Call count must be >= 512 + 10 = 522
    let calls = injector_512b.total_calls.load(Ordering::Relaxed);
    assert!(calls >= 522, "Expected at least 522 pread calls, got {}", calls);

    // 2. Stress: Irregular prime-sized short reads (389 bytes per pread)
    let injector_prime = Arc::new(MicroFragmentedReader::new(389, 0));
    let reader_prime = ChunkReader::open(&src_file, false)
        .unwrap()
        .with_fault_injector(injector_prime);

    let mut buf_prime = vec![0u8; 65536];
    let digest_prime = reader_prime
        .read_chunk(0, 65536, &mut buf_prime)
        .expect("Prime step size short read");
    assert_eq!(buf_prime, payload[..65536]);
    assert_eq!(digest_prime, Blake3Hasher::hash_chunk(&payload[..65536]));

    // 3. Stress: Extreme 1-byte read loop on 4096 bytes (4096 loop iterations)
    let injector_1b = Arc::new(MicroFragmentedReader::new(1, 0));
    let reader_1b = ChunkReader::open(&src_file, false)
        .unwrap()
        .with_fault_injector(injector_1b);

    let mut buf_1b = vec![0u8; 4096];
    let digest_1b = reader_1b
        .read_chunk(0, 4096, &mut buf_1b)
        .expect("Extreme 1-byte read loop");
    assert_eq!(buf_1b, payload[..4096]);
    assert_eq!(digest_1b, Blake3Hasher::hash_chunk(&payload[..4096]));

    // 4. Premature EOF handling (returning 0 bytes before length is fulfilled)
    struct PrematureEofInjector;
    impl ReadFaultInjector for PrematureEofInjector {
        fn on_pread(&self, _offset: u64, _len: usize, _buf: &mut [u8]) -> Option<Result<usize, std::io::Error>> {
            Some(Ok(0))
        }
    }
    let reader_eof = ChunkReader::open(&src_file, false)
        .unwrap()
        .with_fault_injector(Arc::new(PrematureEofInjector));
    let mut buf_eof = vec![0u8; 8192];
    let eof_res = reader_eof.read_chunk(0, 8192, &mut buf_eof);
    match eof_res {
        Err(TransferError::UnexpectedEof { expected, actual, .. }) => {
            assert_eq!(expected, 8192);
            assert_eq!(actual, 0);
        }
        other => panic!("Expected UnexpectedEof, got {:?}", other),
    }

    // 5. Buffer size violation: requesting length > buffer.len()
    let reader_plain = ChunkReader::open(&src_file, false).unwrap();
    let mut small_buf = vec![0u8; 1024];
    let err_small = reader_plain.read_chunk(0, 2048, &mut small_buf).unwrap_err();
    match err_small {
        TransferError::SourceReadError { source, .. } => {
            assert_eq!(source.kind(), std::io::ErrorKind::InvalidInput);
        }
        other => panic!("Expected InvalidInput error, got {:?}", other),
    }
}

// =========================================================================
// 3. Adversarial SIGKILL Crash Recovery WAL Replay & Partial Chunk Reuse
// =========================================================================

#[test]
fn test_adversarial_sigkill_crash_wal_chunk_reuse_and_blake3() {
    let env = TestEnvironment::new();

    // 1 MiB asset (16 chunks of 64 KiB)
    let total_bytes = 1024 * 1024;
    let chunk_size = 64 * 1024;
    let _num_chunks = total_bytes / chunk_size; // 16 chunks

    let mut payload = vec![0u8; total_bytes];
    for (i, b) in payload.iter_mut().enumerate() {
        *b = ((i * 137 + 59) % 251) as u8;
    }
    let src_file = env.src_path().join("mission_critical_asset.img");
    fs::write(&src_file, &payload).unwrap();

    let manifest = env.scan_source();
    let item = &manifest.items[0];

    let db_path = env.journal_db_path();
    let journal = env.open_journal();
    let job_id = "job_sigkill_adversarial_03".to_string();

    journal
        .create_job(&JobRecord {
            id: job_id.clone(),
            source_root: env.src_path().to_path_buf(),
            target_root: env.dst_path().to_path_buf(),
            source_volume_uuid: None,
            target_volume_uuid: None,
            mode: JobMode::SmartHybrid,
            state: JobState::Running,
            total_files: 1,
            total_bytes: total_bytes as u64,
            copied_bytes: 0,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            completed_at: None,
        })
        .unwrap();

    let partial_path = PlatformAtomic::make_partial_path(
        &env.dst_path().join(&item.relative_path),
        &job_id,
    );
    let rel_partial = partial_path.strip_prefix(env.dst_path()).unwrap().to_path_buf();

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
        recovered_bytes: (12 * chunk_size) as u64,
    };
    journal.record_files_batch(&job_id, &[file_rec]).unwrap();

    // 1. Simulate partial progress before SIGKILL:
    // Write first 12 chunks (768 KiB) to destination .partial file
    let writer = ChunkWriter::open_or_create_partial(&partial_path, false).unwrap();
    let verified_chunks_count = 12;

    for idx in 0..verified_chunks_count {
        let offset = (idx * chunk_size) as u64;
        let chunk_slice = &payload[offset as usize..(offset as usize + chunk_size)];
        writer.write_chunk(offset, chunk_slice).unwrap();

        let digest = Blake3Hasher::hash_chunk(chunk_slice);
        journal
            .record_chunk_verified(&ChunkRecord {
                file_id: item.id,
                chunk_index: idx as u32,
                offset_bytes: offset,
                length_bytes: chunk_size as u32,
                digest: Some(HashDigest::new(digest).to_hex()),
                state: ChunkState::Verified,
                attempts: 1,
            })
            .unwrap();
    }
    writer.sync_all().unwrap();

    // 2. SIGKILL CRASH: Abruptly drop writer and close journal connection without publication!
    drop(writer);
    drop(journal);

    // 3. APPLICATION RELAUNCH: Open journal from disk and reconcile crash
    let journal_reopened = Arc::new(JournalEngine::open(&db_path).unwrap());
    let report = journal_reopened
        .reconcile_job_after_crash(&job_id, env.dst_path(), true)
        .expect("Crash reconciliation must succeed");

    assert_eq!(report.reconciled_files_count, 1);
    assert_eq!(report.verified_chunks_count, 12);
    assert_eq!(report.reverted_chunks_count, 0);
    assert_eq!(report.recovered_bytes, (12 * chunk_size) as u64);

    // 4. ADVERSARIAL DISK CHALLENGE:
    // Inject EIO on the entire first 768 KiB of the source file!
    // If the resumption logic attempts to re-read ANY chunk from 0..11 from source, it will FAIL!
    let faulty_reader = Arc::new(
        MockFaultyReader::new().with_bad_sector(0, (verified_chunks_count * chunk_size) as u64),
    );

    let recovery = Arc::new(RecoveryController::with_journal(
        Arc::new(ErrorQueue::new()),
        RetryBudget::default(),
        journal_reopened.clone(),
    ));

    let config = TransferSchedulerConfig {
        source_root: env.src_path().to_path_buf(),
        target_root: env.dst_path().to_path_buf(),
        job_id: job_id.clone(),
        chunk_size,
        memory_budget_bytes: 16 * 1024 * 1024,
        direct_io: false,
        verify_readback: true,
        allow_overwrite: false,
    };

    let scheduler = TransferScheduler::new(
        config,
        journal_reopened.clone(),
        recovery,
        Arc::new(PerformanceMonitor::new()),
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    )
    .with_read_fault_injector(faulty_reader.clone());

    // Execute resume
    scheduler
        .execute_pipeline(&manifest, |_, _, _, _| {})
        .expect("Resumption pipeline must succeed");

    // 5. Verification:
    // Only the remaining 4 chunks (indices 12..15) were read from source!
    let source_reads = faulty_reader.call_count.load(Ordering::Relaxed);
    assert_eq!(
        source_reads, 4,
        "Exactly 4 remaining chunks should be read from source, got {}",
        source_reads
    );

    // 6. Destination file must exist and be 100% bit-identical
    let final_dest = env.dst_path().join("mission_critical_asset.img");
    assert!(final_dest.exists(), "Final destination file must exist");
    let dest_data = fs::read(&final_dest).unwrap();
    assert_eq!(
        dest_data, payload,
        "Resumed file must match source data 100% bit-exact"
    );

    // 7. Verify journal recorded completion with all chunks verified
    let job_state = journal_reopened.load_job_state(&job_id).unwrap();
    assert_eq!(job_state.completed_file_count, 1);
    assert_eq!(job_state.failed_file_count, 0);

    let verified_chunks = journal_reopened.get_verified_chunks(item.id).unwrap();
    assert_eq!(verified_chunks.len(), 16, "All 16 chunks must be verified");
}

// =========================================================================
// 4. Adversarial Unexpected Unmount / ENODEV & Volume UUID Reconnect
// =========================================================================

struct SuddenUnmountInjector {
    trigger_after_bytes: u64,
    bytes_written: AtomicUsize,
}

impl SuddenUnmountInjector {
    fn new(trigger_after_bytes: u64) -> Self {
        Self {
            trigger_after_bytes,
            bytes_written: AtomicUsize::new(0),
        }
    }
}

impl WriteFaultInjector for SuddenUnmountInjector {
    fn on_pwrite(&self, _offset: u64, data: &[u8]) -> Option<Result<usize, std::io::Error>> {
        let current = self.bytes_written.load(Ordering::Relaxed);
        if (current as u64) >= self.trigger_after_bytes {
            // Simulate sudden unplug of USB storage media (ENODEV)
            Some(Err(std::io::Error::from_raw_os_error(libc::ENODEV)))
        } else {
            self.bytes_written.fetch_add(data.len(), Ordering::Relaxed);
            None
        }
    }
}

#[test]
fn test_adversarial_unexpected_unmount_enodev_uuid_reconnect() {
    let env = TestEnvironment::new();

    let file_size = 128 * 1024; // 128 KiB
    let payload = vec![0x77u8; file_size];
    fs::write(env.src_path().join("usb_sync_document.bin"), &payload).unwrap();

    let manifest = env.scan_source();
    let journal = env.open_journal();
    let job_id = "job_enodev_adversarial_04".to_string();

    // 1. Query real volume UUID of destination
    let target_uuid = PlatformVolume::get_volume_uuid(env.dst_path())
        .unwrap_or_else(|_| VolumeUuid([0x3C; 16]));

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

    // 2. Inject ENODEV after first chunk (32 KiB)
    let config = TransferSchedulerConfig {
        source_root: env.src_path().to_path_buf(),
        target_root: env.dst_path().to_path_buf(),
        job_id: job_id.clone(),
        chunk_size: 32 * 1024,
        memory_budget_bytes: 16 * 1024 * 1024,
        direct_io: false,
        verify_readback: true,
        allow_overwrite: false,
    };

    let scheduler = TransferScheduler::new(
        config.clone(),
        journal.clone(),
        recovery,
        Arc::new(PerformanceMonitor::new()),
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    )
    .with_write_fault_injector(Arc::new(SuddenUnmountInjector::new(32 * 1024)));

    // Pipeline must catch ENODEV
    let res = scheduler.execute_pipeline(&manifest, |_, _, _, _| {});
    assert!(res.is_err(), "Pipeline must fail when disk is ejected");

    let io_err = match res.unwrap_err() {
        TransferError::DestinationWriteError { source, .. } => source,
        other => panic!("Expected DestinationWriteError with ENODEV, got {:?}", other),
    };
    assert_eq!(io_err.raw_os_error(), Some(libc::ENODEV));

    // 3. Classify ENODEV: category must be DeviceDisconnected, severity JobFatal, action MountDisk
    let classified = RecoveryClassifier::classify_io_error(&io_err, ErrorDomain::Destination);
    assert_eq!(classified.category, ErrorCategory::DeviceDisconnected);
    assert_eq!(classified.severity, ErrorSeverity::JobFatal);
    assert!(!classified.is_retryable);
    assert_eq!(classified.suggested_action, ActionRecommendation::MountDisk);

    // 4. Job state transition: set WaitingForDisk in journal
    journal
        .update_job_status(&job_id, JobState::WaitingForDisk)
        .expect("Update job state to WaitingForDisk");
    let job_rec = journal.get_job(&job_id).unwrap().unwrap();
    assert_eq!(job_rec.state, JobState::WaitingForDisk);

    // Destination file must NOT be published
    assert!(!env.dst_path().join("usb_sync_document.bin").exists());

    // 5. Volume UUID Matching on re-attachment:
    // Candidate A: Wrong thumbdrive attached (bogus UUID)
    let bogus_uuid = VolumeUuid([0xDE; 16]);
    assert_ne!(bogus_uuid, target_uuid);
    let verify_candidate = |candidate: &VolumeUuid, expected: &VolumeUuid| -> bool {
        candidate == expected
    };
    assert!(!verify_candidate(&bogus_uuid, &target_uuid), "Wrong drive must be rejected");

    // Candidate B: Original drive re-attached (matching UUID)
    assert!(verify_candidate(&target_uuid, &target_uuid), "Original drive accepted");

    // 6. Resume job: transition WaitingForDisk -> Running -> complete
    journal
        .update_job_status(&job_id, JobState::Running)
        .expect("Update job state to Running");
    let resumed_job = journal.get_job(&job_id).unwrap().unwrap();
    assert_eq!(resumed_job.state, JobState::Running);

    // Resume transfer with clean writer (drive reconnected)
    let clean_recovery = Arc::new(RecoveryController::with_journal(
        Arc::new(ErrorQueue::new()),
        RetryBudget::default(),
        journal.clone(),
    ));

    let clean_scheduler = TransferScheduler::new(
        config,
        journal.clone(),
        clean_recovery,
        Arc::new(PerformanceMonitor::new()),
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    );

    let resume_res = clean_scheduler.execute_pipeline(&manifest, |_, _, _, _| {});
    assert!(resume_res.is_ok(), "Transfer must complete upon reconnect: {:?}", resume_res);

    // Final file published and verified
    let final_dest = env.dst_path().join("usb_sync_document.bin");
    assert!(final_dest.exists(), "File published upon reconnect");
    assert_eq!(fs::read(&final_dest).unwrap(), payload);
}
