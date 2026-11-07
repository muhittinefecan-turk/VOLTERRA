//! Fault Injection USB Disconnect & Volume UUID Utilities & Tests (TEST-06)

// =========================================================================
// TEST-06: USB Disconnect (ENODEV) Volume UUID Handling
// =========================================================================
pub fn run_test_06_usb_disconnect_enodev_and_volume_uuid_reconnection() {
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    use tempfile::tempdir;
    use volterra_journal::{JobMode, JobRecord, JobState, JournalEngine};
    use volterra_manifest::{ManifestScanner, ScanOptions};
    use volterra_platform_macos::{PlatformVolume, VolumeUuid};
    use volterra_profiler::PerformanceMonitor;
    use volterra_recovery::{
        ErrorCategory, ErrorSeverity, RecoveryClassifier, RecoveryController, RetryBudget,
    };
    use volterra_transfer::{
        TransferError, TransferScheduler, TransferSchedulerConfig, WriteFaultInjector,
    };

    let src = tempdir().unwrap();
    let dst = tempdir().unwrap();
    let jnl_dir = tempdir().unwrap();

    let src_file = src.path().join("usb_data.bin");
    std::fs::write(&src_file, vec![0x33u8; 128 * 1024]).unwrap();

    let manifest = ManifestScanner::scan_tree(src.path(), &ScanOptions::default()).unwrap();

    // 1. Query real Darwin Volume UUID of source and target
    let target_uuid =
        PlatformVolume::get_volume_uuid(dst.path()).unwrap_or_else(|_| VolumeUuid([0xAA; 16]));

    // 2. Classify ENODEV / ENXIO syscall error
    let disconnect_err = std::io::Error::from_raw_os_error(libc::ENODEV);
    let classified = RecoveryClassifier::classify_io_error(
        &disconnect_err,
        volterra_recovery::ErrorDomain::Destination,
    );
    assert_eq!(classified.category, ErrorCategory::DeviceDisconnected);
    assert_eq!(classified.severity, ErrorSeverity::JobFatal);
    assert!(!classified.is_retryable);
    assert_eq!(
        classified.suggested_action,
        volterra_recovery::ActionRecommendation::MountDisk
    );

    // 3. Simulate Volume UUID Mismatch on candidate reconnection:
    // A different USB thumbdrive was plugged in with different UUID
    let bogus_uuid = VolumeUuid([0xBB; 16]);
    assert_ne!(
        bogus_uuid, target_uuid,
        "Bogus UUID must not match target volume UUID"
    );

    // Verification check for reconnection:
    let is_same_volume = |candidate_path: &std::path::Path, expected: &VolumeUuid| -> bool {
        match PlatformVolume::get_volume_uuid(candidate_path) {
            Ok(u) => u == *expected,
            Err(_) => false,
        }
    };

    assert!(
        is_same_volume(dst.path(), &target_uuid),
        "Target directory must match its own UUID"
    );
    assert!(
        !is_same_volume(dst.path(), &bogus_uuid),
        "Bogus UUID must be rejected"
    );

    // 4. Inject ENODEV into scheduler to verify clean failure without false completion
    struct DisconnectInjector;
    impl WriteFaultInjector for DisconnectInjector {
        fn on_pwrite(&self, _offset: u64, _data: &[u8]) -> Option<Result<usize, std::io::Error>> {
            Some(Err(std::io::Error::from_raw_os_error(libc::ENODEV)))
        }
    }

    let db_path = jnl_dir.path().join("journal.db");
    let journal = Arc::new(JournalEngine::open(&db_path).unwrap());
    let job_id = "job_usb_disconnect_06".to_string();

    journal
        .create_job(&JobRecord {
            id: job_id.clone(),
            source_root: src.path().to_path_buf(),
            target_root: dst.path().to_path_buf(),
            source_volume_uuid: None,
            target_volume_uuid: Some(target_uuid.to_string_formatted()),
            mode: JobMode::SmartHybrid,
            state: JobState::WaitingForDisk,
            total_files: 1,
            total_bytes: 128 * 1024,
            copied_bytes: 0,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            completed_at: None,
        })
        .unwrap();

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

    let recovery = Arc::new(RecoveryController::with_journal(
        Arc::new(volterra_recovery::ErrorQueue::new()),
        RetryBudget::default(),
        journal.clone(),
    ));

    let scheduler = TransferScheduler::new(
        config,
        journal,
        recovery,
        Arc::new(PerformanceMonitor::new()),
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    )
    .with_write_fault_injector(Arc::new(DisconnectInjector));

    let res = scheduler.execute_pipeline(&manifest, |_, _, _, _| {});

    // Must return DestinationWriteError wrapping ENODEV
    assert!(res.is_err());
    match res.unwrap_err() {
        TransferError::DestinationWriteError { source, .. } => {
            assert_eq!(source.raw_os_error(), Some(libc::ENODEV));
        }
        other => panic!("Expected DestinationWriteError with ENODEV, got {:?}", other),
    }

    // Final file was never published
    assert!(!dst.path().join("usb_data.bin").exists());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_06_usb_disconnect_enodev_and_volume_uuid_reconnection() {
        run_test_06_usb_disconnect_enodev_and_volume_uuid_reconnection();
    }
}
