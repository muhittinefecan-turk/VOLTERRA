//! Fault Injection Writer Utilities & Tests (TEST-03 & TEST-04)

use std::sync::atomic::{AtomicUsize, Ordering};
use volterra_transfer::WriteFaultInjector;

/// Mock fault injector simulating destination disk full (ENOSPC) or write errors.
#[derive(Debug)]
pub struct MockFaultyWriter {
    pub max_writable_bytes: u64,
    pub bytes_written: AtomicUsize,
}

impl MockFaultyWriter {
    pub fn new(max_writable_bytes: u64) -> Self {
        Self {
            max_writable_bytes,
            bytes_written: AtomicUsize::new(0),
        }
    }
}

impl WriteFaultInjector for MockFaultyWriter {
    fn on_pwrite(&self, _offset: u64, data: &[u8]) -> Option<Result<usize, std::io::Error>> {
        let current = self.bytes_written.load(Ordering::Relaxed);
        if (current as u64) + (data.len() as u64) > self.max_writable_bytes {
            Some(Err(std::io::Error::from_raw_os_error(libc::ENOSPC)))
        } else {
            self.bytes_written.fetch_add(data.len(), Ordering::Relaxed);
            None
        }
    }
}

// =========================================================================
// TEST-03: FaultInjectingWriter Simulating ENOSPC (Disk Full)
// =========================================================================
pub fn run_test_03_fault_injecting_writer_enospc_rollback_and_preservation() {
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    use tempfile::tempdir;
    use volterra_journal::{JobMode, JobRecord, JobState, JournalEngine};
    use volterra_manifest::{ManifestScanner, ScanOptions};
    use volterra_profiler::PerformanceMonitor;
    use volterra_recovery::{
        ErrorCategory, RecoveryClassifier, RecoveryController, RetryBudget,
    };
    use volterra_transfer::{TransferError, TransferScheduler, TransferSchedulerConfig};

    let src = tempdir().unwrap();
    let dst = tempdir().unwrap();
    let jnl_dir = tempdir().unwrap();

    // Create a 256 KiB source file
    let src_file = src.path().join("large_file.dat");
    let payload = vec![0x55u8; 256 * 1024];
    std::fs::write(&src_file, &payload).unwrap();

    let manifest = ManifestScanner::scan_tree(src.path(), &ScanOptions::default()).unwrap();

    let db_path = jnl_dir.path().join("journal.db");
    let journal = Arc::new(JournalEngine::open(&db_path).unwrap());
    let job_id = "job_test_03_enospc".to_string();

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

    let recovery = Arc::new(RecoveryController::with_journal(
        Arc::new(volterra_recovery::ErrorQueue::new()),
        RetryBudget::default(),
        journal.clone(),
    ));

    // Inject ENOSPC after 64 KiB written
    let faulty_writer = Arc::new(MockFaultyWriter::new(64 * 1024));

    let config = TransferSchedulerConfig {
        source_root: src.path().to_path_buf(),
        target_root: dst.path().to_path_buf(),
        job_id: job_id.clone(),
        chunk_size: 32 * 1024,
        memory_budget_bytes: 16 * 1024 * 1024,
        direct_io: false,
        verify_readback: false,
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
    .with_write_fault_injector(faulty_writer);

    let res = scheduler.execute_pipeline(&manifest, |_, _, _, _| {});

    // Must return DiskFull error
    match res {
        Err(TransferError::DiskFull { .. }) => {}
        other => panic!("Expected DiskFull error, got {:?}", other),
    }

    // 1. Destination final file must NEVER exist (atomic rollback prevented publish)
    let final_dest = dst.path().join("large_file.dat");
    assert!(
        !final_dest.exists(),
        "Final destination file must not exist after ENOSPC"
    );

    // 2. Temporary partial file must exist (so verified chunks are durable and not lost)
    let partial_files: Vec<_> = std::fs::read_dir(dst.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().contains(".partial"))
        .collect();
    assert_eq!(
        partial_files.len(),
        1,
        "Partial file should remain for crash resumption"
    );

    // 3. Classifier correctly categorizes ENOSPC as JobFatal
    let classified = RecoveryClassifier::classify_errno(
        libc::ENOSPC,
        volterra_recovery::ErrorDomain::Destination,
        "Disk Full",
    );
    assert_eq!(classified.category, ErrorCategory::StorageFull);
    assert_eq!(
        classified.severity,
        volterra_recovery::ErrorSeverity::JobFatal
    );
}

// =========================================================================
// TEST-04: Permission Denial (EACCES) Handling
// =========================================================================
pub fn run_test_04_permission_denial_eacces_source_and_destination() {
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    use tempfile::tempdir;
    use volterra_journal::{FileRecord, FileState, JobMode, JobRecord, JobState, JournalEngine};
    use volterra_manifest::{ManifestScanner, ScanOptions};
    use volterra_profiler::PerformanceMonitor;
    use volterra_recovery::{
        ErrorCategory, RecoveryController, RetryBudget,
    };
    use volterra_transfer::{ChunkWriter, TransferScheduler, TransferSchedulerConfig};

    let src = tempdir().unwrap();
    let dst = tempdir().unwrap();
    let jnl_dir = tempdir().unwrap();

    // --- Scenario A: Unreadable source file (chmod 000) ---
    let locked_file = src.path().join("locked.txt");
    std::fs::write(&locked_file, b"Confidential data").unwrap();

    let open_file = src.path().join("open.txt");
    std::fs::write(&open_file, b"Public data").unwrap();

    let manifest = ManifestScanner::scan_tree(src.path(), &ScanOptions::default()).unwrap();

    // Restrict permissions on locked file
    #[cfg(unix)]
    unsafe {
        let c_path = std::ffi::CString::new(locked_file.to_string_lossy().as_bytes()).unwrap();
        libc::chmod(c_path.as_ptr(), 0o000);
    }

    let db_path = jnl_dir.path().join("journal.db");
    let journal = Arc::new(JournalEngine::open(&db_path).unwrap());
    let job_id = "job_test_04_eacces".to_string();

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

    let config = TransferSchedulerConfig {
        source_root: src.path().to_path_buf(),
        target_root: dst.path().to_path_buf(),
        job_id: job_id.clone(),
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
        Arc::new(PerformanceMonitor::new()),
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    );

    let res = scheduler.execute_pipeline(&manifest, |_, _, _, _| {});
    assert!(res.is_ok(), "Pipeline should proceed with healthy files");

    // Restore permissions for cleanup
    #[cfg(unix)]
    unsafe {
        let c_path = std::ffi::CString::new(locked_file.to_string_lossy().as_bytes()).unwrap();
        libc::chmod(c_path.as_ptr(), 0o644);
    }

    // Healthy file copied
    assert!(dst.path().join("open.txt").exists());
    // Locked file quarantined
    assert!(!dst.path().join("locked.txt").exists());
    assert_eq!(recovery.queue().list_all().len(), 1);
    assert_eq!(
        recovery.queue().list_all()[0].last_error.category,
        ErrorCategory::PermissionDenied
    );

    // --- Scenario B: Unwriteable destination directory ---
    let read_only_dest = dst.path().join("ro_dir");
    std::fs::create_dir_all(&read_only_dest).unwrap();
    #[cfg(unix)]
    unsafe {
        let c_ro = std::ffi::CString::new(read_only_dest.to_string_lossy().as_bytes()).unwrap();
        libc::chmod(c_ro.as_ptr(), 0o555); // Read-only dir
    }

    let p_path = read_only_dest.join(".volterra.partial");
    let writer_res = ChunkWriter::open_or_create_partial(&p_path, false);
    assert!(
        writer_res.is_err(),
        "Writer must fail with PermissionDenied on read-only directory"
    );

    #[cfg(unix)]
    unsafe {
        let c_ro = std::ffi::CString::new(read_only_dest.to_string_lossy().as_bytes()).unwrap();
        libc::chmod(c_ro.as_ptr(), 0o755); // Restore
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_03_fault_injecting_writer_enospc_rollback_and_preservation() {
        run_test_03_fault_injecting_writer_enospc_rollback_and_preservation();
    }

    #[test]
    fn test_04_permission_denial_eacces_source_and_destination() {
        run_test_04_permission_denial_eacces_source_and_destination();
    }
}
