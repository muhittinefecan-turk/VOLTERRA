//! Fault Injection Reader Utilities & Tests (TEST-01 & TEST-02)

use std::sync::atomic::{AtomicUsize, Ordering};
use volterra_transfer::ReadFaultInjector;

/// Mock fault injector simulating EIO bad sectors, short reads, or EINTR interruptions.
#[derive(Debug)]
pub struct MockFaultyReader {
    pub bad_sector_ranges: Vec<(u64, u64)>, // [start_offset, end_offset) -> EIO
    pub short_read_chunk_limit: Option<usize>, // clamps each pread to N bytes
    pub inject_eintr_count: AtomicUsize,    // injects EINTR N times before succeeding
    pub call_count: AtomicUsize,
}

impl MockFaultyReader {
    pub fn new() -> Self {
        Self {
            bad_sector_ranges: Vec::new(),
            short_read_chunk_limit: None,
            inject_eintr_count: AtomicUsize::new(0),
            call_count: AtomicUsize::new(0),
        }
    }

    pub fn with_bad_sector(mut self, start: u64, length: u64) -> Self {
        self.bad_sector_ranges.push((start, start + length));
        self
    }

    pub fn with_short_read_limit(mut self, limit: usize) -> Self {
        self.short_read_chunk_limit = Some(limit);
        self
    }

    pub fn with_eintr_injections(mut self, count: usize) -> Self {
        self.inject_eintr_count = AtomicUsize::new(count);
        self
    }
}

impl Default for MockFaultyReader {
    fn default() -> Self {
        Self::new()
    }
}

impl ReadFaultInjector for MockFaultyReader {
    fn on_pread(
        &self,
        offset: u64,
        length: usize,
        _buffer: &mut [u8],
    ) -> Option<Result<usize, std::io::Error>> {
        self.call_count.fetch_add(1, Ordering::Relaxed);

        // 1. Simulate EINTR if configured
        let remaining_eintr = self.inject_eintr_count.load(Ordering::Relaxed);
        if remaining_eintr > 0 {
            self.inject_eintr_count.fetch_sub(1, Ordering::Relaxed);
            return Some(Err(std::io::Error::from_raw_os_error(libc::EINTR)));
        }

        // 2. Check bad sectors for EIO
        for &(start, end) in &self.bad_sector_ranges {
            let req_end = offset + length as u64;
            if offset < end && req_end > start {
                return Some(Err(std::io::Error::from_raw_os_error(libc::EIO)));
            }
        }

        // 3. Check short read limit
        if let Some(limit) = self.short_read_chunk_limit {
            if length > limit {
                return Some(Ok(limit));
            }
        }

        None
    }
}

// =========================================================================
// TEST-01: FaultInjectingReader Simulating EIO Sector Errors
// =========================================================================
pub fn run_test_01_fault_injecting_reader_eio_quarantine_and_isolation() {
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    use tempfile::tempdir;
    use volterra_journal::{FileRecord, FileState, JobMode, JobRecord, JobState, JournalEngine};
    use volterra_manifest::{ManifestScanner, ScanOptions};
    use volterra_profiler::PerformanceMonitor;
    use volterra_recovery::{
        ErrorCategory, ExtentInterval, QuarantineState, RecoveryController, RetryBudget,
    };
    use volterra_transfer::{TransferScheduler, TransferSchedulerConfig};

    let src = tempdir().unwrap();
    let dst = tempdir().unwrap();
    let jnl_dir = tempdir().unwrap();

    // 1. Create 3 files: healthy_1.bin, faulty.bin, healthy_2.bin
    let f1 = src.path().join("healthy_1.bin");
    let f_bad = src.path().join("faulty.bin");
    let f2 = src.path().join("healthy_2.bin");

    std::fs::write(&f1, vec![0x11u8; 16 * 1024]).unwrap();
    std::fs::write(&f_bad, vec![0xEEu8; 128 * 1024]).unwrap(); // 128 KiB
    std::fs::write(&f2, vec![0x22u8; 16 * 1024]).unwrap();

    let manifest = ManifestScanner::scan_tree(src.path(), &ScanOptions::default()).unwrap();
    let bad_item = manifest
        .items
        .iter()
        .find(|i| i.relative_path.to_str().unwrap() == "faulty.bin")
        .unwrap();

    let db_path = jnl_dir.path().join("journal.db");
    let journal = Arc::new(JournalEngine::open(&db_path).unwrap());
    let job_id = "job_test_01".to_string();

    journal
        .create_job(&JobRecord {
            id: job_id.clone(),
            source_root: src.path().to_path_buf(),
            target_root: dst.path().to_path_buf(),
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

    // Configure fault injector: inject EIO bad sector at offset 32 KiB for length 4 KiB
    let faulty_reader =
        Arc::new(MockFaultyReader::new().with_bad_sector(32 * 1024, 4 * 1024));

    let config = TransferSchedulerConfig {
        source_root: src.path().to_path_buf(),
        target_root: dst.path().to_path_buf(),
        job_id: job_id.clone(),
        chunk_size: 32 * 1024,
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

    let res = scheduler.execute_pipeline(&manifest, |_, _, _, _| {});

    // Pipeline succeeds because unreadable file is isolated into error quarantine!
    assert!(
        res.is_ok(),
        "Pipeline must continue healthy files despite read EIO"
    );

    // 1. Healthy files exist and match contents
    let dst_1 = dst.path().join("healthy_1.bin");
    let dst_2 = dst.path().join("healthy_2.bin");
    assert!(dst_1.exists());
    assert!(dst_2.exists());
    assert_eq!(std::fs::read(dst_1).unwrap(), vec![0x11u8; 16 * 1024]);
    assert_eq!(std::fs::read(dst_2).unwrap(), vec![0x22u8; 16 * 1024]);

    // 2. Corrupted file NEVER published to final destination
    let dst_bad = dst.path().join("faulty.bin");
    assert!(
        !dst_bad.exists(),
        "Faulty file must not be published to target"
    );

    // 3. Error queue contains quarantined file with exact extent record
    let q_list = recovery.queue().list_all();
    assert_eq!(q_list.len(), 1);
    let qf = &q_list[0];
    assert_eq!(qf.file_id, bad_item.id);
    assert_eq!(qf.state, QuarantineState::Quarantined);
    assert_eq!(qf.last_error.category, ErrorCategory::DeviceIo);

    // 4. Extent map recorded the bad sector interval
    let expected_interval = ExtentInterval::new(32 * 1024, 32 * 1024).unwrap();
    assert!(qf.failed_extents.intervals().contains(&expected_interval));

    // 5. Test Interactive Control: Skip file
    let skip_res = recovery.apply_action(volterra_recovery::InteractiveAction::SkipFile {
        file_id: bad_item.id,
        reason: Some("Hardware sector defect".into()),
    });
    assert!(skip_res.is_ok());
    assert_eq!(
        recovery.queue().get_file(bad_item.id).unwrap().state,
        QuarantineState::Skipped
    );
}

// =========================================================================
// TEST-02: Short Read Simulation Loop Verification
// =========================================================================
pub fn run_test_02_short_read_simulation_loop_verification() {
    use std::sync::Arc;
    use tempfile::tempdir;
    use volterra_transfer::{ChunkReader, TransferError};

    let dir = tempdir().unwrap();
    let src_file = dir.path().join("large_payload.dat");
    let payload = (0..65536).map(|i| (i % 256) as u8).collect::<Vec<u8>>(); // 64 KiB
    std::fs::write(&src_file, &payload).unwrap();

    // Custom injector returning at most 1024 bytes per pread call (forcing 64 loop iterations)
    struct ShortReadInjector {
        step_size: usize,
    }
    impl ReadFaultInjector for ShortReadInjector {
        fn on_pread(
            &self,
            _offset: u64,
            length: usize,
            _buf: &mut [u8],
        ) -> Option<Result<usize, std::io::Error>> {
            let actual = length.min(self.step_size);
            Some(Ok(actual))
        }
    }

    let injector = Arc::new(ShortReadInjector { step_size: 1024 });
    let reader = ChunkReader::open(&src_file, false)
        .unwrap()
        .with_fault_injector(injector);

    let mut buf = vec![0u8; 65536];
    let digest = reader
        .read_chunk(0, 65536, &mut buf)
        .expect("Read chunk through short read loop");

    // Verify all bytes assembled properly
    assert_eq!(
        buf, payload,
        "Buffer assembled via short-read loop must match payload exactly"
    );
    let expected_digest = volterra_integrity::Blake3Hasher::hash_chunk(&payload);
    assert_eq!(digest, expected_digest);

    // Verify EINTR retry resilience in loop
    let eintr_injector = Arc::new(MockFaultyReader::new().with_eintr_injections(3));
    let reader_eintr = ChunkReader::open(&src_file, false)
        .unwrap()
        .with_fault_injector(eintr_injector);
    let mut buf_eintr = vec![0u8; 65536];
    let digest_eintr = reader_eintr
        .read_chunk(0, 65536, &mut buf_eintr)
        .expect("Recover from EINTR");
    assert_eq!(digest_eintr, expected_digest);

    // Verify unexpected EOF when pread returns 0 before length is fulfilled
    struct EofInjector;
    impl ReadFaultInjector for EofInjector {
        fn on_pread(
            &self,
            _offset: u64,
            _len: usize,
            _buf: &mut [u8],
        ) -> Option<Result<usize, std::io::Error>> {
            Some(Ok(0)) // Immediate EOF
        }
    }
    let reader_eof = ChunkReader::open(&src_file, false)
        .unwrap()
        .with_fault_injector(Arc::new(EofInjector));
    let mut buf_eof = vec![0u8; 4096];
    let err = reader_eof.read_chunk(0, 4096, &mut buf_eof).unwrap_err();
    match err {
        TransferError::UnexpectedEof {
            expected, actual, ..
        } => {
            assert_eq!(expected, 4096);
            assert_eq!(actual, 0);
        }
        other => panic!("Expected UnexpectedEof, got {:?}", other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_01_fault_injecting_reader_eio_quarantine_and_isolation() {
        run_test_01_fault_injecting_reader_eio_quarantine_and_isolation();
    }

    #[test]
    fn test_02_short_read_simulation_loop_verification() {
        run_test_02_short_read_simulation_loop_verification();
    }
}
