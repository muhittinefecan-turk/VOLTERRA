use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;
use tempfile::tempdir;
use volterra_journal::{
    FileRecord, FileState, JobMode, JobRecord, JobState, JournalEngine, JournalStorage,
};
use volterra_recovery::{
    ActionRecommendation, ClassifiedError, ErrorCategory, ErrorDomain, ErrorQueue, ErrorSeverity,
    ExtentInterval, ExtentMap, ExtentSet, HoleFillPolicy, InteractiveAction, QuarantineState,
    QuarantinedFile, RecoveryController, RecoveryError, RetryBudget, RetryPolicy, SidecarManifest,
};

fn dummy_error(msg: &str) -> ClassifiedError {
    ClassifiedError {
        category: ErrorCategory::DeviceIo,
        domain: ErrorDomain::Source,
        severity: ErrorSeverity::FileSpecific,
        errno_code: Some(libc::EIO),
        message: msg.to_string(),
        is_retryable: true,
        suggested_action: ActionRecommendation::QuarantineAndProceed,
    }
}

fn create_test_quarantined_file(
    id: i64,
    path: &str,
    size: u64,
    extents: ExtentMap,
) -> QuarantinedFile {
    QuarantinedFile {
        file_id: id,
        job_id: "job_adversarial_test".to_string(),
        relative_path: PathBuf::from(path),
        total_size_bytes: size,
        failed_extents: extents,
        last_error: dummy_error("hardware bad sector"),
        state: QuarantineState::Quarantined,
        attempts: 1,
        quarantined_at: 1730460000,
    }
}

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

// ============================================================================
// AREA 1: EXTENT INTERVAL ALGEBRA STRESS TESTS
// ============================================================================

#[test]
fn test_adversarial_extent_fragmented_random_inserts() {
    let mut map = ExtentSet::new();

    // Generate 1,000 disjoint intervals [i * 10, i * 10 + 6)
    // and scramble them deterministically with an LCG permutation
    let count = 1000usize;
    let mut indices: Vec<usize> = (0..count).collect();

    let mut seed = 0xdeadbeef_u64;
    for i in (1..count).rev() {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let j = (seed % (i as u64 + 1)) as usize;
        indices.swap(i, j);
    }

    // Insert all scrambled intervals
    for &idx in &indices {
        let offset = (idx as u64) * 10;
        let length = 6;
        map.insert(ExtentInterval::new(offset, length).unwrap());
    }

    // 1. Must contain exactly 1,000 intervals
    assert_eq!(
        map.count(),
        1000,
        "ExtentMap count mismatch after fragmented inserts"
    );

    // 2. Total bytes must be exactly 1000 * 6 = 6,000
    assert_eq!(map.total_bytes(), 6000, "Total bytes mismatch");
    assert_eq!(map.total_failed_bytes(), 6000);

    // 3. Verify strict monotonic sorting and strictly disjoint property
    let intervals = map.intervals();
    for i in 0..intervals.len() {
        assert_eq!(intervals[i].offset, (i as u64) * 10);
        assert_eq!(intervals[i].length, 6);
        assert_eq!(intervals[i].end(), (i as u64) * 10 + 6);

        if i + 1 < intervals.len() {
            assert!(
                intervals[i].end() < intervals[i + 1].offset,
                "Intervals at {} and {} are not strictly disjoint: [{}, {}) vs [{}, {})",
                i,
                i + 1,
                intervals[i].offset,
                intervals[i].end(),
                intervals[i + 1].offset,
                intervals[i + 1].end()
            );
        }
    }
}

#[test]
fn test_adversarial_extent_massive_span_bridge() {
    let mut map = ExtentSet::new();

    // 500 intervals: [i * 20, i * 20 + 10) for i in 0..500
    // e.g. [0, 10), [20, 30), [40, 50), ..., [9980, 9990)
    for i in 0..500 {
        map.add_extent(i * 20, 10);
    }
    assert_eq!(map.count(), 500);

    // Now insert a massive span that bridges from offset 990 to 8010
    // (covers from [980, 990) up through [8000, 8010))
    // Specifically: [990, 8010) touches adjacent [980, 990) and overlaps [8000, 8010).
    // The merged span will start at 980 and end at 8010.
    map.insert(ExtentInterval::new(990, 7020).unwrap());

    // Before 980: intervals 0..48 (offsets 0, 20, ..., 960) -> 49 intervals
    // Merged span: [980, 8010) -> 1 interval (at index 49)
    // After 8010: intervals 401..500 (offsets 8020, 8040, ..., 9980) -> 99 intervals
    // Total intervals should be 49 + 1 + 99 = 149
    assert_eq!(map.count(), 149);

    let intervals = map.intervals();
    assert_eq!(intervals[48].offset, 960);
    assert_eq!(intervals[48].end(), 970);

    assert_eq!(intervals[49].offset, 980);
    assert_eq!(intervals[49].end(), 8010);
    assert_eq!(intervals[49].length, 8010 - 980);

    assert_eq!(intervals[50].offset, 8020);
    assert_eq!(intervals[50].end(), 8030);

    // Verify disjointness across entire map
    for i in 0..intervals.len() - 1 {
        assert!(intervals[i].end() < intervals[i + 1].offset);
    }
}

#[test]
fn test_adversarial_extent_overlapping_subsets_supersets_and_adjacent() {
    let mut map = ExtentMap::new();

    // Initial base interval: [100, 300)
    map.insert(ExtentInterval::new(100, 200).unwrap());
    assert_eq!(map.count(), 1);
    assert_eq!(
        map.intervals()[0],
        ExtentInterval {
            offset: 100,
            length: 200
        }
    );

    // 1. Strict subset [150, 250) (offset 150, len 100) -> should not change map
    map.insert(ExtentInterval::new(150, 100).unwrap());
    assert_eq!(map.count(), 1);
    assert_eq!(
        map.intervals()[0],
        ExtentInterval {
            offset: 100,
            length: 200
        }
    );
    assert_eq!(map.total_bytes(), 200);

    // 2. Left-overlapping [50, 100) (offset 50, len 100, ends at 150) -> expands to [50, 300)
    map.insert(ExtentInterval::new(50, 100).unwrap());
    assert_eq!(map.count(), 1);
    assert_eq!(
        map.intervals()[0],
        ExtentInterval {
            offset: 50,
            length: 250
        }
    );
    assert_eq!(map.total_bytes(), 250);

    // 3. Right-adjacent [300, 400) (offset 300, len 100) -> expands to [50, 400)
    map.insert(ExtentInterval::new(300, 100).unwrap());
    assert_eq!(map.count(), 1);
    assert_eq!(
        map.intervals()[0],
        ExtentInterval {
            offset: 50,
            length: 350
        }
    );
    assert_eq!(map.total_bytes(), 350);

    // 4. Left-adjacent [20, 50) (offset 20, len 30) -> expands to [20, 400)
    map.insert(ExtentInterval::new(20, 30).unwrap());
    assert_eq!(map.count(), 1);
    assert_eq!(
        map.intervals()[0],
        ExtentInterval {
            offset: 20,
            length: 380
        }
    );
    assert_eq!(map.total_bytes(), 380);

    // 5. Strict disjoint before: [0, 10) (offset 0, len 10) -> count becomes 2: [0, 10) and [20, 400)
    map.insert(ExtentInterval::new(0, 10).unwrap());
    assert_eq!(map.count(), 2);
    assert_eq!(
        map.intervals()[0],
        ExtentInterval {
            offset: 0,
            length: 10
        }
    );
    assert_eq!(
        map.intervals()[1],
        ExtentInterval {
            offset: 20,
            length: 380
        }
    );
    assert_eq!(map.total_bytes(), 390);

    // 6. Strict disjoint after: [500, 600) -> count becomes 3: [0, 10), [20, 400), [500, 600)
    map.insert(ExtentInterval::new(500, 100).unwrap());
    assert_eq!(map.count(), 3);
    assert_eq!(
        map.intervals()[2],
        ExtentInterval {
            offset: 500,
            length: 100
        }
    );
    assert_eq!(map.total_bytes(), 490);

    // 7. Bridge gap between [0, 10) and [20, 400) with [10, 10) -> [10, 20)
    map.insert(ExtentInterval::new(10, 10).unwrap());
    assert_eq!(map.count(), 2);
    assert_eq!(
        map.intervals()[0],
        ExtentInterval {
            offset: 0,
            length: 400
        }
    );
    assert_eq!(
        map.intervals()[1],
        ExtentInterval {
            offset: 500,
            length: 100
        }
    );
    assert_eq!(map.total_bytes(), 500);

    // 8. Massive superset [0, 1000) absorbs everything
    map.insert(ExtentInterval::new(0, 1000).unwrap());
    assert_eq!(map.count(), 1);
    assert_eq!(
        map.intervals()[0],
        ExtentInterval {
            offset: 0,
            length: 1000
        }
    );
    assert_eq!(map.total_bytes(), 1000);
}

#[test]
fn test_adversarial_extent_empty_and_zero_ranges() {
    let mut map = ExtentMap::new();

    // 1. ExtentInterval::new(0, 0) must return Err
    assert!(matches!(
        ExtentInterval::new(0, 0),
        Err(RecoveryError::InvalidExtent { .. })
    ));
    assert!(matches!(
        ExtentInterval::new(500, 0),
        Err(RecoveryError::InvalidExtent { .. })
    ));

    // 2. insert_range with length 0 must return Err
    assert!(map.insert_range(100, 0).is_err());
    assert!(map.is_empty());

    // 3. add_extent with length 0 must be silently ignored without modifying map
    map.add_extent(200, 0);
    assert!(map.is_empty());

    // 4. Direct insert of length 0 extent (if constructed)
    map.insert(ExtentInterval {
        offset: 300,
        length: 0,
    });
    assert!(map.is_empty());
}

#[test]
fn test_adversarial_extent_u64_limits_and_1tb_ranges() {
    let mut map = ExtentSet::new();

    let one_tb: u64 = 1024 * 1024 * 1024 * 1024; // 1,099,511,627,776 bytes (1 TiB)

    // Insert 1 TiB range: [0, 1 TiB)
    map.add_extent(0, one_tb);
    assert_eq!(map.count(), 1);
    assert_eq!(map.total_bytes(), one_tb);

    // Insert contiguous second 1 TiB range: [1 TiB, 2 TiB)
    map.add_extent(one_tb, one_tb);
    assert_eq!(map.count(), 1);
    assert_eq!(map.total_bytes(), 2 * one_tb);
    assert_eq!(
        map.intervals()[0],
        ExtentInterval {
            offset: 0,
            length: 2 * one_tb
        }
    );

    // Overflow boundary checks
    // Max safe interval: offset = u64::MAX - 1000, length = 1000 -> end() = u64::MAX
    let max_safe = ExtentInterval::new(u64::MAX - 1000, 1000);
    assert!(max_safe.is_ok());
    let ext = max_safe.unwrap();
    assert_eq!(ext.end(), u64::MAX);

    // 1 past u64::MAX must fail
    let overflow1 = ExtentInterval::new(u64::MAX - 1000, 1001);
    assert!(overflow1.is_err());

    let overflow2 = ExtentInterval::new(u64::MAX, 1);
    assert!(overflow2.is_err());

    let overflow_range = map.insert_range(u64::MAX - 50, 100);
    assert!(overflow_range.is_err());
}

#[test]
fn test_adversarial_extent_complement_derivation_invariants() {
    let mut map = ExtentSet::new();

    // 1. Empty map complement on 0-length file -> empty
    let comp_zero = map.complement(0);
    assert!(comp_zero.is_empty());

    // 2. Empty map complement on 10,000-byte file -> single [0, 10,000) interval
    let comp_empty = map.complement(10_000);
    assert_eq!(comp_empty.count(), 1);
    assert_eq!(
        comp_empty.intervals()[0],
        ExtentInterval {
            offset: 0,
            length: 10_000
        }
    );
    assert_eq!(comp_empty.total_bytes(), 10_000);

    // 3. Extent entirely inside file: [2000, 5000) in 10,000-byte file
    map.add_extent(2000, 3000);
    let comp = map.complement(10_000);
    assert_eq!(comp.count(), 2);
    assert_eq!(
        comp.intervals()[0],
        ExtentInterval {
            offset: 0,
            length: 2000
        }
    );
    assert_eq!(
        comp.intervals()[1],
        ExtentInterval {
            offset: 5000,
            length: 5000
        }
    );
    // Invariant: Failed + Healthy = Total
    assert_eq!(map.total_bytes() + comp.total_bytes(), 10_000);

    // 4. Involutive property (Double Complement):
    // comp(comp(map)) == map (for extents strictly bounded within total_size)
    let double_comp = comp.complement(10_000);
    assert_eq!(
        double_comp, map,
        "Double complement must restore original extent map"
    );

    // 5. Extents reaching beyond file size:
    // File size = 8000, but map has [2000, 5000) and [7000, 12000)
    map.add_extent(7000, 5000); // [7000, 12000)
    let comp_clamped = map.complement(8000);
    // Complement should have: [0, 2000) and [5000, 7000). Nothing past 8000!
    assert_eq!(comp_clamped.count(), 2);
    assert_eq!(
        comp_clamped.intervals()[0],
        ExtentInterval {
            offset: 0,
            length: 2000
        }
    );
    assert_eq!(
        comp_clamped.intervals()[1],
        ExtentInterval {
            offset: 5000,
            length: 2000
        }
    );
    for slice in comp_clamped.intervals() {
        assert!(
            slice.end() <= 8000,
            "Complement slice extends beyond total_size"
        );
    }

    // 6. Extent starts strictly after file size: [9000, 10000) with file size 5000
    let mut map_past = ExtentSet::new();
    map_past.add_extent(9000, 1000);
    let comp_past = map_past.complement(5000);
    assert_eq!(comp_past.count(), 1);
    assert_eq!(
        comp_past.intervals()[0],
        ExtentInterval {
            offset: 0,
            length: 5000
        }
    );
}

// ============================================================================
// AREA 2: QUARANTINE QUEUE MULTI-THREADED STRESS TESTS
// ============================================================================

#[test]
fn test_adversarial_quarantine_queue_20_threads_1000_files() {
    let queue = Arc::new(ErrorQueue::new());
    let thread_count = 20;
    let files_per_thread = 50;
    let total_expected_files = thread_count * files_per_thread; // 1,000 files

    let barrier = Arc::new(Barrier::new(thread_count + 2)); // 20 writers + 2 readers
    let mut handles = Vec::new();

    // 20 Writer threads: concurrently isolate errors across distinct file IDs
    for t in 0..thread_count {
        let q = Arc::clone(&queue);
        let b = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            b.wait();
            for f in 0..files_per_thread {
                let file_id = (t * files_per_thread + f + 1) as i64;
                let mut extents = ExtentMap::new();
                extents.add_extent((f * 4096) as u64, 4096);

                let qf = create_test_quarantined_file(
                    file_id,
                    &format!("batch_{}/file_{}.dat", t, f),
                    100_000,
                    extents,
                );
                q.quarantine_file(qf).expect("Failed to quarantine file");
            }
        }));
    }

    // 2 Concurrent Reader threads continuously calling list_all, list_quarantined, count
    let stop_readers = Arc::new(AtomicBool::new(false));
    let read_iterations = Arc::new(AtomicUsize::new(0));

    for _ in 0..2 {
        let q = Arc::clone(&queue);
        let b = Arc::clone(&barrier);
        let stop = Arc::clone(&stop_readers);
        let iters = Arc::clone(&read_iterations);
        handles.push(thread::spawn(move || {
            b.wait();
            while !stop.load(Ordering::Relaxed) {
                let _ = q.count();
                let _ = q.count_active_quarantine();
                let _ = q.list_quarantined();
                let _ = q.list_all();
                iters.fetch_add(1, Ordering::Relaxed);
                thread::yield_now();
            }
        }));
    }

    // Wait for all 20 writer threads
    for handle in handles.drain(..thread_count) {
        handle.join().expect("Writer thread panicked");
    }

    // Stop readers
    stop_readers.store(true, Ordering::Relaxed);
    for handle in handles {
        handle.join().expect("Reader thread panicked");
    }

    // Verification:
    // 1. Total queue count must be exactly 1,000
    assert_eq!(
        queue.count(),
        total_expected_files,
        "Queue count mismatch after 20-thread concurrency"
    );
    assert_eq!(queue.count_active_quarantine(), total_expected_files);

    // 2. All 1,000 files must exist and be uniquely retrievable with correct IDs
    let all_files = queue.list_all();
    assert_eq!(all_files.len(), total_expected_files);

    let mut found_ids = HashSet::new();
    for f in all_files {
        assert!(
            found_ids.insert(f.file_id),
            "Duplicate file ID found in queue: {}",
            f.file_id
        );
        let retrieved = queue.get_file(f.file_id);
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().file_id, f.file_id);
    }
    assert_eq!(found_ids.len(), total_expected_files);

    for expected_id in 1..=total_expected_files as i64 {
        assert!(
            found_ids.contains(&expected_id),
            "Missing file ID: {}",
            expected_id
        );
    }

    println!(
        "Concurrent 20-thread stress completed successfully: 1,000 files isolated, {} reader snapshots observed.",
        read_iterations.load(Ordering::Relaxed)
    );
}

#[test]
fn test_adversarial_quarantine_queue_unaffected_files_isolation() {
    let queue = Arc::new(ErrorQueue::new());

    // Populate queue with 200 quarantined files (IDs 1..=200)
    for i in 1..=200 {
        let mut ext = ExtentMap::new();
        ext.add_extent(0, 512);
        let qf = create_test_quarantined_file(i, &format!("unreadable_{}.dat", i), 10_000, ext);
        queue.quarantine_file(qf).unwrap();
    }

    let barrier = Arc::new(Barrier::new(10));
    let mut handles = Vec::new();

    // 5 threads simulate unaffected/healthy worker threads querying their IDs (201..=500)
    for t in 0..5 {
        let q = Arc::clone(&queue);
        let b = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            b.wait();
            for offset in 1..=100 {
                let healthy_id = 200 + t * 100 + offset;
                // Querying unaffected file must return None without blocking or failing
                assert!(q.get_file(healthy_id).is_none());
                // Attempting state update on unaffected file must return FileNotFound
                assert!(matches!(
                    q.update_state(healthy_id, QuarantineState::Skipped),
                    Err(RecoveryError::FileNotFound(_))
                ));
            }
        }));
    }

    // 5 threads simultaneously querying and reading quarantined files (1..=200)
    for _ in 0..5 {
        let q = Arc::clone(&queue);
        let b = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            b.wait();
            for id in 1..=200 {
                let file = q.get_file(id);
                assert!(file.is_some());
                assert_eq!(file.unwrap().state, QuarantineState::Quarantined);
            }
        }));
    }

    for h in handles {
        h.join().expect("Isolation stress thread panicked");
    }

    assert_eq!(queue.count(), 200);
}

#[test]
fn test_adversarial_quarantine_queue_concurrent_read_write_remove_stress() {
    let queue = Arc::new(ErrorQueue::new());
    let barrier = Arc::new(Barrier::new(8));
    let mut handles = Vec::new();

    // Thread 0-1: Continually insert / quarantine files
    for t in 0..2 {
        let q = Arc::clone(&queue);
        let b = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            b.wait();
            for i in 0..500 {
                let id = (t * 1000 + i) as i64;
                let mut ext = ExtentMap::new();
                ext.add_extent(0, 1024);
                let qf = create_test_quarantined_file(id, &format!("f_{}.bin", id), 2000, ext);
                let _ = q.quarantine_file(qf);
            }
        }));
    }

    // Thread 2-3: Continually update states
    for t in 0..2 {
        let q = Arc::clone(&queue);
        let b = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            b.wait();
            for i in 0..500 {
                let id = (t * 1000 + i) as i64;
                let _ = q.update_state(id, QuarantineState::Deferred);
                let _ = q.update_state(id, QuarantineState::Retrying);
            }
        }));
    }

    // Thread 4-5: Continually remove subset of files
    for t in 0..2 {
        let q = Arc::clone(&queue);
        let b = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            b.wait();
            for i in 0..250 {
                let id = (t * 1000 + i * 2) as i64;
                let _ = q.remove(id);
            }
        }));
    }

    // Thread 6-7: Continually read lists
    for _ in 0..2 {
        let q = Arc::clone(&queue);
        let b = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            b.wait();
            for _ in 0..500 {
                let _ = q.list_quarantined();
                let _ = q.list_deferred();
                let _ = q.list_all();
                let _ = q.count();
            }
        }));
    }

    for h in handles {
        h.join().expect("R/W/Remove stress thread panicked");
    }

    // Queue must remain fully coherent
    let remaining = queue.list_all();
    assert_eq!(remaining.len(), queue.count());
}

// ============================================================================
// AREA 3: INTERACTIVE CONTROLS STATE MACHINE & CONCURRENCY
// ============================================================================

#[test]
fn test_adversarial_interactive_controls_rapid_state_cycles() {
    let queue = Arc::new(ErrorQueue::new());
    let budget = RetryBudget::default();
    let controller = RecoveryController::new(Arc::clone(&queue), budget);

    let mut ext = ExtentMap::new();
    ext.add_extent(100, 500);
    let qf = create_test_quarantined_file(42, "state_test.dat", 10_000, ext);
    queue.quarantine_file(qf).unwrap();

    // 1. Initial: Quarantined
    assert_eq!(
        queue.get_file(42).unwrap().state,
        QuarantineState::Quarantined
    );

    // 2. SkipFile -> Skipped
    let r1 = controller
        .apply_action(InteractiveAction::SkipFile {
            file_id: 42,
            reason: Some("User skipped".to_string()),
        })
        .unwrap();
    assert_eq!(r1[0].state, QuarantineState::Skipped);
    assert_eq!(queue.get_file(42).unwrap().state, QuarantineState::Skipped);

    // 3. RetryFile -> Retrying (even from Skipped)
    let r2 = controller
        .apply_action(InteractiveAction::RetryFile {
            file_id: 42,
            reset_budget: true,
        })
        .unwrap();
    assert_eq!(r2[0].state, QuarantineState::Retrying);
    assert_eq!(queue.get_file(42).unwrap().state, QuarantineState::Retrying);

    // 4. DeferFile -> Deferred
    let r3 = controller
        .apply_action(InteractiveAction::DeferFile { file_id: 42 })
        .unwrap();
    assert_eq!(r3[0].state, QuarantineState::Deferred);
    assert_eq!(queue.get_file(42).unwrap().state, QuarantineState::Deferred);

    // 5. SkipAll on Deferred -> Skipped
    let r4 = controller.apply_action(InteractiveAction::SkipAll).unwrap();
    assert_eq!(r4.len(), 1);
    assert_eq!(r4[0].file_id, 42);
    assert_eq!(r4[0].state, QuarantineState::Skipped);
    assert_eq!(queue.get_file(42).unwrap().state, QuarantineState::Skipped);

    // 6. Idempotent SkipAll -> returns 0 skipped since file is already Skipped
    let r5 = controller.apply_action(InteractiveAction::SkipAll).unwrap();
    assert_eq!(r5.len(), 0);

    // 7. RetryAll -> affects Quarantined and Deferred files. Skipped files are preserved!
    let r6 = controller
        .apply_action(InteractiveAction::RetryAll)
        .unwrap();
    assert_eq!(r6.len(), 0);
    assert_eq!(queue.get_file(42).unwrap().state, QuarantineState::Skipped);

    // 8. Explicit RetryFile on Skipped file works
    let r7 = controller
        .apply_action(InteractiveAction::RetryFile {
            file_id: 42,
            reset_budget: true,
        })
        .unwrap();
    assert_eq!(r7[0].state, QuarantineState::Retrying);
}

#[test]
fn test_adversarial_interactive_controls_concurrent_actions() {
    let queue = Arc::new(ErrorQueue::new());
    let budget = RetryBudget::default();
    let controller = Arc::new(RecoveryController::new(Arc::clone(&queue), budget));

    // Populate 100 files
    for i in 1..=100 {
        let mut ext = ExtentMap::new();
        ext.add_extent(0, 100);
        let qf = create_test_quarantined_file(i, &format!("f_{}.dat", i), 5000, ext);
        queue.quarantine_file(qf).unwrap();
    }

    let barrier = Arc::new(Barrier::new(8));
    let mut handles = Vec::new();

    // 2 threads calling SkipFile randomly
    for t in 0..2 {
        let c = Arc::clone(&controller);
        let b = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            b.wait();
            for i in 1..=50 {
                let id = (t * 50 + i) as i64;
                let _ = c.apply_action(InteractiveAction::SkipFile {
                    file_id: id,
                    reason: None,
                });
            }
        }));
    }

    // 2 threads calling RetryFile randomly
    for t in 0..2 {
        let c = Arc::clone(&controller);
        let b = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            b.wait();
            for i in 1..=50 {
                let id = (t * 50 + i) as i64;
                let _ = c.apply_action(InteractiveAction::RetryFile {
                    file_id: id,
                    reset_budget: true,
                });
            }
        }));
    }

    // 2 threads calling DeferFile
    for t in 0..2 {
        let c = Arc::clone(&controller);
        let b = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            b.wait();
            for i in 1..=50 {
                let id = (t * 50 + i) as i64;
                let _ = c.apply_action(InteractiveAction::DeferFile { file_id: id });
            }
        }));
    }

    // 2 threads alternating SkipAll and RetryAll
    for _ in 0..2 {
        let c = Arc::clone(&controller);
        let b = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            b.wait();
            for _ in 0..10 {
                let _ = c.apply_action(InteractiveAction::SkipAll);
                let _ = c.apply_action(InteractiveAction::RetryAll);
            }
        }));
    }

    for h in handles {
        h.join()
            .expect("Concurrent interactive actions thread panicked");
    }

    // Check consistency: all 100 files must remain in queue, with valid states
    let files = queue.list_all();
    assert_eq!(files.len(), 100);
    for f in files {
        assert!(matches!(
            f.state,
            QuarantineState::Quarantined
                | QuarantineState::Deferred
                | QuarantineState::Retrying
                | QuarantineState::Skipped
        ));
    }
}

#[test]
fn test_adversarial_interactive_controls_journal_sync() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("volterra_sync.db");
    let journal = Arc::new(JournalEngine::open(&db_path).unwrap());

    // Create job record
    let job = create_sample_job("sync_job_1");
    journal.create_job(&job).unwrap();

    let f1 = create_sample_file("sync_job_1", "f1.bin", 10_000);
    let f2 = create_sample_file("sync_job_1", "f2.bin", 10_000);

    journal.record_files_batch("sync_job_1", &[f1, f2]).unwrap();

    // Fetch generated file IDs from journal DB
    let (id1, id2) = {
        let conn = journal.lock_conn();
        let id1: i64 = conn
            .query_row(
                "SELECT id FROM files WHERE relative_path = 'f1.bin'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let id2: i64 = conn
            .query_row(
                "SELECT id FROM files WHERE relative_path = 'f2.bin'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        (id1, id2)
    };

    // Create RecoveryController with journal
    let queue = Arc::new(ErrorQueue::new());
    let budget = RetryBudget::default();
    let controller = RecoveryController::with_journal(
        Arc::clone(&queue),
        budget,
        Arc::clone(&journal) as Arc<dyn JournalStorage>,
    );

    // Quarantine files
    for id in [id1, id2] {
        let mut ext = ExtentMap::new();
        ext.add_extent(0, 1024);
        let qf = create_test_quarantined_file(id, &format!("f_{}.bin", id), 10_000, ext);
        queue.quarantine_file(qf).unwrap();
    }

    // 1. SkipFile -> journal must reflect Skipped
    controller
        .apply_action(InteractiveAction::SkipFile {
            file_id: id1,
            reason: Some("skip test".to_string()),
        })
        .unwrap();

    {
        let conn = journal.lock_conn();
        let state1: String = conn
            .query_row("SELECT state FROM files WHERE id = ?1", [id1], |r| r.get(0))
            .unwrap();
        assert_eq!(state1, "Skipped");
    }

    // 2. DeferFile -> journal must reflect NeedsAttention
    controller
        .apply_action(InteractiveAction::DeferFile { file_id: id2 })
        .unwrap();
    {
        let conn = journal.lock_conn();
        let state2: String = conn
            .query_row("SELECT state FROM files WHERE id = ?1", [id2], |r| r.get(0))
            .unwrap();
        assert_eq!(state2, "NeedsAttention");
    }

    // 3. RetryFile -> journal must reflect Pending
    controller
        .apply_action(InteractiveAction::RetryFile {
            file_id: id2,
            reset_budget: true,
        })
        .unwrap();
    {
        let conn = journal.lock_conn();
        let state2_retry: String = conn
            .query_row("SELECT state FROM files WHERE id = ?1", [id2], |r| r.get(0))
            .unwrap();
        assert_eq!(state2_retry, "Pending");
    }
}

#[test]
fn test_adversarial_retry_budget_exhaustion_and_reset() {
    let policy = RetryPolicy {
        max_retries_per_file: 3,
        max_job_retries: 5,
        base_delay_ms: 10,
        max_delay_ms: 100,
        backoff_multiplier: 2.0,
        jitter_factor: 0.1,
    };
    let mut budget = RetryBudget::new(policy);

    // File 1 attempts 1, 2, 3
    let d1 = budget.record_file_attempt(1).unwrap();
    let d2 = budget.record_file_attempt(1).unwrap();
    let d3 = budget.record_file_attempt(1).unwrap();
    assert!(d2 >= d1);
    assert!(d3 >= d2);

    // Attempt 4 must fail with RetryBudgetExhausted
    let d4 = budget.record_file_attempt(1);
    assert!(matches!(
        d4,
        Err(RecoveryError::RetryBudgetExhausted {
            attempts: 3,
            max_retries: 3
        })
    ));

    // Reset file budget
    budget.reset_file(1);

    // Can attempt again after reset
    let d1_after = budget.record_file_attempt(1);
    assert!(d1_after.is_ok());
}

// ============================================================================
// AREA 4: FAIL-CLOSED PARTIAL RECOVERY SIDECAR INTEGRITY
// ============================================================================

#[test]
fn test_adversarial_sidecar_fail_closed_barrier_enforcement() {
    // 1. Partial file: verified < total, 0 missing extents recorded
    // MUST fail closed!
    assert!(matches!(
        SidecarManifest::enforce_fail_closed_barrier(1000, 999, 0),
        Err(RecoveryError::FailClosedBarrierFailed {
            total_size: 1000,
            verified_bytes: 999,
            missing_extents: 0
        })
    ));

    // 2. Fully copied bytes, but missing extent count > 0 (bad sector occurred)
    // MUST fail closed!
    assert!(matches!(
        SidecarManifest::enforce_fail_closed_barrier(1000, 1000, 1),
        Err(RecoveryError::FailClosedBarrierFailed {
            total_size: 1000,
            verified_bytes: 1000,
            missing_extents: 1
        })
    ));

    // 3. Both incomplete and missing extents
    // MUST fail closed!
    assert!(matches!(
        SidecarManifest::enforce_fail_closed_barrier(1000, 500, 3),
        Err(RecoveryError::FailClosedBarrierFailed {
            total_size: 1000,
            verified_bytes: 500,
            missing_extents: 3
        })
    ));

    // 4. Zero-byte file: verified = 0, total = 0, missing = 0 -> OK
    assert!(SidecarManifest::enforce_fail_closed_barrier(0, 0, 0).is_ok());

    // 5. Zero-byte file with 1 missing extent -> MUST fail closed!
    assert!(SidecarManifest::enforce_fail_closed_barrier(0, 0, 1).is_err());

    // 6. Complete and verified file: verified = total, missing = 0 -> OK
    assert!(SidecarManifest::enforce_fail_closed_barrier(1024 * 1024, 1024 * 1024, 0).is_ok());
}

#[test]
fn test_adversarial_sidecar_manifest_invariants_and_integrity() {
    let mut failed = ExtentMap::new();
    failed.add_extent(1000, 500); // 500 missing bytes
    failed.add_extent(3000, 500); // 500 missing bytes

    let total_size = 10_000u64;
    let sidecar = SidecarManifest::create(
        "job_sidecar_adv".to_string(),
        PathBuf::from("docs/report.pdf"),
        total_size,
        &failed,
        HoleFillPolicy::ZeroFilled,
    );

    // Invariants mandated by specification:
    // - is_complete is ALWAYS false
    // - is_partial_recovery is ALWAYS true
    // - checksum_verified is ALWAYS false
    assert!(
        !sidecar.is_complete,
        "Incomplete recovery cannot be marked complete"
    );
    assert!(
        sidecar.is_partial_recovery,
        "Partial recovery flag must be true"
    );
    assert!(
        !sidecar.checksum_verified,
        "Partial recovery cannot claim checksum verified"
    );

    assert_eq!(sidecar.total_missing_bytes, 1000);
    assert_eq!(sidecar.total_recovered_bytes, 9000);
    assert_eq!(sidecar.unreadable_bytes, 1000);
    assert_eq!(sidecar.recovered_bytes, 9000);
    assert!((sidecar.recovery_percentage - 90.0).abs() < 1e-6);

    assert_eq!(sidecar.missing_extents.len(), 2);
    assert_eq!(sidecar.healthy_extents.len(), 3);
    assert!(sidecar.warning.contains("WARNING"));
    assert!(sidecar
        .warning
        .contains("DO NOT treat as a complete or bit-identical copy"));
}

#[test]
fn test_adversarial_sidecar_file_io_roundtrip_and_tampering() {
    let dir = tempdir().unwrap();
    let sidecar_path = dir
        .path()
        .join("nested")
        .join("sub")
        .join(".volterra.archive.tar.sidecar.json");

    let mut failed = ExtentMap::new();
    for i in 0..50 {
        failed.add_extent(i * 1000, 200);
    }

    let sidecar = SidecarManifest::create(
        "job_io_adv".to_string(),
        PathBuf::from("archive.tar"),
        100_000,
        &failed,
        HoleFillPolicy::SparseHole,
    );

    // 1. Write to nested path (verifies create_dir_all behavior)
    sidecar
        .write_to_file(&sidecar_path)
        .expect("Failed to write sidecar");
    assert!(sidecar_path.exists());

    // 2. Read back and verify exact integrity
    let loaded = SidecarManifest::read_from_file(&sidecar_path).expect("Failed to read sidecar");
    assert_eq!(loaded.job_id, sidecar.job_id);
    assert_eq!(loaded.relative_path, sidecar.relative_path);
    assert_eq!(loaded.total_size_bytes, sidecar.total_size_bytes);
    assert_eq!(loaded.total_missing_bytes, sidecar.total_missing_bytes);
    assert_eq!(loaded.missing_extents.len(), 50);
    assert_eq!(loaded.hole_fill_policy, HoleFillPolicy::SparseHole);
    assert!(!loaded.is_complete);

    // 3. Tampering: corrupt JSON syntax on disk
    fs::write(
        &sidecar_path,
        b"{\"volterra_sidecar_version\": 1, CORRUPT_JSON",
    )
    .unwrap();
    let corrupt_res = SidecarManifest::read_from_file(&sidecar_path);
    assert!(matches!(corrupt_res, Err(RecoveryError::Serialization(_))));

    // 4. Tampering: truncate file to 0 bytes
    fs::write(&sidecar_path, b"").unwrap();
    let empty_res = SidecarManifest::read_from_file(&sidecar_path);
    assert!(matches!(empty_res, Err(RecoveryError::Serialization(_))));
}
