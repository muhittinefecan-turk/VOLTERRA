//! Fault Injection Crash / SIGKILL WAL Replay Utilities & Tests (TEST-05)

// =========================================================================
// TEST-05: Process Crash / SIGKILL WAL Replay Reconciliation
// =========================================================================
pub fn run_test_05_process_crash_wal_replay_reconciliation_and_chunk_reuse() {
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    use tempfile::tempdir;
    use volterra_journal::{
        ChunkRecord, ChunkState, FileRecord, FileState, JobMode, JobRecord, JobState, JournalEngine,
    };
    use volterra_manifest::{ManifestScanner, ScanOptions};
    use volterra_platform_macos::PlatformAtomic;
    use volterra_profiler::PerformanceMonitor;
    use volterra_recovery::{RecoveryController, RetryBudget};
    use volterra_transfer::{ChunkWriter, TransferScheduler, TransferSchedulerConfig};

    let src = tempdir().unwrap();
    let dst = tempdir().unwrap();
    let jnl_dir = tempdir().unwrap();

    // 1. Create a large source file of 512 KiB (8 chunks of 64 KiB)
    let src_file = src.path().join("crash_target.dat");
    let payload = (0..512 * 1024)
        .map(|i| (i % 251) as u8)
        .collect::<Vec<u8>>();
    std::fs::write(&src_file, &payload).unwrap();

    let manifest = ManifestScanner::scan_tree(src.path(), &ScanOptions::default()).unwrap();
    let item = manifest
        .items
        .iter()
        .find(|i| i.relative_path.to_str().unwrap() == "crash_target.dat")
        .unwrap();

    let db_path = jnl_dir.path().join("crash_test.db");
    let journal = Arc::new(JournalEngine::open(&db_path).unwrap());
    let job_id = "job_crash_sim_05".to_string();

    journal
        .create_job(&JobRecord {
            id: job_id.clone(),
            source_root: src.path().to_path_buf(),
            target_root: dst.path().to_path_buf(),
            source_volume_uuid: None,
            target_volume_uuid: None,
            mode: JobMode::SmartHybrid,
            state: JobState::Running,
            total_files: 1,
            total_bytes: item.size_bytes,
            copied_bytes: 0,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            completed_at: None,
        })
        .unwrap();

    // 2. Simulate pre-crash: compute partial path and write chunks 0, 1, 2, 3 to partial file and journal
    let chunk_size = 64 * 1024;
    let partial_path =
        PlatformAtomic::make_partial_path(&dst.path().join(&item.relative_path), &job_id);
    let rel_partial = partial_path
        .strip_prefix(dst.path())
        .unwrap()
        .to_path_buf();

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
        recovered_bytes: 0,
    };
    journal.record_files_batch(&job_id, &[file_rec]).unwrap();

    let writer = ChunkWriter::open_or_create_partial(&partial_path, false).unwrap();

    for chunk_idx in 0..4 {
        let offset = (chunk_idx * chunk_size) as u64;
        let chunk_data = &payload[offset as usize..(offset as usize + chunk_size)];
        writer.write_chunk(offset, chunk_data).unwrap();

        let digest = volterra_integrity::Blake3Hasher::hash_chunk(chunk_data);
        journal
            .record_chunk_verified(&ChunkRecord {
                file_id: item.id,
                chunk_index: chunk_idx as u32,
                offset_bytes: offset,
                length_bytes: chunk_size as u32,
                digest: Some(volterra_integrity::HashDigest::new(digest).to_hex()),
                state: ChunkState::Verified,
                attempts: 1,
            })
            .unwrap();
    }
    writer.sync_all().unwrap();

    // 3. PROCESS CRASH / SIGKILL SIMULATION: Drop writer and drop journal connection
    drop(writer);
    drop(journal);

    // 4. APPLICATION RELAUNCH: Open journal from disk and reconcile crash
    let journal_reopened = Arc::new(JournalEngine::open(&db_path).unwrap());
    let report = journal_reopened
        .reconcile_job_after_crash(&job_id, dst.path(), true)
        .expect("Crash reconciliation");

    // Verify 4 surviving chunks were verified from disk
    assert_eq!(report.reconciled_files_count, 1);
    assert_eq!(report.verified_chunks_count, 4);
    assert_eq!(report.reverted_chunks_count, 0);
    assert_eq!(report.recovered_bytes, 4 * chunk_size as u64);

    // 5. RESUME JOB: Run transfer scheduler
    let recovery = Arc::new(RecoveryController::with_journal(
        Arc::new(volterra_recovery::ErrorQueue::new()),
        RetryBudget::default(),
        journal_reopened.clone(),
    ));

    let config = TransferSchedulerConfig {
        source_root: src.path().to_path_buf(),
        target_root: dst.path().to_path_buf(),
        job_id: job_id.clone(),
        chunk_size,
        memory_budget_bytes: 16 * 1024 * 1024,
        direct_io: false,
        verify_readback: true,
        allow_overwrite: false,
    };

    // Attach fault injector that fails with EIO on offsets 0..256 KiB.
    // If scheduler attempts to re-read chunks 0..=3 from source, it will fail with EIO!
    let faulty_reader = Arc::new(
        crate::fault_injection::MockFaultyReader::new()
            .with_bad_sector(0, 4 * chunk_size as u64),
    );

    let scheduler = TransferScheduler::new(
        config,
        journal_reopened.clone(),
        recovery,
        Arc::new(PerformanceMonitor::new()),
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    )
    .with_read_fault_injector(faulty_reader.clone());

    let mut progress_events = Vec::new();
    scheduler
        .execute_pipeline(&manifest, |_file_id, chunk_idx, _copied, _total| {
            progress_events.push(chunk_idx);
        })
        .expect("Execute pipeline on resume");

    // 6. Verify chunks 0..=3 were reused without reading from source
    // Only chunks 4..=7 (4 reads) were read from source!
    assert_eq!(
        faulty_reader.call_count.load(std::sync::atomic::Ordering::Relaxed),
        4,
        "Exactly 4 reads should be issued to source (chunks 4..=7)"
    );

    // 7. Verify destination file exists and is 100% bit-exact
    let final_dest = dst.path().join("crash_target.dat");
    assert!(final_dest.exists());
    let dest_data = std::fs::read(final_dest).unwrap();
    assert_eq!(
        dest_data, payload,
        "Final destination must be bit-exact after crash resumption"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_05_process_crash_wal_replay_reconciliation_and_chunk_reuse() {
        run_test_05_process_crash_wal_replay_reconciliation_and_chunk_reuse();
    }
}
