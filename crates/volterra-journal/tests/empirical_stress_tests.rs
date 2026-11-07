use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Instant;
use tempfile::tempdir;
use volterra_integrity::blake3::{Blake3Hasher, HashDigest};
use volterra_journal::{
    ChunkRecord, ChunkState, ExtentErrorRecord, FileRecord, FileState, JobMode, JobRecord,
    JobState, JournalEngine,
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
        source_file_id: format!("16777220:{}", relative_path.len() * 1000 + 42),
        temp_path: Some(PathBuf::from(format!(
            ".volterra.{}.partial",
            relative_path.replace('/', "_")
        ))),
        state: FileState::Pending,
        verified_digest: None,
        error_code: None,
        recovered_bytes: 0,
    }
}

// ============================================================================
// STRESS TEST 1: WAL Crash Recovery Simulation
// - Rollback of uncommitted transactions during active writes
// - Preservation of committed chunk records across restarts
// - Disk truncation & bit-rot recovery via reconcile_job_after_crash
// ============================================================================
#[test]
fn test_stress_wal_crash_rollback_uncommitted_and_preserve_committed() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("crash_durability.db");
    let target_root = dir.path().join("target");
    std::fs::create_dir_all(&target_root).unwrap();

    let job_id = "job_crash_durability";
    let chunk_size = 1024 * 1024; // 1 MiB

    // --- PHASE 1: Write committed state ---
    {
        let engine = JournalEngine::open(&db_path).unwrap();
        let mut job = create_sample_job(job_id);
        job.state = JobState::Running;
        job.target_root = target_root.clone();
        engine.create_job(&job).unwrap();

        let f1 = create_sample_file(job_id, "committed_file_1.bin", 5 * 1024 * 1024);
        let f2 = create_sample_file(job_id, "committed_file_2.bin", 5 * 1024 * 1024);
        let f3 = create_sample_file(job_id, "uncommitted_file_3.bin", 5 * 1024 * 1024);
        engine.record_files_batch(job_id, &[f1, f2, f3]).unwrap();

        let (f1_id, f2_id, f3_id) = {
            let conn = engine.lock_conn();
            let id1: i64 = conn
                .query_row(
                    "SELECT id FROM files WHERE relative_path = 'committed_file_1.bin'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            let id2: i64 = conn
                .query_row(
                    "SELECT id FROM files WHERE relative_path = 'committed_file_2.bin'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            let id3: i64 = conn
                .query_row(
                    "SELECT id FROM files WHERE relative_path = 'uncommitted_file_3.bin'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            (id1, id2, id3)
        };

        // Commit 5 verified chunks for f1
        for c_idx in 0..5 {
            let chunk = ChunkRecord {
                file_id: f1_id,
                chunk_index: c_idx,
                offset_bytes: (c_idx as u64) * (chunk_size as u64),
                length_bytes: chunk_size as u32,
                digest: Some(format!("digest_f1_{}", c_idx)),
                state: ChunkState::Verified,
                attempts: 1,
            };
            engine.record_chunk_verified(&chunk).unwrap();
        }

        // Commit 5 verified chunks for f2
        for c_idx in 0..5 {
            let chunk = ChunkRecord {
                file_id: f2_id,
                chunk_index: c_idx,
                offset_bytes: (c_idx as u64) * (chunk_size as u64),
                length_bytes: chunk_size as u32,
                digest: Some(format!("digest_f2_{}", c_idx)),
                state: ChunkState::Verified,
                attempts: 1,
            };
            engine.record_chunk_verified(&chunk).unwrap();
        }

        // Mark f1 and f2 as Copying/Partial
        engine
            .update_file_status(f1_id, FileState::Copying)
            .unwrap();
        engine
            .update_file_status(f2_id, FileState::Copying)
            .unwrap();

        // Verify committed state
        assert_eq!(engine.get_verified_chunks(f1_id).unwrap().len(), 5);
        assert_eq!(engine.get_verified_chunks(f2_id).unwrap().len(), 5);

        // --- Simulate crash during active uncommitted write ---
        // Open an explicit immediate transaction, insert uncommitted writes, then DROP without commit
        {
            let mut conn = engine.lock_conn();
            let tx = conn
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .unwrap();

            // Insert 5 uncommitted chunks for f3
            for c_idx in 0..5 {
                tx.execute(
                    "INSERT INTO chunks (file_id, chunk_index, offset_bytes, length_bytes, digest, state, attempts)
                     VALUES (?1, ?2, ?3, ?4, ?5, 'Verified', 1)",
                    rusqlite::params![
                        f3_id,
                        c_idx,
                        (c_idx as u64) * (chunk_size as u64),
                        chunk_size,
                        format!("uncommitted_digest_{}", c_idx),
                    ],
                )
                .unwrap();
            }

            // Insert uncommitted error records
            for err_idx in 0..50 {
                tx.execute(
                    "INSERT INTO errors (job_id, file_id, offset_bytes, length_bytes, phase, errno_code, error_type, message, attempts, recorded_at)
                     VALUES (?1, ?2, ?3, 4096, 'read', 5, 'EIO', 'Uncommitted error', 1, ?4)",
                    rusqlite::params![
                        job_id,
                        f3_id,
                        err_idx * 4096,
                        chrono::Utc::now().to_rfc3339(),
                    ],
                )
                .unwrap();
            }

            // ABRUPT CRASH: Explicitly rollback or drop transaction without commit!
            // When tx drops without tx.commit(), it triggers rollback in SQLite WAL
            let _ = tx.rollback();
        }

        // Drop the engine abruptly simulating SIGKILL / power cutoff
        drop(engine);
    }

    // --- PHASE 2: Reopen database and verify uncommitted writes rolled back ---
    {
        let engine = JournalEngine::open(&db_path).unwrap();

        {
            let conn = engine.lock_conn();

            // 1. Verify uncommitted file 3 has 0 chunks and 0 errors
            let f3_chunks_count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM chunks c JOIN files f ON c.file_id = f.id WHERE f.relative_path = 'uncommitted_file_3.bin'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(
                f3_chunks_count, 0,
                "CRITICAL: Uncommitted chunks must be rolled back completely upon crash!"
            );

            let uncommitted_errors_count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM errors WHERE job_id = ?1",
                    [job_id],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(
                uncommitted_errors_count, 0,
                "CRITICAL: Uncommitted error records must be rolled back completely upon crash!"
            );

            // 2. Verify committed files have all 5 chunks preserved intact
            let f1_chunks_count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM chunks c JOIN files f ON c.file_id = f.id WHERE f.relative_path = 'committed_file_1.bin' AND c.state = 'Verified'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(
                f1_chunks_count, 5,
                "Committed chunks for file 1 must be preserved intact"
            );

            let f2_chunks_count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM chunks c JOIN files f ON c.file_id = f.id WHERE f.relative_path = 'committed_file_2.bin' AND c.state = 'Verified'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(
                f2_chunks_count, 5,
                "Committed chunks for file 2 must be preserved intact"
            );
        }

        // 3. Verify total copied bytes match committed chunks (10 * 1 MiB = 10,485,760 bytes)
        let job = engine.get_job(job_id).unwrap().unwrap();
        assert_eq!(job.copied_bytes, 10 * 1024 * 1024);
    }

    // --- PHASE 3: Physical Disk Crash Simulation with Partial Truncation & Bit-Rot ---
    {
        let engine = JournalEngine::open(&db_path).unwrap();

        let (f1_id, f2_id) = {
            let conn = engine.lock_conn();
            let id1: i64 = conn
                .query_row(
                    "SELECT id FROM files WHERE relative_path = 'committed_file_1.bin'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            let id2: i64 = conn
                .query_row(
                    "SELECT id FROM files WHERE relative_path = 'committed_file_2.bin'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            (id1, id2)
        };

        // Prepare physical partial files on disk:
        // File 1: partial file truncated to 3.5 MiB on disk (chunks 0, 1, 2 full; chunk 3 partial; chunk 4 missing)
        let partial_f1 = target_root.join(".volterra.committed_file_1.bin.partial");
        let f1_data = vec![0xAAu8; (3.5 * 1024.0 * 1024.0) as usize];
        {
            let mut f = File::create(&partial_f1).unwrap();
            f.write_all(&f1_data).unwrap();
            f.sync_all().unwrap();
        }

        // Set genuine BLAKE3 digests for f1 chunks
        for c_idx in 0..5 {
            let digest_hex = if c_idx < 3 {
                let start = (c_idx as usize) * chunk_size;
                let end = start + chunk_size;
                let digest_bytes = Blake3Hasher::hash_chunk(&f1_data[start..end]);
                HashDigest(digest_bytes).to_hex()
            } else {
                format!("expected_f1_chunk_{}", c_idx)
            };

            let chunk = ChunkRecord {
                file_id: f1_id,
                chunk_index: c_idx,
                offset_bytes: (c_idx as u64) * (chunk_size as u64),
                length_bytes: chunk_size as u32,
                digest: Some(digest_hex),
                state: ChunkState::Verified,
                attempts: 1,
            };
            engine.record_chunk_verified(&chunk).unwrap();
        }

        // File 2: partial file is 5 MiB on disk with genuine BLAKE3 hashes for chunks 0, 1, 2, 3, 4
        // BUT chunk 2 has corrupted bytes (bit rot on crash)
        let partial_f2 = target_root.join(".volterra.committed_file_2.bin.partial");
        let mut f2_bytes = vec![0u8; 5 * chunk_size];
        for i in 0..5 {
            let start = i * chunk_size;
            let end = start + chunk_size;
            let byte_val = (i + 1) as u8;
            f2_bytes[start..end].fill(byte_val);
        }

        // Update chunks for f2 with their REAL BLAKE3 digests
        for c_idx in 0..5 {
            let start = (c_idx as usize) * chunk_size;
            let end = start + chunk_size;
            let digest_bytes = Blake3Hasher::hash_chunk(&f2_bytes[start..end]);
            let digest_hex = HashDigest(digest_bytes).to_hex();

            let chunk = ChunkRecord {
                file_id: f2_id,
                chunk_index: c_idx,
                offset_bytes: (c_idx as u64) * (chunk_size as u64),
                length_bytes: chunk_size as u32,
                digest: Some(digest_hex),
                state: ChunkState::Verified,
                attempts: 1,
            };
            engine.record_chunk_verified(&chunk).unwrap();
        }

        // Corrupt chunk 2 in memory/disk: flip bytes at 2 MiB offset
        f2_bytes[2 * chunk_size..2 * chunk_size + 128].fill(0xFF);
        {
            let mut f = File::create(&partial_f2).unwrap();
            f.write_all(&f2_bytes).unwrap();
            f.sync_all().unwrap();
        }

        // Run crash reconciliation with digest verification
        let report = engine
            .reconcile_job_after_crash(job_id, &target_root, true)
            .unwrap();

        assert_eq!(report.job_id, job_id);
        // File 1 has 2 reverted chunks (3 and 4 truncated beyond 3.5 MiB)
        // File 2 has 1 reverted chunk (chunk 2 corrupted digest)
        assert_eq!(
            report.reverted_chunks_count, 3,
            "Must revert exactly 3 chunks (2 truncated in f1, 1 corrupted in f2)"
        );
        // File 1 has 3 surviving verified chunks (0, 1, 2)
        // File 2 has 4 surviving verified chunks (0, 1, 3, 4)
        assert_eq!(
            report.verified_chunks_count, 7,
            "Must preserve exactly 7 verified chunks (3 in f1, 4 in f2)"
        );
        assert_eq!(report.recovered_bytes, 7 * (chunk_size as u64));

        // Verify job transitioned to Paused
        let job_after = engine.get_job(job_id).unwrap().unwrap();
        assert_eq!(job_after.state, JobState::Paused);
        assert_eq!(job_after.copied_bytes, 7 * (chunk_size as u64));
    }
}

// ============================================================================
// STRESS TEST 2: Large-Scale Batch Throughput (10,000 files)
// - Batch insertion speed
// - Query responsiveness & indexed search latency
// - Memory stability & report generation
// ============================================================================
#[test]
fn test_stress_large_scale_10000_file_batch_throughput() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("large_batch_10k.db");

    let engine = JournalEngine::open(&db_path).unwrap();
    let job_id = "job_10k_stress";
    let mut job = create_sample_job(job_id);
    job.state = JobState::Scanning;
    engine.create_job(&job).unwrap();

    let total_file_count: usize = 10_000;
    let mut files = Vec::with_capacity(total_file_count);
    let mut expected_total_bytes: u64 = 0;

    println!("Generating 10,000 synthetic FileRecords...");
    for i in 0..total_file_count {
        let dir_idx = i / 100;
        let sub_idx = (i % 100) / 10;
        let rel_path = format!("dir_{:03}/sub_{:02}/file_{:05}.dat", dir_idx, sub_idx, i);
        // Varying sizes: 0B, 1KB, 64KB, 1MB, 16MB
        let size = match i % 5 {
            0 => 0,
            1 => 1024,
            2 => 65536,
            3 => 1024 * 1024,
            _ => 16 * 1024 * 1024,
        };
        expected_total_bytes = expected_total_bytes.saturating_add(size);
        files.push(create_sample_file(job_id, &rel_path, size));
    }

    // Measure batch insertion execution speed
    let insert_start = Instant::now();
    engine.record_files_batch(job_id, &files).unwrap();
    let insert_duration = insert_start.elapsed();
    let files_per_sec = (total_file_count as f64) / insert_duration.as_secs_f64();

    println!(
        "Batch insertion: 10,000 files inserted in {:?} ({:.2} files/sec)",
        insert_duration, files_per_sec
    );

    // Verify insertion speed: 10,000 files in a single immediate SQLite transaction should easily exceed 2,000 files/sec
    assert!(
        insert_duration.as_secs() < 10,
        "10,000 file batch insertion took too long: {:?}",
        insert_duration
    );

    // Verify atomic job counters
    let updated_job = engine.get_job(job_id).unwrap().expect("Job must exist");
    assert_eq!(updated_job.total_files, total_file_count as u64);
    assert_eq!(updated_job.total_bytes, expected_total_bytes);

    // Verify raw row count in database
    let db_count: u64 = {
        let conn = engine.lock_conn();
        conn.query_row(
            "SELECT COUNT(*) FROM files WHERE job_id = ?1",
            [job_id],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!(db_count, total_file_count as u64);

    // Query Responsiveness: Measure latency of 500 random path lookups
    println!("Benchmarking query responsiveness for 500 indexed random lookups...");
    let query_start = Instant::now();
    let mut total_lookup_ns = 0u128;
    for sample in 0..500 {
        let idx = (sample * 19 + 7) % total_file_count;
        let dir_idx = idx / 100;
        let sub_idx = (idx % 100) / 10;
        let rel_path = format!("dir_{:03}/sub_{:02}/file_{:05}.dat", dir_idx, sub_idx, idx);

        let t0 = Instant::now();
        let conn = engine.lock_conn();
        let found_size: u64 = conn
            .query_row(
                "SELECT source_size FROM files WHERE job_id = ?1 AND relative_path = ?2",
                rusqlite::params![job_id, rel_path],
                |r| r.get(0),
            )
            .unwrap();
        total_lookup_ns += t0.elapsed().as_nanos();
        assert_eq!(found_size, files[idx].source_size);
    }
    let query_duration = query_start.elapsed();
    let avg_lookup_micros = (total_lookup_ns / 500) as f64 / 1_000.0;
    println!(
        "500 lookups completed in {:?} (avg: {:.2} µs/lookup)",
        query_duration, avg_lookup_micros
    );

    // Each indexed lookup should be sub-millisecond (< 1000 µs)
    assert!(
        avg_lookup_micros < 1000.0,
        "Average indexed query latency too high: {:.2} µs",
        avg_lookup_micros
    );

    // Measure load_job_state on 10,000 files
    let load_start = Instant::now();
    let job_state = engine.load_job_state(job_id).unwrap();
    let load_duration = load_start.elapsed();
    println!("load_job_state on 10,000 files took {:?}", load_duration);
    assert_eq!(job_state.pending_file_count, 10_000);
    assert_eq!(job_state.completed_file_count, 0);
    assert_eq!(job_state.interrupted_files.len(), 0);
    assert!(
        load_duration.as_millis() < 500,
        "load_job_state took > 500ms"
    );

    // Measure CSV report generation speed
    let csv_start = Instant::now();
    let csv = engine.export_job_report_csv(job_id).unwrap();
    let csv_duration = csv_start.elapsed();
    println!("CSV export of 10,000 files took {:?}", csv_duration);
    let line_count = csv.lines().count();
    // 1 header + 10,000 file rows = 10,001 lines
    assert_eq!(line_count, 10_001);
}

// ============================================================================
// STRESS TEST 3: Concurrent Reader/Writer Stress under WAL Mode
// - Multi-threaded reader and writer execution
// - Multiple independent SQLite connections to same database file
// - 0 database lock errors (busy_timeout)
// ============================================================================
#[test]
fn test_stress_concurrent_reader_writer_wal_isolation() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("wal_concurrency_stress.db");

    // Initialize database and schema with seed file
    let job_id = "job_concurrency_stress";
    {
        let engine = JournalEngine::open(&db_path).unwrap();
        let mut job = create_sample_job(job_id);
        job.state = JobState::Running;
        engine.create_job(&job).unwrap();

        let mut files = Vec::new();
        for i in 0..100 {
            files.push(create_sample_file(
                job_id,
                &format!("file_{:03}.dat", i),
                10 * 1024 * 1024,
            ));
        }
        engine.record_files_batch(job_id, &files).unwrap();
    }

    let is_writer_done = Arc::new(AtomicBool::new(false));
    let total_busy_errors = Arc::new(AtomicUsize::new(0));
    let total_successful_reads = Arc::new(AtomicUsize::new(0));
    let total_successful_writes = Arc::new(AtomicUsize::new(0));
    let last_observed_copied_bytes = Arc::new(AtomicU64::new(0));

    // Spawn 1 Writer Thread (commits 400 chunk completions across files)
    let writer_db = db_path.clone();
    let writer_done_flag = Arc::clone(&is_writer_done);
    let writer_busy_errs = Arc::clone(&total_busy_errors);
    let writer_successes = Arc::clone(&total_successful_writes);

    let writer_handle = thread::spawn(move || {
        let engine = match JournalEngine::open(&writer_db) {
            Ok(e) => e,
            Err(e) => {
                eprintln!("Writer engine open error: {:?}", e);
                return;
            }
        };

        // Query file IDs
        let file_ids: Vec<i64> = {
            let conn = engine.lock_conn();
            let mut stmt = conn
                .prepare("SELECT id FROM files WHERE job_id = ?1 ORDER BY id ASC")
                .unwrap();
            let rows = stmt
                .query_map([job_id], |r| r.get(0))
                .unwrap()
                .map(|r| r.unwrap())
                .collect();
            rows
        };

        let chunk_size = 64 * 1024; // 64 KiB
        for write_iter in 0..400 {
            let file_id = file_ids[write_iter % file_ids.len()];
            let chunk_idx = (write_iter / file_ids.len()) as u32;

            let chunk = ChunkRecord {
                file_id,
                chunk_index: chunk_idx,
                offset_bytes: (chunk_idx as u64) * (chunk_size as u64),
                length_bytes: chunk_size,
                digest: Some(format!("digest_{}_{}", file_id, chunk_idx)),
                state: ChunkState::Verified,
                attempts: 1,
            };

            match engine.record_chunk_verified(&chunk) {
                Ok(_) => {
                    writer_successes.fetch_add(1, Ordering::Relaxed);
                }
                Err(e) => {
                    eprintln!("Writer error on iter {}: {:?}", write_iter, e);
                    writer_busy_errs.fetch_add(1, Ordering::Relaxed);
                }
            }

            // Also occasionally log an extent error
            if write_iter % 40 == 0 {
                let err = ExtentErrorRecord {
                    id: None,
                    job_id: job_id.to_string(),
                    file_id: Some(file_id),
                    offset_bytes: 0,
                    length_bytes: 4096,
                    phase: "read".to_string(),
                    errno_code: 5,
                    error_type: "EIO".to_string(),
                    attempts: 1,
                    message: format!("Transient read error at {}", write_iter),
                    action_taken: None,
                    recorded_at: chrono::Utc::now().to_rfc3339(),
                };
                let _ = engine.record_extent_error(&err);
            }
        }

        writer_done_flag.store(true, Ordering::SeqCst);
    });

    // Spawn 4 Reader Threads (simultaneously polling job state, snapshot, and chunk status)
    let num_readers = 4;
    let mut reader_handles = Vec::new();

    for reader_id in 0..num_readers {
        let reader_db = db_path.clone();
        let writer_done_ref = Arc::clone(&is_writer_done);
        let reader_busy_errs = Arc::clone(&total_busy_errors);
        let reader_successes = Arc::clone(&total_successful_reads);
        let last_bytes_ref = Arc::clone(&last_observed_copied_bytes);

        let handle = thread::spawn(move || {
            let engine = match JournalEngine::open(&reader_db) {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("Reader {} engine open error: {:?}", reader_id, e);
                    return;
                }
            };

            let mut reader_ops = 0;
            while !writer_done_ref.load(Ordering::Relaxed) || reader_ops < 150 {
                reader_ops += 1;

                // Op 1: get_job
                match engine.get_job(job_id) {
                    Ok(Some(job)) => {
                        reader_successes.fetch_add(1, Ordering::Relaxed);
                        // Monotonicity check
                        let prev = last_bytes_ref.load(Ordering::Relaxed);
                        if job.copied_bytes >= prev {
                            last_bytes_ref.store(job.copied_bytes, Ordering::Relaxed);
                        }
                    }
                    Ok(None) => {}
                    Err(e) => {
                        eprintln!("Reader {} get_job error: {:?}", reader_id, e);
                        reader_busy_errs.fetch_add(1, Ordering::Relaxed);
                    }
                }

                // Op 2: load_job_state
                match engine.load_job_state(job_id) {
                    Ok(_) => {
                        reader_successes.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(e) => {
                        eprintln!("Reader {} load_job_state error: {:?}", reader_id, e);
                        reader_busy_errs.fetch_add(1, Ordering::Relaxed);
                    }
                }

                // Op 3: get_job_errors
                match engine.get_job_errors(job_id) {
                    Ok(_) => {
                        reader_successes.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(e) => {
                        eprintln!("Reader {} get_job_errors error: {:?}", reader_id, e);
                        reader_busy_errs.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        });
        reader_handles.push(handle);
    }

    // Wait for all threads to complete
    writer_handle.join().unwrap();
    for h in reader_handles {
        h.join().unwrap();
    }

    let busy_count = total_busy_errors.load(Ordering::SeqCst);
    let writes = total_successful_writes.load(Ordering::SeqCst);
    let reads = total_successful_reads.load(Ordering::SeqCst);

    println!(
        "Concurrent WAL Stress: {} successful writes, {} successful reads, {} busy/lock errors",
        writes, reads, busy_count
    );

    assert_eq!(writes, 400, "All 400 writes must succeed without error");
    assert!(reads > 600, "Readers should have executed > 600 queries");
    assert_eq!(
        busy_count, 0,
        "CRITICAL: Zero database lock / busy_timeout errors allowed under SQLite WAL mode!"
    );

    // Verify final database consistency
    let engine = JournalEngine::open(&db_path).unwrap();
    let final_job = engine.get_job(job_id).unwrap().unwrap();
    let expected_copied = 400_u64 * (64 * 1024);
    assert_eq!(final_job.copied_bytes, expected_copied);
}

// ============================================================================
// STRESS TEST 4: Resumption Chunk Plan Accuracy (50 of 100 Chunks)
// - Exactly 50 chunks marked verified out of 100 chunks
// - Alternating, front-half, back-half, and irregular residue patterns
// - Verify exact 50 unverified chunks returned in order
// - Verify missing extents mathematically partition missing regions
// ============================================================================
#[test]
fn test_stress_resumption_chunk_plan_accuracy_50_of_100() {
    let engine = JournalEngine::open_in_memory().unwrap();
    let job_id = "job_resumption_50_100";
    let job = create_sample_job(job_id);
    engine.create_job(&job).unwrap();

    let chunk_size: u32 = 1024 * 1024; // 1 MiB per chunk
    let total_chunks: u32 = 100;
    let file_size: u64 = (total_chunks as u64) * (chunk_size as u64); // 100 MiB

    // Pattern A: Alternating Chunks (Even chunks 0, 2, 4... 98 verified; Odd chunks 1, 3, 5... 99 pending)
    let f_alt = create_sample_file(job_id, "alternating_100mb.bin", file_size);
    engine.record_files_batch(job_id, &[f_alt]).unwrap();

    let f_alt_id: i64 = {
        let conn = engine.lock_conn();
        conn.query_row(
            "SELECT id FROM files WHERE relative_path = 'alternating_100mb.bin'",
            [],
            |r| r.get(0),
        )
        .unwrap()
    };

    // Seed all 100 chunks into the chunks table:
    // Chunks with even index -> Verified; Chunks with odd index -> Pending
    let mut chunks = Vec::with_capacity(total_chunks as usize);
    for idx in 0..total_chunks {
        let is_even = idx % 2 == 0;
        chunks.push(ChunkRecord {
            file_id: f_alt_id,
            chunk_index: idx,
            offset_bytes: (idx as u64) * (chunk_size as u64),
            length_bytes: chunk_size,
            digest: if is_even {
                Some(format!("digest_{}", idx))
            } else {
                None
            },
            state: if is_even {
                ChunkState::Verified
            } else {
                ChunkState::Pending
            },
            attempts: if is_even { 1 } else { 0 },
        });
    }
    engine.record_chunks_batch(&chunks).unwrap();

    // Verification 1: get_unverified_chunks returns exactly 50 chunks in strictly ascending order
    let unverified_chunks = engine.get_unverified_chunks(f_alt_id).unwrap();
    assert_eq!(
        unverified_chunks.len(),
        50,
        "Must return exactly 50 unverified chunks"
    );

    for (pos, chunk) in unverified_chunks.iter().enumerate() {
        let expected_index = (pos as u32) * 2 + 1; // 1, 3, 5, ..., 99
        assert_eq!(
            chunk.chunk_index, expected_index,
            "Unverified chunk index mismatch at position {}",
            pos
        );
        assert_eq!(
            chunk.offset_bytes,
            (expected_index as u64) * (chunk_size as u64)
        );
        assert_eq!(chunk.length_bytes, chunk_size);
        assert_eq!(chunk.state, ChunkState::Pending);
    }

    // Verification 2: get_resumption_plan returns exact missing extents
    let plan = engine.get_resumption_plan(f_alt_id).unwrap();
    assert_eq!(plan.total_size, file_size);
    assert_eq!(
        plan.already_verified_bytes,
        50 * (chunk_size as u64),
        "Already verified bytes must equal exactly 50 MiB"
    );
    assert_eq!(
        plan.remaining_bytes,
        50 * (chunk_size as u64),
        "Remaining bytes must equal exactly 50 MiB"
    );
    assert_eq!(
        plan.already_verified_bytes + plan.remaining_bytes,
        plan.total_size,
        "Verified + Remaining must strictly equal total_size"
    );

    assert_eq!(plan.verified_extents.len(), 50);
    assert_eq!(plan.missing_extents.len(), 50);

    for i in 0..50 {
        let expected_offset = ((2 * i + 1) as u64) * (chunk_size as u64);
        assert_eq!(
            plan.missing_extents[i],
            (expected_offset, chunk_size as u64),
            "Missing extent {} offset/length mismatch",
            i
        );
    }

    // Pattern B: Contiguous Front Half Verified (chunks 0..49 verified, 50..99 pending)
    let f_front = create_sample_file(job_id, "front_half_100mb.bin", file_size);
    engine.record_files_batch(job_id, &[f_front]).unwrap();
    let f_front_id: i64 = {
        let conn = engine.lock_conn();
        conn.query_row(
            "SELECT id FROM files WHERE relative_path = 'front_half_100mb.bin'",
            [],
            |r| r.get(0),
        )
        .unwrap()
    };

    let mut front_chunks = Vec::with_capacity(total_chunks as usize);
    for idx in 0..total_chunks {
        let is_verified = idx < 50;
        front_chunks.push(ChunkRecord {
            file_id: f_front_id,
            chunk_index: idx,
            offset_bytes: (idx as u64) * (chunk_size as u64),
            length_bytes: chunk_size,
            digest: if is_verified {
                Some(format!("digest_{}", idx))
            } else {
                None
            },
            state: if is_verified {
                ChunkState::Verified
            } else {
                ChunkState::Pending
            },
            attempts: if is_verified { 1 } else { 0 },
        });
    }
    engine.record_chunks_batch(&front_chunks).unwrap();

    let front_unverified = engine.get_unverified_chunks(f_front_id).unwrap();
    assert_eq!(front_unverified.len(), 50);
    assert_eq!(front_unverified.first().unwrap().chunk_index, 50);
    assert_eq!(front_unverified.last().unwrap().chunk_index, 99);

    let front_plan = engine.get_resumption_plan(f_front_id).unwrap();
    // Because missing region is contiguous from 50 MiB to 100 MiB, it should coalesce into 1 single missing extent
    assert_eq!(front_plan.missing_extents.len(), 1);
    assert_eq!(
        front_plan.missing_extents[0],
        (50 * (chunk_size as u64), 50 * (chunk_size as u64))
    );
    assert_eq!(front_plan.already_verified_bytes, 50 * (chunk_size as u64));
    assert_eq!(front_plan.remaining_bytes, 50 * (chunk_size as u64));

    // Pattern C: Contiguous Back Half Verified (chunks 50..99 verified, 0..49 pending)
    let f_back = create_sample_file(job_id, "back_half_100mb.bin", file_size);
    engine.record_files_batch(job_id, &[f_back]).unwrap();
    let f_back_id: i64 = {
        let conn = engine.lock_conn();
        conn.query_row(
            "SELECT id FROM files WHERE relative_path = 'back_half_100mb.bin'",
            [],
            |r| r.get(0),
        )
        .unwrap()
    };

    let mut back_chunks = Vec::with_capacity(total_chunks as usize);
    for idx in 0..total_chunks {
        let is_verified = idx >= 50;
        back_chunks.push(ChunkRecord {
            file_id: f_back_id,
            chunk_index: idx,
            offset_bytes: (idx as u64) * (chunk_size as u64),
            length_bytes: chunk_size,
            digest: if is_verified {
                Some(format!("digest_{}", idx))
            } else {
                None
            },
            state: if is_verified {
                ChunkState::Verified
            } else {
                ChunkState::Pending
            },
            attempts: if is_verified { 1 } else { 0 },
        });
    }
    engine.record_chunks_batch(&back_chunks).unwrap();

    let back_unverified = engine.get_unverified_chunks(f_back_id).unwrap();
    assert_eq!(back_unverified.len(), 50);
    assert_eq!(back_unverified.first().unwrap().chunk_index, 0);
    assert_eq!(back_unverified.last().unwrap().chunk_index, 49);

    let back_plan = engine.get_resumption_plan(f_back_id).unwrap();
    // Missing region coalesces from 0 to 50 MiB
    assert_eq!(back_plan.missing_extents.len(), 1);
    assert_eq!(back_plan.missing_extents[0], (0, 50 * (chunk_size as u64)));
    assert_eq!(back_plan.already_verified_bytes, 50 * (chunk_size as u64));
    assert_eq!(back_plan.remaining_bytes, 50 * (chunk_size as u64));

    // Pattern D: Non-aligned Residue File (100 MiB + 37,211 bytes leftover)
    let irregular_size = file_size + 37_211;
    let f_irreg = create_sample_file(job_id, "irregular_100mb.bin", irregular_size);
    engine.record_files_batch(job_id, &[f_irreg]).unwrap();
    let f_irreg_id: i64 = {
        let conn = engine.lock_conn();
        conn.query_row(
            "SELECT id FROM files WHERE relative_path = 'irregular_100mb.bin'",
            [],
            |r| r.get(0),
        )
        .unwrap()
    };

    // 101 chunks total, mark even chunks 0, 2, 4... 98 (50 chunks) verified
    let mut irreg_chunks = Vec::new();
    for idx in 0..101 {
        let offset = (idx as u64) * (chunk_size as u64);
        let len = if idx == 100 { 37_211 } else { chunk_size };
        let is_verified = idx < 100 && idx % 2 == 0;
        irreg_chunks.push(ChunkRecord {
            file_id: f_irreg_id,
            chunk_index: idx,
            offset_bytes: offset,
            length_bytes: len,
            digest: if is_verified {
                Some(format!("digest_{}", idx))
            } else {
                None
            },
            state: if is_verified {
                ChunkState::Verified
            } else {
                ChunkState::Pending
            },
            attempts: if is_verified { 1 } else { 0 },
        });
    }
    engine.record_chunks_batch(&irreg_chunks).unwrap();

    let irreg_unverified = engine.get_unverified_chunks(f_irreg_id).unwrap();
    // 101 chunks - 50 verified = 51 unverified
    assert_eq!(irreg_unverified.len(), 51);

    let irreg_plan = engine.get_resumption_plan(f_irreg_id).unwrap();
    assert_eq!(irreg_plan.total_size, irregular_size);
    assert_eq!(irreg_plan.already_verified_bytes, 50 * (chunk_size as u64));
    assert_eq!(
        irreg_plan.remaining_bytes,
        irregular_size - 50 * (chunk_size as u64)
    );
    assert_eq!(
        irreg_plan.already_verified_bytes + irreg_plan.remaining_bytes,
        irregular_size
    );

    // Verify last missing extent includes the trailing partial chunk
    let last_missing = irreg_plan.missing_extents.last().unwrap();
    // Chunk 98 is verified (offset 98 MiB, len 1 MiB -> ends at 99 MiB).
    // Chunk 99 (1 MiB) + Chunk 100 (37,211 B) are missing -> coalesce into [99 MiB, 99 MiB + 1 MiB + 37,211 B)
    assert_eq!(last_missing.0, 99 * (chunk_size as u64));
    assert_eq!(last_missing.1, (chunk_size as u64) + 37_211);
}

// ============================================================================
// STRESS TEST 5: Multi-Writer Contention under WAL Busy Timeout
// - 2 concurrent writers competing for write locks on distinct connections
// - 4 concurrent readers continuously executing queries
// - busy_timeout = 5000ms guarantees 0 SQLITE_BUSY errors
// ============================================================================
#[test]
fn test_stress_multi_writer_serialization_wal() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("wal_multi_writer_stress.db");

    let job_id = "job_multi_writer_stress";
    {
        let engine = JournalEngine::open(&db_path).unwrap();
        let mut job = create_sample_job(job_id);
        job.state = JobState::Running;
        engine.create_job(&job).unwrap();

        let mut files = Vec::new();
        for i in 0..50 {
            files.push(create_sample_file(
                job_id,
                &format!("shared_file_{:02}.dat", i),
                5 * 1024 * 1024,
            ));
        }
        engine.record_files_batch(job_id, &files).unwrap();
    }

    let is_done = Arc::new(AtomicBool::new(false));
    let busy_errors = Arc::new(AtomicUsize::new(0));
    let total_writer1_ops = Arc::new(AtomicUsize::new(0));
    let total_writer2_ops = Arc::new(AtomicUsize::new(0));
    let total_reader_ops = Arc::new(AtomicUsize::new(0));

    // Writer 1
    let db1 = db_path.clone();
    let _done_flag1 = Arc::clone(&is_done);
    let busy_ref1 = Arc::clone(&busy_errors);
    let w1_ops = Arc::clone(&total_writer1_ops);
    let w1_handle = thread::spawn(move || {
        let engine = JournalEngine::open(&db1).unwrap();
        for i in 0..100 {
            let chunk = ChunkRecord {
                file_id: (i % 25) + 1,
                chunk_index: (i / 25) as u32,
                offset_bytes: ((i / 25) as u64) * 65536,
                length_bytes: 65536,
                digest: Some(format!("digest_w1_{}", i)),
                state: ChunkState::Verified,
                attempts: 1,
            };
            match engine.record_chunk_verified(&chunk) {
                Ok(_) => {
                    w1_ops.fetch_add(1, Ordering::Relaxed);
                }
                Err(e) => {
                    eprintln!("Writer 1 error: {:?}", e);
                    busy_ref1.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    });

    // Writer 2
    let db2 = db_path.clone();
    let busy_ref2 = Arc::clone(&busy_errors);
    let w2_ops = Arc::clone(&total_writer2_ops);
    let w2_handle = thread::spawn(move || {
        let engine = JournalEngine::open(&db2).unwrap();
        for i in 0..100 {
            let chunk = ChunkRecord {
                file_id: (i % 25) + 26,
                chunk_index: (i / 25) as u32,
                offset_bytes: ((i / 25) as u64) * 65536,
                length_bytes: 65536,
                digest: Some(format!("digest_w2_{}", i)),
                state: ChunkState::Verified,
                attempts: 1,
            };
            match engine.record_chunk_verified(&chunk) {
                Ok(_) => {
                    w2_ops.fetch_add(1, Ordering::Relaxed);
                }
                Err(e) => {
                    eprintln!("Writer 2 error: {:?}", e);
                    busy_ref2.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    });

    // 4 Readers
    let mut reader_handles = Vec::new();
    for r_id in 0..4 {
        let dbr = db_path.clone();
        let done_ref = Arc::clone(&is_done);
        let busy_refr = Arc::clone(&busy_errors);
        let r_ops = Arc::clone(&total_reader_ops);

        let rh = thread::spawn(move || {
            let engine = JournalEngine::open(&dbr).unwrap();
            let mut iters = 0;
            while !done_ref.load(Ordering::Relaxed) || iters < 80 {
                iters += 1;
                match engine.get_job(job_id) {
                    Ok(_) => {
                        r_ops.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(e) => {
                        eprintln!("Reader {} error: {:?}", r_id, e);
                        busy_refr.fetch_add(1, Ordering::Relaxed);
                    }
                }
                let _ = engine.load_job_state(job_id);
            }
        });
        reader_handles.push(rh);
    }

    w1_handle.join().unwrap();
    w2_handle.join().unwrap();
    is_done.store(true, Ordering::SeqCst);
    for rh in reader_handles {
        rh.join().unwrap();
    }

    assert_eq!(total_writer1_ops.load(Ordering::SeqCst), 100);
    assert_eq!(total_writer2_ops.load(Ordering::SeqCst), 100);
    assert_eq!(
        busy_errors.load(Ordering::SeqCst),
        0,
        "Zero SQLITE_BUSY lock errors allowed during multi-writer contention!"
    );
}

// ============================================================================
// STRESS TEST 6: Unicode, Deeply Nested Paths & Complex Residues
// ============================================================================
#[test]
fn test_stress_unicode_and_deep_nested_paths() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("unicode_stress.db");
    let engine = JournalEngine::open(&db_path).unwrap();

    let job_id = "job_unicode_deep";
    let mut job = create_sample_job(job_id);
    job.state = JobState::Running;
    engine.create_job(&job).unwrap();

    // Generate paths with special Unicode (Turkish, Cyrillic, Emoji, Spaces, Quotes)
    let exotic_names = [
        "Belgelerim/Türkçe_İçerik_öüşıüğç_ÖÜŞİĞÇ/Dosya_1.pdf",
        "Deep/a/b/c/d/e/f/g/h/i/j/k/l/m/n/o/p/q/r/s/t/u/v/w/x/y/z/depth_30.bin",
        "Files with spaces/and 'single' and \"double\" quotes/data.txt",
        "Unicode_Emoji/🚀_rockét_üñîçødé_日本語_кириллица.tar.gz",
        "Zero_Byte_Boundary/empty.dat",
    ];

    let mut files = Vec::new();
    for (i, name) in exotic_names.iter().enumerate() {
        let size = if i == 4 { 0 } else { 1024 * (i as u64 + 1) };
        files.push(create_sample_file(job_id, name, size));
    }

    engine.record_files_batch(job_id, &files).unwrap();

    // Verify lookup by exact exotic Unicode path
    for f in &files {
        let conn = engine.lock_conn();
        let found_id: i64 = conn
            .query_row(
                "SELECT id FROM files WHERE job_id = ?1 AND relative_path = ?2",
                rusqlite::params![job_id, f.relative_path.to_string_lossy()],
                |r| r.get(0),
            )
            .unwrap();
        assert!(found_id > 0);
    }

    // Verify CSV report correctly escapes exotic Unicode and quotes
    let csv = engine.export_job_report_csv(job_id).unwrap();
    assert!(csv.contains("Türkçe_İçerik"));
    assert!(csv.contains("depth_30.bin"));
    assert!(csv.contains("'single'"));
    assert!(csv.contains("\"\"double\"\"")); // escaped double quotes
    assert!(csv.contains("🚀_rockét_üñîçødé"));

    // Verify JSON report exports without serialization errors
    let json = engine.export_job_report_json(job_id).unwrap();
    assert!(json.contains("Türkçe_İçerik"));
    assert!(json.contains("depth_30.bin"));
}
