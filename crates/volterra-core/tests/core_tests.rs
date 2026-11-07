use std::time::{Duration, Instant};
use tempfile::tempdir;
use volterra_core::{CoreError, EventHub, JobBuilder, JobEvent, JobLifecycleState};
use volterra_journal::JobMode;

#[test]
fn test_job_lifecycle_state_transitions() {
    let pending = JobLifecycleState::Pending;
    assert!(pending.can_transition_to(JobLifecycleState::Scanning));
    assert!(!pending.can_transition_to(JobLifecycleState::Running));
    assert!(pending.can_transition_to(JobLifecycleState::Cancelled));
    assert!(!pending.can_transition_to(JobLifecycleState::Completed));

    let running = JobLifecycleState::Running;
    assert!(running.can_transition_to(JobLifecycleState::Paused));
    assert!(running.can_transition_to(JobLifecycleState::Completed));
    assert!(running.can_transition_to(JobLifecycleState::Failed));
    assert!(running.can_transition_to(JobLifecycleState::Cancelled));
    assert!(!running.can_transition_to(JobLifecycleState::Pending));

    let paused = JobLifecycleState::Paused;
    assert!(paused.can_transition_to(JobLifecycleState::Running));
    assert!(paused.can_transition_to(JobLifecycleState::Cancelled));
    assert!(!paused.can_transition_to(JobLifecycleState::Completed));

    let completed = JobLifecycleState::Completed;
    assert!(completed.is_terminal());
    assert!(!completed.can_transition_to(JobLifecycleState::Running));
    assert!(!completed.can_transition_to(JobLifecycleState::Pending));
    assert!(!completed.can_transition_to(JobLifecycleState::Completed));
}

#[test]
fn test_event_hub_nonblocking_broadcast() {
    let hub = EventHub::new();
    let rx1 = hub.subscribe(10);
    let rx2 = hub.subscribe(10);

    assert_eq!(hub.subscriber_count(), 2);

    hub.broadcast(JobEvent::StateChanged {
        old_state: JobLifecycleState::Pending,
        new_state: JobLifecycleState::Running,
    });

    let ev1 = rx1
        .recv_timeout(Duration::from_millis(500))
        .expect("rx1 receive");
    let ev2 = rx2
        .recv_timeout(Duration::from_millis(500))
        .expect("rx2 receive");

    match (ev1, ev2) {
        (
            JobEvent::StateChanged { new_state: s1, .. },
            JobEvent::StateChanged { new_state: s2, .. },
        ) => {
            assert_eq!(s1, JobLifecycleState::Running);
            assert_eq!(s2, JobLifecycleState::Running);
        }
        _ => panic!("Expected StateChanged event"),
    }

    // Drop rx1 and broadcast again; rx1 should be cleaned up automatically
    drop(rx1);
    hub.broadcast(JobEvent::DiskWarning {
        message: "Warning test".into(),
    });

    let ev = rx2
        .recv_timeout(Duration::from_millis(500))
        .expect("rx2 receive");
    match ev {
        JobEvent::DiskWarning { message } => assert_eq!(message, "Warning test"),
        _ => panic!("Expected DiskWarning"),
    }
    assert_eq!(hub.subscriber_count(), 1);
}

#[test]
fn test_job_builder_validation() {
    let res_no_source = JobBuilder::new().destination("/tmp/dest").build();
    match res_no_source {
        Err(CoreError::MissingParameter(param)) => assert_eq!(param, "source_root"),
        other => panic!("Expected MissingParameter, got {:?}", other),
    }

    let res_no_dest = JobBuilder::new().source("/tmp").build();
    match res_no_dest {
        Err(CoreError::MissingParameter(param)) => assert_eq!(param, "target_root"),
        other => panic!("Expected MissingParameter, got {:?}", other),
    }

    let res_bad_src = JobBuilder::new()
        .source("/non_existent_volterra_path_xyz123")
        .destination("/tmp")
        .build();
    match res_bad_src {
        Err(CoreError::SourceNotFound(_)) => {}
        other => panic!("Expected SourceNotFound, got {:?}", other),
    }
}

#[test]
fn test_job_handle_lifecycle_controls() {
    let src = tempdir().unwrap();
    let dst = tempdir().unwrap();

    let handle = JobBuilder::new()
        .source(src.path())
        .destination(dst.path())
        .mode(JobMode::SmartHybrid)
        .build()
        .expect("Build job handle");

    assert_eq!(handle.state(), JobLifecycleState::Pending);

    // Cancel from pending
    handle.cancel().expect("Cancel job");
    assert_eq!(handle.state(), JobLifecycleState::Cancelled);

    // Cannot resume from cancelled
    assert!(handle.resume().is_err());
}

#[test]
fn test_interactive_error_actions_on_handle() {
    let src = tempdir().unwrap();
    let dst = tempdir().unwrap();

    let handle = JobBuilder::new()
        .source(src.path())
        .destination(dst.path())
        .mode(JobMode::SafeRecovery)
        .build()
        .expect("Build job handle");

    // Add a quarantined file to queue to test actions
    let qf = volterra_recovery::QuarantinedFile {
        file_id: 100,
        job_id: handle.id().to_string(),
        relative_path: "failing.dat".into(),
        total_size_bytes: 1024,
        failed_extents: volterra_recovery::ExtentMap::new(),
        last_error: volterra_recovery::RecoveryClassifier::classify_errno(
            libc::EIO,
            volterra_recovery::ErrorDomain::Source,
            "Bad sector",
        ),
        state: volterra_recovery::QuarantineState::Quarantined,
        attempts: 1,
        quarantined_at: 1000,
    };
    handle.recovery().queue().quarantine_file(qf).unwrap();

    // Test calling interactive recovery methods on handle
    let skip_res = handle.skip_file(100);
    assert!(skip_res.is_ok());

    let retry_res = handle.retry_file(100);
    assert!(retry_res.is_ok());

    let skip_all_res = handle.skip_all_errors();
    assert!(skip_all_res.is_ok());

    let retry_all_res = handle.retry_all_failed();
    assert!(retry_all_res.is_ok());
}

#[test]
fn test_job_execution_end_to_end_with_events_and_telemetry() {
    let src = tempdir().unwrap();
    let dst = tempdir().unwrap();

    // Create synthetic files
    std::fs::write(
        src.path().join("alpha.txt"),
        b"Data Alpha for Volterra Core",
    )
    .unwrap();
    std::fs::create_dir_all(src.path().join("subdir")).unwrap();
    std::fs::write(src.path().join("subdir/beta.bin"), vec![0x77u8; 128 * 1024]).unwrap();

    let handle = JobBuilder::new()
        .source(src.path())
        .destination(dst.path())
        .mode(JobMode::SmartHybrid)
        .chunk_size(32 * 1024)
        .memory_budget(32 * 1024 * 1024)
        .build()
        .expect("Build job");

    let rx = handle.subscribe_events(50);
    handle.start().expect("Start job");

    // Listen for completion event
    let mut completed = false;
    let timeout = Instant::now() + Duration::from_secs(10);

    while Instant::now() < timeout {
        if let Ok(JobEvent::JobCompleted {
            total_files,
            total_bytes,
            ..
        }) = rx.recv_timeout(Duration::from_millis(100))
        {
            assert!(total_files >= 2);
            assert!(total_bytes > 0);
            completed = true;
            break;
        }
    }

    assert!(
        completed,
        "Job should emit JobCompleted event within deadline"
    );

    // Verify snapshot
    let snap = handle.snapshot();
    assert_eq!(snap.state, JobLifecycleState::Completed);
    assert_eq!(snap.percent_complete, 100.0);
    assert_eq!(snap.remaining_bytes, 0);

    // Verify destination files exist
    assert!(dst.path().join("alpha.txt").exists());
    assert!(dst.path().join("subdir/beta.bin").exists());
    assert_eq!(
        std::fs::read(dst.path().join("alpha.txt")).unwrap(),
        b"Data Alpha for Volterra Core"
    );
    assert_eq!(
        std::fs::read(dst.path().join("subdir/beta.bin")).unwrap(),
        vec![0x77u8; 128 * 1024]
    );
}

#[test]
fn test_out_of_order_lifecycle_resume_and_cancel() {
    let src = tempdir().unwrap();
    let dst = tempdir().unwrap();
    std::fs::write(src.path().join("test.txt"), b"sample data").unwrap();

    let handle = JobBuilder::new()
        .source(src.path())
        .destination(dst.path())
        .mode(JobMode::SmartHybrid)
        .build()
        .unwrap();

    // In Pending state: resume must fail with InvalidStateTransition
    let resume_err = handle.resume();
    assert!(resume_err.is_err());
    match resume_err {
        Err(CoreError::InvalidStateTransition { current, target }) => {
            assert_eq!(current, JobLifecycleState::Pending);
            assert_eq!(target, JobLifecycleState::Running);
        }
        other => panic!("Expected InvalidStateTransition, got {:?}", other),
    }

    // State must remain Pending
    assert_eq!(handle.state(), JobLifecycleState::Pending);

    // Cancel must succeed
    assert!(handle.cancel().is_ok());
    assert_eq!(handle.state(), JobLifecycleState::Cancelled);

    // Cancel again must be idempotent and succeed
    assert!(handle.cancel().is_ok());
    assert_eq!(handle.state(), JobLifecycleState::Cancelled);
}
