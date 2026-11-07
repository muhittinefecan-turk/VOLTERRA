use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use tempfile::tempdir;
use volterra_journal::{
    CheckpointMode, ChunkRecord, ChunkState, ExtentErrorRecord, FileRecord, FileState, JobMode,
    JobRecord, JobState, JournalEngine,
};

fn create_sample_job(id: &str) -> JobRecord {
    JobRecord {
        id: id.to_string(),
        source_root: PathBuf::from("/Volumes/Source"),
        target_root: PathBuf::from("/Volumes/Target"),
        source_volume_uuid: Some("UUID-SRC-1234".to_string()),
        target_volume_uuid: Some("UUID-TGT-5678".to_string()),
        mode: JobMode::SmartHybrid,
        state: JobState::Pending,
        total_files: 0,
        total_bytes: 0,
        copied_bytes: 0,
        created_at: chrono::Utc::now().to_rfc3339(),
        updated_at: chrono::Utc::now().to_rfc3339(),
        completed_at: None,
    }
}

fn create_sample_file(job_id: &str, relative_path: &str, size: u64) -> FileRecord {
    FileRecord {
        id: 0,
        job_id: job_id.to_string(),
        relative_path: PathBuf::from(relative_path),
        source_size: size,
        source_mtime_ns: 1_700_000_000_000_000_000,
        source_btime_ns: Some(1_690_000_000_000_000_000),
        posix_mode: 0o644,
        source_file_id: "16777220:123456".to_string(),
        temp_path: Some(PathBuf::from(format!(
            ".volterra.{}.partial",
            relative_path
        ))),
        state: FileState::Pending,
        verified_digest: None,
        error_code: None,
        recovered_bytes: 0,
    }
}

/// TEST-JOUR-01: SQLite WAL Mode and Pragmas Initialization
#[test]
fn test_jour_01_wal_mode_and_pragmas() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("volterra.db");
    let engine = JournalEngine::open(&db_path).unwrap();

    let conn = engine.lock_conn();

    let journal_mode: String = conn
        .query_row("PRAGMA journal_mode;", [], |row| row.get(0))
        .unwrap();
    assert_eq!(journal_mode.to_lowercase(), "wal");

    let synchronous: i32 = conn
        .query_row("PRAGMA synchronous;", [], |row| row.get(0))
        .unwrap();
    assert_eq!(synchronous, 1); // 1 = NORMAL

    let foreign_keys: i32 = conn
        .query_row("PRAGMA foreign_keys;", [], |row| row.get(0))
        .unwrap();
    assert_eq!(foreign_keys, 1); // 1 = ON

    let busy_timeout: i32 = conn
        .query_row("PRAGMA busy_timeout;", [], |row| row.get(0))
        .unwrap();
    assert_eq!(busy_timeout, 5000);
}

/// TEST-JOUR-02: Atomic Job Creation and Retrieval
#[test]
fn test_jour_02_atomic_job_creation_and_retrieval() {
    let engine = JournalEngine::open_in_memory().unwrap();
    let job = create_sample_job("job_alpha");

    engine.create_job(&job).unwrap();

    let loaded_state = engine.load_job_state("job_alpha").unwrap();
    assert_eq!(loaded_state.job.id, "job_alpha");
    assert_eq!(loaded_state.job.mode, JobMode::SmartHybrid);
    assert_eq!(loaded_state.job.state, JobState::Pending);
    assert_eq!(
        loaded_state.job.source_root,
        PathBuf::from("/Volumes/Source")
    );
    assert_eq!(
        loaded_state.job.target_root,
        PathBuf::from("/Volumes/Target")
    );
    assert_eq!(
        loaded_state.job.source_volume_uuid.as_deref(),
        Some("UUID-SRC-1234")
    );
    assert_eq!(
        loaded_state.job.target_volume_uuid.as_deref(),
        Some("UUID-TGT-5678")
    );
}

/// TEST-JOUR-03: Duplicate Job Creation Rejection
#[test]
fn test_jour_03_duplicate_job_rejection() {
    let engine = JournalEngine::open_in_memory().unwrap();
    let job = create_sample_job("job_dup");

    engine.create_job(&job).unwrap();
    let second_try = engine.create_job(&job);

    assert!(second_try.is_err());
    match second_try {
        Err(volterra_journal::JournalError::Sqlite(e)) => {
            let msg = e.to_string();
            assert!(msg.contains("UNIQUE") || msg.contains("PRIMARY KEY"));
        }
        _ => panic!("Expected Sqlite primary key / unique constraint error"),
    }
}

/// TEST-JOUR-04: Batch Manifest Files Insertion (500 records)
#[test]
fn test_jour_04_batch_manifest_files_insertion() {
    let engine = JournalEngine::open_in_memory().unwrap();
    let job = create_sample_job("job_batch_500");
    engine.create_job(&job).unwrap();

    let mut files = Vec::new();
    let mut expected_total_bytes: u64 = 0;
    for i in 0..500 {
        let size = 1024 * (i + 1) as u64;
        expected_total_bytes += size;
        files.push(create_sample_file(
            "job_batch_500",
            &format!("folder/file_{:04}.bin", i),
            size,
        ));
    }

    engine.record_files_batch("job_batch_500", &files).unwrap();

    {
        let conn = engine.lock_conn();
        let count: u64 = conn
            .query_row(
                "SELECT COUNT(*) FROM files WHERE job_id = 'job_batch_500'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 500);
    }

    let updated_job = engine.get_job("job_batch_500").unwrap().unwrap();
    assert_eq!(updated_job.total_files, 500);
    assert_eq!(updated_job.total_bytes, expected_total_bytes);
}

/// TEST-JOUR-05: Unique Constraint on (job_id, relative_path)
#[test]
fn test_jour_05_unique_constraint_file_path() {
    let engine = JournalEngine::open_in_memory().unwrap();
    let job = create_sample_job("job_path_unique");
    engine.create_job(&job).unwrap();

    let file1 = create_sample_file("job_path_unique", "docs/readme.txt", 100);
    let file2 = create_sample_file("job_path_unique", "docs/readme.txt", 200);

    engine
        .record_files_batch("job_path_unique", &[file1])
        .unwrap();
    let res = engine.record_files_batch("job_path_unique", &[file2]);

    assert!(res.is_err());
    match res {
        Err(volterra_journal::JournalError::Sqlite(e)) => {
            let msg = e.to_string();
            assert!(msg.contains("UNIQUE") || msg.contains("unique"));
        }
        _ => panic!("Expected SQLite unique constraint error"),
    }
}

/// TEST-JOUR-06: Chunk Verified Update & Digest Recording
#[test]
fn test_jour_06_chunk_verified_update() {
    let engine = JournalEngine::open_in_memory().unwrap();
    let job = create_sample_job("job_chunk_test");
    engine.create_job(&job).unwrap();

    let file = create_sample_file("job_chunk_test", "data.dat", 4 * 1024 * 1024);
    engine
        .record_files_batch("job_chunk_test", &[file])
        .unwrap();

    let file_id: i64 = {
        let conn = engine.lock_conn();
        conn.query_row(
            "SELECT id FROM files WHERE job_id = 'job_chunk_test'",
            [],
            |r| r.get(0),
        )
        .unwrap()
    };

    // Insert 4 chunks as Pending
    let mut pending_chunks = Vec::new();
    for i in 0..4 {
        pending_chunks.push(ChunkRecord {
            file_id,
            chunk_index: i,
            offset_bytes: (i as u64) * 1024 * 1024,
            length_bytes: 1024 * 1024,
            digest: None,
            state: ChunkState::Pending,
            attempts: 0,
        });
    }
    engine.record_chunks_batch(&pending_chunks).unwrap();

    // Verify chunk 0
    let digest_hex = "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262";
    let verified_chunk = ChunkRecord {
        file_id,
        chunk_index: 0,
        offset_bytes: 0,
        length_bytes: 1024 * 1024,
        digest: Some(digest_hex.to_string()),
        state: ChunkState::Verified,
        attempts: 1,
    };
    engine.record_chunk_verified(&verified_chunk).unwrap();

    let verified = engine.get_verified_chunks(file_id).unwrap();
    assert_eq!(verified.len(), 1);
    assert_eq!(verified[0].chunk_index, 0);
    assert_eq!(verified[0].state, ChunkState::Verified);
    assert_eq!(verified[0].digest.as_deref(), Some(digest_hex));

    let unverified = engine.get_unverified_chunks(file_id).unwrap();
    assert_eq!(unverified.len(), 3);

    // Verify file and job recovered/copied bytes were updated atomically
    let job_after = engine.get_job("job_chunk_test").unwrap().unwrap();
    assert_eq!(job_after.copied_bytes, 1024 * 1024);
}

/// TEST-JOUR-07: File Lifecycle State Machine Transitions
#[test]
fn test_jour_07_file_lifecycle_transitions() {
    let engine = JournalEngine::open_in_memory().unwrap();
    let job = create_sample_job("job_lifecycle");
    engine.create_job(&job).unwrap();

    let file = create_sample_file("job_lifecycle", "test.mp4", 5000);
    engine.record_files_batch("job_lifecycle", &[file]).unwrap();

    let file_id: i64 = {
        let conn = engine.lock_conn();
        conn.query_row(
            "SELECT id FROM files WHERE job_id = 'job_lifecycle'",
            [],
            |r| r.get(0),
        )
        .unwrap()
    };

    // Pending -> Copying
    engine
        .update_file_status(file_id, FileState::Copying)
        .unwrap();
    {
        let conn = engine.lock_conn();
        let state: String = conn
            .query_row("SELECT state FROM files WHERE id = ?1", [file_id], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(state, "Copying");
    }

    // Copying -> Verifying
    engine
        .update_file_status(file_id, FileState::Verifying)
        .unwrap();
    {
        let conn = engine.lock_conn();
        let state: String = conn
            .query_row("SELECT state FROM files WHERE id = ?1", [file_id], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(state, "Verifying");
    }

    // Verifying -> Completed
    let dummy_digest = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    engine
        .update_file_completion(file_id, dummy_digest, 5000)
        .unwrap();
    {
        let conn = engine.lock_conn();
        let (state, digest, recovered): (String, Option<String>, u64) = conn
            .query_row(
                "SELECT state, verified_digest, recovered_bytes FROM files WHERE id = ?1",
                [file_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(state, "Completed");
        assert_eq!(digest.as_deref(), Some(dummy_digest));
        assert_eq!(recovered, 5000);
    }
}

/// TEST-JOUR-08: Extent Error Recording and Retrieval
#[test]
fn test_jour_08_extent_error_recording_and_retrieval() {
    let engine = JournalEngine::open_in_memory().unwrap();
    let job = create_sample_job("job_err_test");
    engine.create_job(&job).unwrap();

    let file = create_sample_file("job_err_test", "bad_drive/file.iso", 10_000_000);
    engine.record_files_batch("job_err_test", &[file]).unwrap();

    let file_id: i64 = {
        let conn = engine.lock_conn();
        conn.query_row(
            "SELECT id FROM files WHERE job_id = 'job_err_test'",
            [],
            |r| r.get(0),
        )
        .unwrap()
    };

    let err_rec = ExtentErrorRecord {
        id: None,
        job_id: "job_err_test".to_string(),
        file_id: Some(file_id),
        offset_bytes: 1048576,
        length_bytes: 4096,
        phase: "read".to_string(),
        errno_code: 5, // libc::EIO
        error_type: "EIO".to_string(),
        attempts: 1,
        message: "Input/output error on sector 2048".to_string(),
        action_taken: None,
        recorded_at: chrono::Utc::now().to_rfc3339(),
    };

    let err_id = engine.record_extent_error(&err_rec).unwrap();
    assert!(err_id > 0);

    let file_errors = engine.get_file_errors(file_id).unwrap();
    assert_eq!(file_errors.len(), 1);
    assert_eq!(file_errors[0].offset_bytes, 1048576);
    assert_eq!(file_errors[0].length_bytes, 4096);
    assert_eq!(file_errors[0].errno_code, 5);
    assert_eq!(file_errors[0].error_type, "EIO");

    let job_errors = engine.get_job_errors("job_err_test").unwrap();
    assert_eq!(job_errors.len(), 1);
    assert_eq!(job_errors[0].id, Some(err_id));
}

/// TEST-JOUR-09: Cascading Foreign Key Deletion
#[test]
fn test_jour_09_cascading_foreign_keys() {
    let engine = JournalEngine::open_in_memory().unwrap();
    let job = create_sample_job("job_cascade");
    engine.create_job(&job).unwrap();

    let mut files = Vec::new();
    for i in 0..10 {
        files.push(create_sample_file(
            "job_cascade",
            &format!("file_{}.txt", i),
            1000,
        ));
    }
    engine
        .record_files_batch("job_cascade", &[files[0].clone()])
        .unwrap();

    let file_id: i64 = {
        let conn = engine.lock_conn();
        conn.query_row(
            "SELECT id FROM files WHERE job_id = 'job_cascade'",
            [],
            |r| r.get(0),
        )
        .unwrap()
    };

    let chunk = ChunkRecord {
        file_id,
        chunk_index: 0,
        offset_bytes: 0,
        length_bytes: 1000,
        digest: None,
        state: ChunkState::Verified,
        attempts: 1,
    };
    engine.record_chunk_verified(&chunk).unwrap();

    let err = ExtentErrorRecord {
        id: None,
        job_id: "job_cascade".to_string(),
        file_id: Some(file_id),
        offset_bytes: 0,
        length_bytes: 512,
        phase: "read".to_string(),
        errno_code: 5,
        error_type: "EIO".to_string(),
        attempts: 1,
        message: "read error".to_string(),
        action_taken: None,
        recorded_at: chrono::Utc::now().to_rfc3339(),
    };
    engine.record_extent_error(&err).unwrap();

    // Verify row counts before delete
    {
        let conn = engine.lock_conn();
        let f_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM files WHERE job_id = 'job_cascade'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let c_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM chunks WHERE file_id = ?1",
                [file_id],
                |r| r.get(0),
            )
            .unwrap();
        let e_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM errors WHERE job_id = 'job_cascade'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(f_count, 1);
        assert_eq!(c_count, 1);
        assert_eq!(e_count, 1);
    }

    // Delete job
    {
        let conn = engine.lock_conn();
        conn.execute("DELETE FROM jobs WHERE id = 'job_cascade'", [])
            .unwrap();
    }

    // Verify CASCADE cleanup
    {
        let conn = engine.lock_conn();
        let f_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM files WHERE job_id = 'job_cascade'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let c_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM chunks WHERE file_id = ?1",
                [file_id],
                |r| r.get(0),
            )
            .unwrap();
        let e_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM errors WHERE job_id = 'job_cascade'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(f_count, 0);
        assert_eq!(c_count, 0);
        assert_eq!(e_count, 0);
    }
}

/// TEST-JOUR-10: Crash Recovery State Reconstruction (Simulated SIGKILL)
#[test]
fn test_jour_10_crash_recovery_state_reconstruction() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("crash_test.db");

    // Phase 1: Create job and populate active transfer
    {
        let engine = JournalEngine::open(&db_path).unwrap();
        let mut job = create_sample_job("job_crash");
        job.state = JobState::Running;
        engine.create_job(&job).unwrap();

        let file1 = create_sample_file("job_crash", "file_done.txt", 1000);
        let file2 = create_sample_file("job_crash", "file_inflight.txt", 3000);
        let file3 = create_sample_file("job_crash", "file_pending.txt", 2000);

        engine
            .record_files_batch("job_crash", &[file1, file2, file3])
            .unwrap();

        // Mark file 1 completed
        let f1_id: i64 = {
            let conn = engine.lock_conn();
            conn.query_row(
                "SELECT id FROM files WHERE relative_path = 'file_done.txt'",
                [],
                |r| r.get(0),
            )
            .unwrap()
        };
        engine
            .update_file_status(f1_id, FileState::Completed)
            .unwrap();

        // Mark file 2 Copying with verified chunk 0
        let f2_id: i64 = {
            let conn = engine.lock_conn();
            conn.query_row(
                "SELECT id FROM files WHERE relative_path = 'file_inflight.txt'",
                [],
                |r| r.get(0),
            )
            .unwrap()
        };
        engine
            .update_file_status(f2_id, FileState::Copying)
            .unwrap();

        let chunk = ChunkRecord {
            file_id: f2_id,
            chunk_index: 0,
            offset_bytes: 0,
            length_bytes: 1000,
            digest: Some("dummy_digest".to_string()),
            state: ChunkState::Verified,
            attempts: 1,
        };
        engine.record_chunk_verified(&chunk).unwrap();
        // Drop engine abruptly simulating SIGKILL
    }

    // Phase 2: Reopen database and load state
    {
        let engine = JournalEngine::open(&db_path).unwrap();
        let recovery_state = engine.load_job_state("job_crash").unwrap();

        assert_eq!(recovery_state.completed_file_count, 1);
        assert_eq!(recovery_state.pending_file_count, 1);
        assert_eq!(recovery_state.interrupted_files.len(), 1);
        assert_eq!(
            recovery_state.interrupted_files[0].relative_path,
            PathBuf::from("file_inflight.txt")
        );

        // Verify chunk 0 remains intact
        let verified_chunks = engine
            .get_verified_chunks(recovery_state.interrupted_files[0].id)
            .unwrap();
        assert_eq!(verified_chunks.len(), 1);
        assert_eq!(verified_chunks[0].chunk_index, 0);
    }
}

/// TEST-JOUR-11: Concurrent Reader-Writer Isolation under WAL
#[test]
fn test_jour_11_concurrent_reader_writer_wal() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("wal_concurrency.db");

    let engine = Arc::new(JournalEngine::open(&db_path).unwrap());
    let job = create_sample_job("job_concurrency");
    engine.create_job(&job).unwrap();

    let writer_engine = Arc::clone(&engine);
    let writer_handle = thread::spawn(move || {
        for i in 0..100 {
            let err = ExtentErrorRecord {
                id: None,
                job_id: "job_concurrency".to_string(),
                file_id: None,
                offset_bytes: (i as u64) * 4096,
                length_bytes: 4096,
                phase: "read".to_string(),
                errno_code: 5,
                error_type: "EIO".to_string(),
                attempts: 1,
                message: format!("Concurrent error {}", i),
                action_taken: None,
                recorded_at: chrono::Utc::now().to_rfc3339(),
            };
            writer_engine.record_extent_error(&err).unwrap();
        }
    });

    let reader_engine = Arc::clone(&engine);
    let reader_handle = thread::spawn(move || {
        let mut total_observed = 0;
        for _ in 0..50 {
            let errors = reader_engine.get_job_errors("job_concurrency").unwrap();
            total_observed += errors.len();
        }
        total_observed
    });

    writer_handle.join().unwrap();
    let read_count = reader_handle.join().unwrap();
    let _ = read_count;

    let final_errors = engine.get_job_errors("job_concurrency").unwrap();
    assert_eq!(final_errors.len(), 100);
}

/// TEST-JOUR-12: WAL Manual Checkpoint Truncation
#[test]
fn test_jour_12_wal_manual_checkpoint_truncate() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("wal_checkpoint.db");
    let wal_path = dir.path().join("wal_checkpoint.db-wal");

    let engine = JournalEngine::open(&db_path).unwrap();
    let job = create_sample_job("job_wal_cp");
    engine.create_job(&job).unwrap();

    // Write multiple records to produce WAL frames
    for i in 0..50 {
        let file = create_sample_file("job_wal_cp", &format!("file_{}.bin", i), 1024);
        engine.record_files_batch("job_wal_cp", &[file]).unwrap();
    }

    // Checkpoint with Truncate
    let (busy_or_log, checkpointed) = engine.checkpoint(CheckpointMode::Truncate).unwrap();
    assert!(checkpointed >= 0);
    assert!(busy_or_log >= 0);

    // If WAL file exists, it should be 0 bytes or cleaned up
    if wal_path.exists() {
        let wal_len = std::fs::metadata(&wal_path).unwrap().len();
        assert_eq!(wal_len, 0);
    }
}

/// Test crash recovery reconciliation with truncated partial file on disk
#[test]
fn test_crash_recovery_reconciliation_truncated_partial() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("recon.db");
    let target_root = dir.path().join("target");
    std::fs::create_dir_all(&target_root).unwrap();

    let engine = JournalEngine::open(&db_path).unwrap();
    let mut job = create_sample_job("job_recon");
    job.state = JobState::Running;
    job.target_root = target_root.clone();
    engine.create_job(&job).unwrap();

    let file = create_sample_file("job_recon", "video.mp4", 3 * 1024 * 1024);
    engine.record_files_batch("job_recon", &[file]).unwrap();

    let file_id: i64 = {
        let conn = engine.lock_conn();
        conn.query_row("SELECT id FROM files WHERE job_id = 'job_recon'", [], |r| {
            r.get(0)
        })
        .unwrap()
    };
    engine
        .update_file_status(file_id, FileState::Copying)
        .unwrap();

    // Create 3 chunks in DB (1 MiB each)
    let c0 = ChunkRecord {
        file_id,
        chunk_index: 0,
        offset_bytes: 0,
        length_bytes: 1024 * 1024,
        digest: None,
        state: ChunkState::Verified,
        attempts: 1,
    };
    let c1 = ChunkRecord {
        file_id,
        chunk_index: 1,
        offset_bytes: 1024 * 1024,
        length_bytes: 1024 * 1024,
        digest: None,
        state: ChunkState::Verified,
        attempts: 1,
    };
    let c2 = ChunkRecord {
        file_id,
        chunk_index: 2,
        offset_bytes: 2 * 1024 * 1024,
        length_bytes: 1024 * 1024,
        digest: None,
        state: ChunkState::Verified,
        attempts: 1,
    };
    engine.record_chunk_verified(&c0).unwrap();
    engine.record_chunk_verified(&c1).unwrap();
    engine.record_chunk_verified(&c2).unwrap();

    // On disk, create partial file truncated to 2.5 MiB (so chunk 2 is incomplete!)
    let partial_path = target_root.join(".volterra.video.mp4.partial");
    {
        let mut f = File::create(&partial_path).unwrap();
        let data = vec![0u8; (2.5 * 1024.0 * 1024.0) as usize];
        f.write_all(&data).unwrap();
    }

    // Run reconciliation
    let report = engine
        .reconcile_job_after_crash("job_recon", &target_root, false)
        .unwrap();

    assert_eq!(report.reconciled_files_count, 1);
    assert_eq!(report.reverted_chunks_count, 1); // Chunk 2 was truncated, so reverted!
    assert_eq!(report.verified_chunks_count, 2); // Chunks 0 & 1 remain verified
    assert_eq!(report.recovered_bytes, 2 * 1024 * 1024);

    // Verify job transitioned to Paused
    let job_state = engine.get_job("job_recon").unwrap().unwrap().state;
    assert_eq!(job_state, JobState::Paused);

    // Verify file transitioned to Partial
    let file_state: String = {
        let conn = engine.lock_conn();
        conn.query_row("SELECT state FROM files WHERE id = ?1", [file_id], |r| {
            r.get(0)
        })
        .unwrap()
    };
    assert_eq!(file_state, "Partial");
}

/// Test resumption chunk reuse and missing extent planning
#[test]
fn test_resumption_chunk_reuse_planning() {
    let engine = JournalEngine::open_in_memory().unwrap();
    let job = create_sample_job("job_plan");
    engine.create_job(&job).unwrap();

    let file = create_sample_file("job_plan", "archive.tar", 10 * 1024 * 1024); // 10 MiB
    engine.record_files_batch("job_plan", &[file]).unwrap();

    let file_id: i64 = {
        let conn = engine.lock_conn();
        conn.query_row("SELECT id FROM files WHERE job_id = 'job_plan'", [], |r| {
            r.get(0)
        })
        .unwrap()
    };

    // Verify chunks 0 (0-2MB) and 2 (4-6MB), leaving missing (2-4MB) and (6-10MB)
    let c0 = ChunkRecord {
        file_id,
        chunk_index: 0,
        offset_bytes: 0,
        length_bytes: 2 * 1024 * 1024,
        digest: None,
        state: ChunkState::Verified,
        attempts: 1,
    };
    let c2 = ChunkRecord {
        file_id,
        chunk_index: 2,
        offset_bytes: 4 * 1024 * 1024,
        length_bytes: 2 * 1024 * 1024,
        digest: None,
        state: ChunkState::Verified,
        attempts: 1,
    };
    engine.record_chunk_verified(&c0).unwrap();
    engine.record_chunk_verified(&c2).unwrap();

    let plan = engine.get_resumption_plan(file_id).unwrap();
    assert_eq!(plan.total_size, 10 * 1024 * 1024);
    assert_eq!(plan.already_verified_bytes, 4 * 1024 * 1024);
    assert_eq!(plan.remaining_bytes, 6 * 1024 * 1024);

    assert_eq!(plan.verified_extents.len(), 2);
    assert_eq!(plan.missing_extents.len(), 2);

    assert_eq!(plan.missing_extents[0], (2 * 1024 * 1024, 2 * 1024 * 1024));
    assert_eq!(plan.missing_extents[1], (6 * 1024 * 1024, 4 * 1024 * 1024));
}

/// Test JSON and CSV report export
#[test]
fn test_json_and_csv_report_export() {
    let engine = JournalEngine::open_in_memory().unwrap();
    let job = create_sample_job("job_export");
    engine.create_job(&job).unwrap();

    let f1 = create_sample_file("job_export", "photo.jpg", 2048);
    let f2 = create_sample_file("job_export", "doc.pdf", 4096);
    engine.record_files_batch("job_export", &[f1, f2]).unwrap();

    let json_report = engine.export_job_report_json("job_export").unwrap();
    assert!(json_report.contains("\"id\": \"job_export\""));
    assert!(json_report.contains("\"relative_path\": \"photo.jpg\""));
    assert!(json_report.contains("\"relative_path\": \"doc.pdf\""));

    let csv_report = engine.export_job_report_csv("job_export").unwrap();
    assert!(csv_report
        .starts_with("id,relative_path,source_size,state,digest,recovered_bytes,error_code\n"));
    assert!(csv_report.contains("\"photo.jpg\",2048"));
    assert!(csv_report.contains("\"doc.pdf\",4096"));
}

/// Test skip all errors bulk remediation
#[test]
fn test_skip_all_errors_bulk() {
    let engine = JournalEngine::open_in_memory().unwrap();
    let job = create_sample_job("job_bulk_skip");
    engine.create_job(&job).unwrap();

    let f1 = create_sample_file("job_bulk_skip", "bad1.bin", 1000);
    let f2 = create_sample_file("job_bulk_skip", "bad2.bin", 2000);
    engine
        .record_files_batch("job_bulk_skip", &[f1, f2])
        .unwrap();

    let (f1_id, f2_id) = {
        let conn = engine.lock_conn();
        let id1: i64 = conn
            .query_row(
                "SELECT id FROM files WHERE relative_path = 'bad1.bin'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let id2: i64 = conn
            .query_row(
                "SELECT id FROM files WHERE relative_path = 'bad2.bin'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        (id1, id2)
    };

    engine
        .update_file_status(f1_id, FileState::ReadError)
        .unwrap();
    engine
        .update_file_status(f2_id, FileState::ReadError)
        .unwrap();

    let err1 = ExtentErrorRecord {
        id: None,
        job_id: "job_bulk_skip".to_string(),
        file_id: Some(f1_id),
        offset_bytes: 0,
        length_bytes: 512,
        phase: "read".to_string(),
        errno_code: 5,
        error_type: "EIO".to_string(),
        attempts: 1,
        message: "read error 1".to_string(),
        action_taken: None,
        recorded_at: chrono::Utc::now().to_rfc3339(),
    };
    let err2 = ExtentErrorRecord {
        id: None,
        job_id: "job_bulk_skip".to_string(),
        file_id: Some(f2_id),
        offset_bytes: 512,
        length_bytes: 512,
        phase: "read".to_string(),
        errno_code: 5,
        error_type: "EIO".to_string(),
        attempts: 1,
        message: "read error 2".to_string(),
        action_taken: None,
        recorded_at: chrono::Utc::now().to_rfc3339(),
    };
    engine.record_extent_error(&err1).unwrap();
    engine.record_extent_error(&err2).unwrap();

    let skipped_count = engine.skip_all_errors("job_bulk_skip").unwrap();
    assert_eq!(skipped_count, 2);

    // Verify files transitioned to Skipped
    let conn = engine.lock_conn();
    let s1: String = conn
        .query_row("SELECT state FROM files WHERE id = ?1", [f1_id], |r| {
            r.get(0)
        })
        .unwrap();
    let s2: String = conn
        .query_row("SELECT state FROM files WHERE id = ?1", [f2_id], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(s1, "Skipped");
    assert_eq!(s2, "Skipped");
}
