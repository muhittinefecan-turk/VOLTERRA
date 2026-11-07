//! Comprehensive integration and acceptance tests for `volterra-ffi`.
//!
//! Validates C ABI invariants: pointer ownership, null-safety, memory cleanup,
//! non-blocking event dispatching, panic shielding, interactive recovery, and reporting.

use std::ffi::{CStr, CString};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tempfile::tempdir;
use volterra_ffi::*;

#[test]
fn test_null_pointer_safety() {
    unsafe {
        let mut err: *mut volterra_error_t = std::ptr::null_mut();

        // 1. volterra_job_create with null config
        let job = volterra_job_create(std::ptr::null(), &mut err);
        assert!(job.is_null());
        assert!(!err.is_null());
        assert_eq!((*err).code, volterra_error_code::ErrInvalidArgument as i32);
        volterra_error_free(err);
        err = std::ptr::null_mut();

        // 2. Lifecycle controls with null job handle
        let ret = volterra_job_start(std::ptr::null_mut(), &mut err);
        assert_eq!(ret, volterra_error_code::ErrInvalidArgument as i32);
        assert!(!err.is_null());
        volterra_error_free(err);

        assert_eq!(
            volterra_job_pause(std::ptr::null_mut(), std::ptr::null_mut()),
            volterra_error_code::ErrInvalidArgument as i32
        );
        assert_eq!(
            volterra_job_resume(std::ptr::null_mut(), std::ptr::null_mut()),
            volterra_error_code::ErrInvalidArgument as i32
        );
        assert_eq!(
            volterra_job_cancel(std::ptr::null_mut(), std::ptr::null_mut()),
            volterra_error_code::ErrInvalidArgument as i32
        );

        // 3. Telemetry and snapshots with null job
        assert!(volterra_get_snapshot(std::ptr::null_mut()).is_null());
        assert!(volterra_get_telemetry(std::ptr::null_mut()).is_null());
        assert!(volterra_get_errors(std::ptr::null_mut()).is_null());
        assert!(volterra_export_report(std::ptr::null_mut(), 0).is_null());

        // 4. Interactive error actions with null job
        assert_eq!(
            volterra_skip_file(std::ptr::null_mut(), 1),
            volterra_error_code::ErrInvalidArgument as i32
        );
        assert_eq!(
            volterra_retry_file(std::ptr::null_mut(), 1),
            volterra_error_code::ErrInvalidArgument as i32
        );
        assert_eq!(
            volterra_defer_file(std::ptr::null_mut(), 1),
            volterra_error_code::ErrInvalidArgument as i32
        );
        assert_eq!(
            volterra_skip_all_errors(std::ptr::null_mut()),
            volterra_error_code::ErrInvalidArgument as i32
        );
        assert_eq!(
            volterra_retry_all_failed(std::ptr::null_mut()),
            volterra_error_code::ErrInvalidArgument as i32
        );

        // 5. Deallocators with null pointers (must be safe no-ops)
        volterra_job_free(std::ptr::null_mut());
        volterra_snapshot_free(std::ptr::null_mut());
        volterra_telemetry_free(std::ptr::null_mut());
        volterra_error_list_free(std::ptr::null_mut());
        volterra_disks_free(std::ptr::null_mut());
        volterra_string_free(std::ptr::null_mut());
        volterra_error_free(std::ptr::null_mut());
    }
}

#[test]
fn test_job_lifecycle_and_telemetry() {
    let src_dir = tempdir().unwrap();
    let dst_dir = tempdir().unwrap();

    for i in 0..5 {
        let file = src_dir.path().join(format!("payload_{}.bin", i));
        std::fs::write(&file, vec![0xAB; 2 * 1024 * 1024]).unwrap();
    }

    let src_c = CString::new(src_dir.path().to_str().unwrap()).unwrap();
    let dst_c = CString::new(dst_dir.path().to_str().unwrap()).unwrap();

    let config = volterra_config_t {
        source_path: src_c.as_ptr(),
        dest_path: dst_c.as_ptr(),
        mode: volterra_mode::SmartHybrid as u32,
        chunk_size: 1024 * 1024,
        direct_io: false,
        verify_readback: true,
        memory_budget: 32 * 1024 * 1024,
        allow_overwrite: true,
        journal_path: std::ptr::null(),
    };

    unsafe {
        let mut err: *mut volterra_error_t = std::ptr::null_mut();
        let job = volterra_job_create(&config, &mut err);
        assert!(!job.is_null());
        assert!(err.is_null());

        // Snapshot in pending state
        let snap = volterra_get_snapshot(job);
        assert!(!snap.is_null());
        assert_eq!((*snap).state, volterra_state::Pending as u32);
        assert!(!(*snap).job_id.is_null());
        volterra_snapshot_free(snap);

        // Telemetry in initial state
        let tel = volterra_get_telemetry(job);
        assert!(!tel.is_null());
        assert_eq!((*tel).active_concurrency, 1);
        assert!(!(*tel).bottleneck_message.is_null());
        volterra_telemetry_free(tel);

        // Start job
        let start_res = volterra_job_start(job, &mut err);
        assert_eq!(start_res, 0);
        assert!(err.is_null());

        // Wait for Running state
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            let st = (*job).handle.state();
            if st == volterra_core::JobLifecycleState::Running {
                // Pause job
                let pause_res = volterra_job_pause(job, &mut err);
                assert_eq!(pause_res, 0);
                assert!(err.is_null());

                // Resume job
                let resume_res = volterra_job_resume(job, &mut err);
                assert_eq!(resume_res, 0);
                assert!(err.is_null());
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        // Cancel job if not finished
        if (*job)
            .handle
            .state()
            .can_transition_to(volterra_core::JobLifecycleState::Cancelled)
        {
            let cancel_res = volterra_job_cancel(job, &mut err);
            assert_eq!(cancel_res, 0);
            assert!(err.is_null());
        }

        // Free job
        volterra_job_free(job);
    }
}

unsafe extern "C" fn test_event_callback(
    event: *const volterra_event_t,
    user_data: *mut libc::c_void,
) {
    if event.is_null() || user_data.is_null() {
        return;
    }
    let counter = &*(user_data as *const AtomicUsize);
    counter.fetch_add(1, Ordering::SeqCst);
}

#[test]
fn test_event_subscription_streaming() {
    let src_dir = tempdir().unwrap();
    let dst_dir = tempdir().unwrap();

    let file_a = src_dir.path().join("file1.bin");
    std::fs::write(&file_a, b"hello volterra streaming").unwrap();

    let src_c = CString::new(src_dir.path().to_str().unwrap()).unwrap();
    let dst_c = CString::new(dst_dir.path().to_str().unwrap()).unwrap();

    let config = volterra_config_t {
        source_path: src_c.as_ptr(),
        dest_path: dst_c.as_ptr(),
        mode: volterra_mode::SmartHybrid as u32,
        chunk_size: 1024 * 1024,
        direct_io: false,
        verify_readback: false,
        memory_budget: 16 * 1024 * 1024,
        allow_overwrite: true,
        journal_path: std::ptr::null(),
    };

    unsafe {
        let mut err: *mut volterra_error_t = std::ptr::null_mut();
        let job = volterra_job_create(&config, &mut err);
        assert!(!job.is_null());

        let event_count = Arc::new(AtomicUsize::new(0));
        let user_data = Arc::as_ptr(&event_count) as *mut libc::c_void;

        let sub_res = volterra_subscribe_events(job, Some(test_event_callback), user_data);
        assert_eq!(sub_res, 0);

        // Start job to trigger events
        let _ = volterra_job_start(job, &mut err);

        // Allow background thread to process events
        std::thread::sleep(std::time::Duration::from_millis(200));

        let received = event_count.load(Ordering::SeqCst);
        assert!(received > 0, "Expected at least 1 event, got {}", received);

        volterra_job_free(job);
    }
}

#[test]
fn test_interactive_error_actions_and_quarantine() {
    let src_dir = tempdir().unwrap();
    let dst_dir = tempdir().unwrap();

    let file_a = src_dir.path().join("file.txt");
    std::fs::write(&file_a, b"content").unwrap();

    let src_c = CString::new(src_dir.path().to_str().unwrap()).unwrap();
    let dst_c = CString::new(dst_dir.path().to_str().unwrap()).unwrap();

    let config = volterra_config_t {
        source_path: src_c.as_ptr(),
        dest_path: dst_c.as_ptr(),
        mode: volterra_mode::SafeRecovery as u32,
        chunk_size: 1024 * 1024,
        direct_io: false,
        verify_readback: false,
        memory_budget: 16 * 1024 * 1024,
        allow_overwrite: true,
        journal_path: std::ptr::null(),
    };

    unsafe {
        let mut err: *mut volterra_error_t = std::ptr::null_mut();
        let job = volterra_job_create(&config, &mut err);
        assert!(!job.is_null());

        // Simulate isolating a failing file into the recovery queue
        let mut extent_map = volterra_recovery::ExtentMap::new();
        extent_map.add_extent(0, 4096);
        extent_map.add_extent(8192, 4096);

        let classified = volterra_recovery::ClassifiedError {
            category: volterra_recovery::ErrorCategory::DeviceIo,
            domain: volterra_recovery::ErrorDomain::Source,
            severity: volterra_recovery::ErrorSeverity::FileSpecific,
            errno_code: Some(libc::EIO),
            message: "I/O hardware read error".to_string(),
            is_retryable: true,
            suggested_action: volterra_recovery::ActionRecommendation::QuarantineAndProceed,
        };

        let q_file = volterra_recovery::QuarantinedFile {
            file_id: 42,
            job_id: (*job).handle.id().to_string(),
            relative_path: std::path::PathBuf::from("damaged/corrupt.dat"),
            total_size_bytes: 16384,
            failed_extents: extent_map,
            last_error: classified,
            state: volterra_recovery::QuarantineState::Quarantined,
            attempts: 1,
            quarantined_at: 1000,
        };

        (*job)
            .handle
            .recovery()
            .queue()
            .quarantine_file(q_file)
            .unwrap();

        // 1. volterra_get_errors
        let error_list = volterra_get_errors(job);
        assert!(!error_list.is_null());
        assert_eq!((*error_list).count, 1);

        let item = &*(*error_list).items;
        assert_eq!(item.file_id, 42);
        assert_eq!(item.total_size_bytes, 16384);
        assert_eq!(item.failed_extents_count, 2);

        let path_str = CStr::from_ptr(item.path).to_str().unwrap();
        assert_eq!(path_str, "damaged/corrupt.dat");

        let err_msg = CStr::from_ptr(item.error_message).to_str().unwrap();
        assert!(err_msg.contains("I/O hardware"));

        // Check extents array
        let extents = std::slice::from_raw_parts(item.extents, item.failed_extents_count);
        assert_eq!(extents[0].offset, 0);
        assert_eq!(extents[0].length, 4096);
        assert_eq!(extents[0].error_code, libc::EIO);
        assert_eq!(extents[1].offset, 8192);
        assert_eq!(extents[1].length, 4096);

        volterra_error_list_free(error_list);

        // 2. volterra_retry_file
        let ret_retry = volterra_retry_file(job, 42);
        assert_eq!(ret_retry, 0);

        // 3. volterra_defer_file
        let ret_defer = volterra_defer_file(job, 42);
        assert_eq!(ret_defer, 0);

        // 4. volterra_skip_file
        let ret_skip = volterra_skip_file(job, 42);
        assert_eq!(ret_skip, 0);

        // 5. volterra_skip_all_errors
        let ret_skip_all = volterra_skip_all_errors(job);
        assert_eq!(ret_skip_all, 0);

        // 6. volterra_retry_all_failed
        let ret_retry_all = volterra_retry_all_failed(job);
        assert_eq!(ret_retry_all, 0);

        volterra_job_free(job);
    }
}

#[test]
fn test_export_report_json_and_csv() {
    let src_dir = tempdir().unwrap();
    let dst_dir = tempdir().unwrap();

    let file_a = src_dir.path().join("report_test.bin");
    std::fs::write(&file_a, b"test report generation").unwrap();

    let src_c = CString::new(src_dir.path().to_str().unwrap()).unwrap();
    let dst_c = CString::new(dst_dir.path().to_str().unwrap()).unwrap();

    let config = volterra_config_t {
        source_path: src_c.as_ptr(),
        dest_path: dst_c.as_ptr(),
        mode: volterra_mode::SmartHybrid as u32,
        chunk_size: 1024 * 1024,
        direct_io: false,
        verify_readback: false,
        memory_budget: 16 * 1024 * 1024,
        allow_overwrite: true,
        journal_path: std::ptr::null(),
    };

    unsafe {
        let mut err: *mut volterra_error_t = std::ptr::null_mut();
        let job = volterra_job_create(&config, &mut err);
        assert!(!job.is_null());

        // JSON Report (format 0)
        let json_ptr = volterra_export_report(job, volterra_report_format::Json as libc::c_int);
        assert!(!json_ptr.is_null());
        let json_str = CStr::from_ptr(json_ptr).to_str().unwrap();
        assert!(json_str.contains("volterra_version"));
        assert!(json_str.contains("job_id"));
        assert!(json_str.contains("summary"));
        assert!(json_str.contains("telemetry"));
        volterra_string_free(json_ptr);

        // CSV Report (format 1)
        let csv_ptr = volterra_export_report(job, volterra_report_format::Csv as libc::c_int);
        assert!(!csv_ptr.is_null());
        let csv_str = CStr::from_ptr(csv_ptr).to_str().unwrap();
        assert!(csv_str.starts_with("record_type,file_id,path,size_bytes,status"));
        assert!(csv_str.contains("SUMMARY"));
        volterra_string_free(csv_ptr);

        volterra_job_free(job);
    }
}

#[test]
fn test_disk_discovery_ffi() {
    unsafe {
        let mut err: *mut volterra_error_t = std::ptr::null_mut();
        let disk_list = volterra_discover_disks(&mut err);

        assert!(!disk_list.is_null());
        assert!(err.is_null());

        let count = (*disk_list).count;
        assert!(count > 0, "Host system must have at least 1 mounted volume");

        let disks = std::slice::from_raw_parts((*disk_list).disks, count);
        for d in disks {
            assert!(!d.device_id.is_null());
            assert!(!d.bsd_name.is_null());
            assert!(!d.mount_point.is_null());
            assert!(!d.fs_type.is_null());
            assert!(d.total_bytes > 0);
        }

        volterra_disks_free(disk_list);
    }
}

#[test]
fn test_panic_shielding_protection() {
    let fallback = panic_shield(-42, || {
        panic!("Simulated critical engine panic inside worker!");
    });
    assert_eq!(
        fallback, -42,
        "panic_shield must catch panics and return default value"
    );
}
