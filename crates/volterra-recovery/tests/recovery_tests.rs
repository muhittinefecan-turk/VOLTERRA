use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::tempdir;
use volterra_recovery::{
    ActionRecommendation, ClassifiedError, ErrorCategory, ErrorDomain, ErrorQueue, ErrorSeverity,
    ExtentInterval, ExtentMap, HoleFillPolicy, InteractiveAction, QuarantineState, QuarantinedFile,
    RecoveryClassifier, RecoveryController, RecoveryErrorClass, RetryBudget, RetryPolicy,
    SidecarManifest,
};

fn create_test_error(msg: &str) -> ClassifiedError {
    ClassifiedError {
        category: ErrorCategory::DeviceIo,
        domain: ErrorDomain::Source,
        severity: ErrorSeverity::FileSpecific,
        errno_code: Some(5),
        message: msg.to_string(),
        is_retryable: true,
        suggested_action: ActionRecommendation::QuarantineAndProceed,
    }
}

fn create_quarantined_file(id: i64, path: &str, size: u64, extents: ExtentMap) -> QuarantinedFile {
    QuarantinedFile {
        file_id: id,
        job_id: "test_job".to_string(),
        relative_path: PathBuf::from(path),
        total_size_bytes: size,
        failed_extents: extents,
        last_error: create_test_error("bad sector read"),
        state: QuarantineState::Quarantined,
        attempts: 1,
        quarantined_at: 1000,
    }
}

/// TEST-RECO-01: Errno to RecoveryErrorClass Classification
#[test]
fn test_reco_01_errno_classification() {
    let eio = std::io::Error::from_raw_os_error(libc::EIO);
    assert_eq!(
        RecoveryErrorClass::classify_io_error(&eio),
        RecoveryErrorClass::ReadEio
    );

    let enospc = std::io::Error::from_raw_os_error(libc::ENOSPC);
    assert_eq!(
        RecoveryErrorClass::classify_io_error(&enospc),
        RecoveryErrorClass::WriteEnospc
    );

    let eacces = std::io::Error::from_raw_os_error(libc::EACCES);
    assert_eq!(
        RecoveryErrorClass::classify_io_error(&eacces),
        RecoveryErrorClass::PermissionDenied
    );

    let enodev = std::io::Error::from_raw_os_error(libc::ENODEV);
    assert_eq!(
        RecoveryErrorClass::classify_io_error(&enodev),
        RecoveryErrorClass::DeviceDisconnected
    );

    let short_read = std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "short read");
    assert_eq!(
        RecoveryErrorClass::classify_io_error(&short_read),
        RecoveryErrorClass::ShortRead
    );
}

/// TEST-RECO-02: Extent Interval Merging - Adjacent Ranges
#[test]
fn test_reco_02_extent_merging_adjacent_ranges() {
    let mut map = ExtentMap::new();
    map.add_extent(0, 1024);
    map.add_extent(1024, 1024);

    assert_eq!(map.intervals().len(), 1);
    assert_eq!(
        map.intervals()[0],
        ExtentInterval {
            offset: 0,
            length: 2048
        }
    );
    assert_eq!(map.total_failed_bytes(), 2048);
}

/// TEST-RECO-03: Extent Interval Merging - Overlapping Ranges
#[test]
fn test_reco_03_extent_merging_overlapping_ranges() {
    let mut map = ExtentMap::new();
    map.add_extent(100, 400); // [100, 500)
    map.add_extent(300, 500); // [300, 800)

    assert_eq!(map.intervals().len(), 1);
    assert_eq!(
        map.intervals()[0],
        ExtentInterval {
            offset: 100,
            length: 700
        }
    ); // [100, 800)
    assert_eq!(map.total_failed_bytes(), 700);
}

/// TEST-RECO-04: Extent Interval Merging - Subset Ranges
#[test]
fn test_reco_04_extent_merging_subset_ranges() {
    let mut map = ExtentMap::new();
    map.add_extent(100, 800); // [100, 900)
    map.add_extent(200, 300); // [200, 500)

    assert_eq!(map.intervals().len(), 1);
    assert_eq!(
        map.intervals()[0],
        ExtentInterval {
            offset: 100,
            length: 800
        }
    );
    assert_eq!(map.total_failed_bytes(), 800);
}

/// TEST-RECO-05: Extent Interval Merging - Disjoint Sorted Order
#[test]
fn test_reco_05_extent_merging_disjoint_sorted() {
    let mut map = ExtentMap::new();
    map.add_extent(5000, 1000);
    map.add_extent(1000, 1000);
    map.add_extent(3000, 1000);

    assert_eq!(map.intervals().len(), 3);
    assert_eq!(
        map.intervals()[0],
        ExtentInterval {
            offset: 1000,
            length: 1000
        }
    );
    assert_eq!(
        map.intervals()[1],
        ExtentInterval {
            offset: 3000,
            length: 1000
        }
    );
    assert_eq!(
        map.intervals()[2],
        ExtentInterval {
            offset: 5000,
            length: 1000
        }
    );
    assert_eq!(map.total_failed_bytes(), 3000);
}

/// TEST-RECO-06: ExtentMap Inversion (Recovered vs Missing Byte Slices)
#[test]
fn test_reco_06_extent_inversion_complement() {
    let mut map = ExtentMap::new();
    map.add_extent(2000, 1000); // [2000, 3000)
    map.add_extent(7000, 1000); // [7000, 8000)

    let total_size = 10_000u64;
    let recovered = map.invert(total_size);

    assert_eq!(recovered.len(), 3);
    assert_eq!(
        recovered[0],
        ExtentInterval {
            offset: 0,
            length: 2000
        }
    );
    assert_eq!(
        recovered[1],
        ExtentInterval {
            offset: 3000,
            length: 4000
        }
    );
    assert_eq!(
        recovered[2],
        ExtentInterval {
            offset: 8000,
            length: 2000
        }
    );

    let recovered_sum: u64 = recovered.iter().map(|i| i.length).sum();
    assert_eq!(recovered_sum + map.total_failed_bytes(), total_size);
}

/// TEST-RECO-07: Error Queue Quarantine Isolation
#[test]
fn test_reco_07_error_queue_quarantine_isolation() {
    let queue = ErrorQueue::new();

    let mut extents = ExtentMap::new();
    extents.add_extent(4096, 4096);
    let bad_file = create_quarantined_file(2, "photos/corrupted.raw", 50_000, extents);

    queue.quarantine_file(bad_file).unwrap();

    assert_eq!(queue.count(), 1);
    assert_eq!(queue.count_active_quarantine(), 1);

    let retrieved = queue.get_file(2).unwrap();
    assert_eq!(retrieved.file_id, 2);
    assert_eq!(
        retrieved.relative_path,
        PathBuf::from("photos/corrupted.raw")
    );
    assert_eq!(retrieved.state, QuarantineState::Quarantined);
}

/// TEST-RECO-08: Interactive Action SkipFile
#[test]
fn test_reco_08_interactive_action_skip_file() {
    let queue = Arc::new(ErrorQueue::new());
    let budget = RetryBudget::default();
    let controller = RecoveryController::new(Arc::clone(&queue), budget);

    let mut extents = ExtentMap::new();
    extents.add_extent(0, 1024);
    let f2 = create_quarantined_file(2, "bad_file.dat", 10_000, extents);
    queue.quarantine_file(f2).unwrap();

    let res = controller
        .apply_action(InteractiveAction::SkipFile {
            file_id: 2,
            reason: Some("Unreadable sectors".to_string()),
        })
        .unwrap();

    assert_eq!(res.len(), 1);
    assert_eq!(res[0].state, QuarantineState::Skipped);

    let updated = queue.get_file(2).unwrap();
    assert_eq!(updated.state, QuarantineState::Skipped);
}

/// TEST-RECO-09: Interactive Action RetryFile with Budget Reset
#[test]
fn test_reco_09_interactive_action_retry_file_budget_reset() {
    let queue = Arc::new(ErrorQueue::new());
    let mut budget = RetryBudget::default();

    // Exhaust budget for file 2
    for _ in 0..3 {
        let _ = budget.record_file_attempt(2);
    }
    assert!(budget.record_file_attempt(2).is_err()); // exhausted!

    let controller = RecoveryController::new(Arc::clone(&queue), budget);

    let mut extents = ExtentMap::new();
    extents.add_extent(0, 1024);
    let mut f2 = create_quarantined_file(2, "bad_file.dat", 10_000, extents);
    f2.attempts = 3;
    queue.quarantine_file(f2).unwrap();

    let res = controller
        .apply_action(InteractiveAction::RetryFile {
            file_id: 2,
            reset_budget: true,
        })
        .unwrap();

    assert_eq!(res.len(), 1);
    assert_eq!(res[0].state, QuarantineState::Retrying);

    let updated = queue.get_file(2).unwrap();
    assert_eq!(updated.state, QuarantineState::Retrying);
}

/// TEST-RECO-10: Interactive Action DeferFile (Second-Pass Queueing)
#[test]
fn test_reco_10_interactive_action_defer_file() {
    let queue = Arc::new(ErrorQueue::new());
    let budget = RetryBudget::default();
    let controller = RecoveryController::new(Arc::clone(&queue), budget);

    let mut extents = ExtentMap::new();
    extents.add_extent(100, 200);
    let f_slow = create_quarantined_file(10, "slow_file.bin", 50_000, extents);
    queue.quarantine_file(f_slow).unwrap();

    let res = controller
        .apply_action(InteractiveAction::DeferFile { file_id: 10 })
        .unwrap();

    assert_eq!(res.len(), 1);
    assert_eq!(res[0].state, QuarantineState::Deferred);

    let deferred_list = queue.list_deferred();
    assert_eq!(deferred_list.len(), 1);
    assert_eq!(deferred_list[0].file_id, 10);
}

/// TEST-RECO-11: Interactive Action SkipAll Bulk Remediation
#[test]
fn test_reco_11_interactive_action_skip_all_bulk() {
    let queue = Arc::new(ErrorQueue::new());
    let budget = RetryBudget::default();
    let controller = RecoveryController::new(Arc::clone(&queue), budget);

    for i in 1..=5 {
        let mut ext = ExtentMap::new();
        ext.add_extent(0, 512);
        let f = create_quarantined_file(i, &format!("file_{}.dat", i), 10_000, ext);
        queue.quarantine_file(f).unwrap();
    }
    assert_eq!(queue.count_active_quarantine(), 5);

    let skipped = controller.apply_action(InteractiveAction::SkipAll).unwrap();
    assert_eq!(skipped.len(), 5);
    assert_eq!(queue.count_active_quarantine(), 0);

    for f in queue.list_all() {
        assert_eq!(f.state, QuarantineState::Skipped);
    }
}

/// TEST-RECO-12: Partial File Sidecar Manifest Generation
#[test]
fn test_reco_12_partial_file_sidecar_generation() {
    let dir = tempdir().unwrap();
    let sidecar_path = dir.path().join(".volterra.movie.mkv.sidecar.json");

    let mut failed = ExtentMap::new();
    failed.add_extent(2 * 1024 * 1024, 1024 * 1024); // 1 MiB bad sector

    let total_size = 8 * 1024 * 1024u64; // 8 MiB
    let sidecar = SidecarManifest::create(
        "job_sidecar".to_string(),
        PathBuf::from("videos/movie.mkv"),
        total_size,
        &failed,
        HoleFillPolicy::ZeroFilled,
    );

    assert!(!sidecar.is_complete);
    assert!(sidecar.is_partial_recovery);
    assert!(!sidecar.checksum_verified);
    assert_eq!(sidecar.total_missing_bytes, 1024 * 1024);
    assert_eq!(sidecar.total_recovered_bytes, 7 * 1024 * 1024);
    assert!((sidecar.recovery_percentage - 87.5).abs() < 0.01);

    sidecar.write_to_file(&sidecar_path).unwrap();
    assert!(sidecar_path.exists());

    let parsed = SidecarManifest::read_from_file(&sidecar_path).unwrap();
    assert!(!parsed.is_complete);
    assert_eq!(parsed.total_missing_bytes, 1024 * 1024);
    assert_eq!(parsed.total_recovered_bytes, 7 * 1024 * 1024);
}

/// TEST-RECO-13: Retry Budget Exponential Backoff with Jitter
#[test]
fn test_reco_13_retry_budget_exponential_backoff() {
    let policy = RetryPolicy {
        max_retries_per_file: 3,
        max_job_retries: 50,
        base_delay_ms: 100,
        max_delay_ms: 1000,
        backoff_multiplier: 2.0,
        jitter_factor: 0.2,
    };
    let mut budget = RetryBudget::new(policy);

    // Attempt 1: ~100ms
    let d1 = budget.record_attempt().unwrap();
    assert!(d1.as_millis() >= 90 && d1.as_millis() <= 130);

    // Attempt 2: ~200ms
    let d2 = budget.record_attempt().unwrap();
    assert!(d2.as_millis() >= 180 && d2.as_millis() <= 260);

    // Attempt 3: ~400ms
    let d3 = budget.record_attempt().unwrap();
    assert!(d3.as_millis() >= 360 && d3.as_millis() <= 500);

    // Attempt 4: Budget exhausted!
    let d4 = budget.record_attempt();
    assert!(d4.is_err());
}

/// TEST-RECO-14: Fail-Closed Completion Barrier
#[test]
fn test_reco_14_fail_closed_barrier() {
    let total_size = 10 * 1024 * 1024u64; // 10 MiB
    let verified_bytes = 8 * 1024 * 1024u64; // Only 8 MiB verified

    // Attempting publication on incomplete file must fail!
    let barrier_res = SidecarManifest::enforce_fail_closed_barrier(total_size, verified_bytes, 1);
    assert!(barrier_res.is_err());

    // When fully complete and zero missing extents:
    let pass_res = SidecarManifest::enforce_fail_closed_barrier(total_size, total_size, 0);
    assert!(pass_res.is_ok());
}

/// TEST-RECO-15: Read-Only Source Invariant Guard
#[test]
fn test_reco_15_read_only_source_invariant() {
    let dir = tempdir().unwrap();
    let src_file = dir.path().join("source.txt");
    {
        let mut f = File::create(&src_file).unwrap();
        f.write_all(b"Original read-only source content").unwrap();
    }

    let initial_meta = std::fs::metadata(&src_file).unwrap();
    let initial_mtime = initial_meta.modified().unwrap();
    let initial_len = initial_meta.len();

    // Perform recovery operations and simulations
    let mut extents = ExtentMap::new();
    extents.add_extent(0, 10);
    let queue = ErrorQueue::new();
    let qf = create_quarantined_file(1, "source.txt", initial_len, extents);
    queue.quarantine_file(qf).unwrap();

    let controller = RecoveryController::new(Arc::new(queue), RetryBudget::default());
    let _ = controller.apply_action(InteractiveAction::SkipFile {
        file_id: 1,
        reason: Some("test skip".to_string()),
    });

    // Verify source file was NEVER modified
    let after_meta = std::fs::metadata(&src_file).unwrap();
    assert_eq!(after_meta.modified().unwrap(), initial_mtime);
    assert_eq!(after_meta.len(), initial_len);

    let content = std::fs::read(&src_file).unwrap();
    assert_eq!(content, b"Original read-only source content");
}

/// Additional tests from explorer blueprints:
#[test]
fn test_extent_validity_and_bounds() {
    assert!(ExtentInterval::new(0, 0).is_err());
    assert!(ExtentInterval::new(u64::MAX - 10, 20).is_err());
    assert!(ExtentInterval::new(100, 50).is_ok());
}

#[test]
fn test_extent_contains_and_overlaps() {
    let e1 = ExtentInterval::new(100, 200).unwrap(); // [100, 300)
    assert!(e1.contains_offset(100));
    assert!(e1.contains_offset(299));
    assert!(!e1.contains_offset(300));
    assert!(!e1.contains_offset(99));

    let e2 = ExtentInterval::new(250, 100).unwrap(); // [250, 350)
    assert!(e1.overlaps(&e2));
    assert!(e2.overlaps(&e1));

    let e3 = ExtentInterval::new(300, 50).unwrap(); // adjacent [300, 350)
    assert!(!e1.overlaps(&e3));
    assert!(e1.is_contiguous(&e3));
    assert!(e1.overlaps_or_adjacent(&e3));
}

#[test]
fn test_extent_set_multi_range_merge() {
    let mut map = ExtentMap::new();
    map.add_extent(1000, 1000); // [1000, 2000)
    map.add_extent(3000, 1000); // [3000, 4000)
    map.add_extent(5000, 1000); // [5000, 6000)
    assert_eq!(map.count(), 3);

    // Insert a wide range that spans across all 3
    map.add_extent(1500, 4000); // [1500, 5500)
    assert_eq!(map.count(), 1);
    assert_eq!(
        map.intervals()[0],
        ExtentInterval {
            offset: 1000,
            length: 5000
        }
    );
}

#[test]
fn test_extent_set_complement_full_unreadable() {
    let mut map = ExtentMap::new();
    map.add_extent(0, 1000);
    let comp = map.invert(1000);
    assert!(comp.is_empty());
}

#[test]
fn test_extent_set_complement_zero_errors() {
    let map = ExtentMap::new();
    let comp = map.invert(5000);
    assert_eq!(comp.len(), 1);
    assert_eq!(
        comp[0],
        ExtentInterval {
            offset: 0,
            length: 5000
        }
    );
}

#[test]
fn test_classifier_posix_eacces_not_bad_sector() {
    let classified = RecoveryClassifier::classify_errno(
        libc::EACCES,
        ErrorDomain::Source,
        "sandbox restriction",
    );
    assert_ne!(classified.category, ErrorCategory::DeviceIo);
    assert_eq!(classified.category, ErrorCategory::PermissionDenied);
    assert!(!classified.is_retryable);
}

#[test]
fn test_interactive_retry_all_bulk() {
    let queue = Arc::new(ErrorQueue::new());
    let mut budget = RetryBudget::default();

    for i in 1..=3 {
        let mut ext = ExtentMap::new();
        ext.add_extent(0, 100);
        let mut f = create_quarantined_file(i, &format!("f_{}.bin", i), 1000, ext);
        f.attempts = 3;
        queue.quarantine_file(f).unwrap();
        let _ = budget.record_file_attempt(i);
    }

    let controller = RecoveryController::new(Arc::clone(&queue), budget);
    let retrying = controller
        .apply_action(InteractiveAction::RetryAll)
        .unwrap();

    assert_eq!(retrying.len(), 3);
    for f in retrying {
        assert_eq!(f.state, QuarantineState::Retrying);
    }
}
