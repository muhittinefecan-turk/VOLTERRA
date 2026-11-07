//! Empirical Challenger Stress & Invariant Test Suite for `volterra-ffi`.
//!
//! Validates:
//! 1. Complete null pointer safety across all 24 exported C ABI symbols.
//! 2. Out-of-order lifecycle transitions (pause before start, resume when not paused, cancel after completion, free on active job).
//! 3. High-concurrency multi-threaded stress testing (concurrent snapshot, telemetry, error querying, report generation, pause/resume).
//! 4. Rapid spawn and destruction loops under load.

use std::ffi::{CStr, CString};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use tempfile::tempdir;
use volterra_ffi::*;

unsafe extern "C" fn dummy_event_cb(_event: *const volterra_event_t, user_data: *mut libc::c_void) {
    if !user_data.is_null() {
        let counter = &*(user_data as *const AtomicUsize);
        counter.fetch_add(1, Ordering::SeqCst);
    }
}

/// 1. Comprehensive Null Pointer Sweep across ALL 24 exported C ABI symbols.
#[test]
fn test_empirical_null_pointer_sweep_all_24_functions() {
    unsafe {
        let mut err: *mut volterra_error_t = std::ptr::null_mut();

        // Symbol 1: volterra_job_create
        // 1a. Both NULL
        let job = volterra_job_create(std::ptr::null(), std::ptr::null_mut());
        assert!(job.is_null(), "Must return NULL when config is NULL");

        // 1b. Config NULL, out_err provided
        let job = volterra_job_create(std::ptr::null(), &mut err);
        assert!(job.is_null());
        assert!(!err.is_null());
        assert_eq!((*err).code, volterra_error_code::ErrInvalidArgument as i32);
        assert!(!(*err).message.is_null());
        volterra_error_free(err);
        err = std::ptr::null_mut();

        // 1c. Config with null source_path
        let cfg_null_src = volterra_config_t {
            source_path: std::ptr::null(),
            dest_path: c"/tmp".as_ptr(),
            mode: 1,
            chunk_size: 0,
            direct_io: false,
            verify_readback: false,
            memory_budget: 0,
            allow_overwrite: true,
            journal_path: std::ptr::null(),
        };
        let job = volterra_job_create(&cfg_null_src, &mut err);
        assert!(job.is_null());
        assert_eq!((*err).code, volterra_error_code::ErrInvalidArgument as i32);
        volterra_error_free(err);
        err = std::ptr::null_mut();

        // 1d. Config with null dest_path
        let cfg_null_dst = volterra_config_t {
            source_path: c"/tmp".as_ptr(),
            dest_path: std::ptr::null(),
            mode: 1,
            chunk_size: 0,
            direct_io: false,
            verify_readback: false,
            memory_budget: 0,
            allow_overwrite: true,
            journal_path: std::ptr::null(),
        };
        let job = volterra_job_create(&cfg_null_dst, &mut err);
        assert!(job.is_null());
        assert_eq!((*err).code, volterra_error_code::ErrInvalidArgument as i32);
        volterra_error_free(err);
        err = std::ptr::null_mut();

        // 1e. Config with nonexistent source path
        let non_exist = CString::new("/non/existent/path/for/volterra/challenge").unwrap();
        let cfg_nonexistent = volterra_config_t {
            source_path: non_exist.as_ptr(),
            dest_path: c"/tmp".as_ptr(),
            mode: 1,
            chunk_size: 0,
            direct_io: false,
            verify_readback: false,
            memory_budget: 0,
            allow_overwrite: true,
            journal_path: std::ptr::null(),
        };
        let job = volterra_job_create(&cfg_nonexistent, &mut err);
        assert!(job.is_null());
        assert_eq!((*err).code, volterra_error_code::ErrSourceNotFound as i32);
        volterra_error_free(err);
        err = std::ptr::null_mut();

        // Symbol 2: volterra_job_start
        let ret = volterra_job_start(std::ptr::null_mut(), std::ptr::null_mut());
        assert_eq!(ret, volterra_error_code::ErrInvalidArgument as i32);
        let ret = volterra_job_start(std::ptr::null_mut(), &mut err);
        assert_eq!(ret, volterra_error_code::ErrInvalidArgument as i32);
        assert!(!err.is_null());
        volterra_error_free(err);
        err = std::ptr::null_mut();

        // Symbol 3: volterra_job_pause
        let ret = volterra_job_pause(std::ptr::null_mut(), std::ptr::null_mut());
        assert_eq!(ret, volterra_error_code::ErrInvalidArgument as i32);
        let ret = volterra_job_pause(std::ptr::null_mut(), &mut err);
        assert_eq!(ret, volterra_error_code::ErrInvalidArgument as i32);
        assert!(!err.is_null());
        volterra_error_free(err);
        err = std::ptr::null_mut();

        // Symbol 4: volterra_job_resume
        let ret = volterra_job_resume(std::ptr::null_mut(), std::ptr::null_mut());
        assert_eq!(ret, volterra_error_code::ErrInvalidArgument as i32);
        let ret = volterra_job_resume(std::ptr::null_mut(), &mut err);
        assert_eq!(ret, volterra_error_code::ErrInvalidArgument as i32);
        assert!(!err.is_null());
        volterra_error_free(err);
        err = std::ptr::null_mut();

        // Symbol 5: volterra_job_cancel
        let ret = volterra_job_cancel(std::ptr::null_mut(), std::ptr::null_mut());
        assert_eq!(ret, volterra_error_code::ErrInvalidArgument as i32);
        let ret = volterra_job_cancel(std::ptr::null_mut(), &mut err);
        assert_eq!(ret, volterra_error_code::ErrInvalidArgument as i32);
        assert!(!err.is_null());
        volterra_error_free(err);

        // Symbol 6: volterra_job_free
        volterra_job_free(std::ptr::null_mut()); // must be safe no-op

        // Symbol 7: volterra_get_snapshot
        let snap = volterra_get_snapshot(std::ptr::null_mut());
        assert!(snap.is_null());

        // Symbol 8: volterra_snapshot_free
        volterra_snapshot_free(std::ptr::null_mut()); // must be safe no-op
                                                      // also test snapshot struct with null strings
        let snap_nulls = Box::into_raw(Box::new(volterra_snapshot_t {
            job_id: std::ptr::null_mut(),
            state: 0,
            total_files: 0,
            processed_files: 0,
            failed_files: 0,
            skipped_files: 0,
            total_bytes: 0,
            processed_bytes: 0,
            speed_bps: 0.0,
            rolling_speed_bps: 0.0,
            eta_seconds: 0,
            error_count: 0,
            percent_complete: 0.0,
            read_latency_p50_ms: 0.0,
            read_latency_p95_ms: 0.0,
            write_latency_p50_ms: 0.0,
            write_latency_p95_ms: 0.0,
            active_chunk_size_bytes: 0,
            active_concurrency: 0,
            active_bottleneck: std::ptr::null_mut(),
        }));
        volterra_snapshot_free(snap_nulls); // must safely deallocate without segfault

        // Symbol 9: volterra_get_telemetry
        let tel = volterra_get_telemetry(std::ptr::null_mut());
        assert!(tel.is_null());

        // Symbol 10: volterra_telemetry_free
        volterra_telemetry_free(std::ptr::null_mut()); // must be safe no-op
        let tel_nulls = Box::into_raw(Box::new(volterra_telemetry_t {
            ewma_1s_mbps: 0.0,
            ewma_5s_mbps: 0.0,
            ewma_30s_mbps: 0.0,
            instantaneous_mbps: 0.0,
            source_read_mbps: 0.0,
            dest_write_mbps: 0.0,
            physical_throughput_mbps: 0.0,
            p50_read_latency_ms: 0.0,
            p95_read_latency_ms: 0.0,
            p50_write_latency_ms: 0.0,
            p95_write_latency_ms: 0.0,
            buffer_utilization_pct: 0.0,
            active_chunk_size_bytes: 0,
            active_concurrency: 0,
            bottleneck_code: 0,
            bottleneck_message: std::ptr::null_mut(),
            is_cache_burst: false,
            is_stalled: false,
            total_bytes_read: 0,
            total_bytes_written: 0,
            read_errors: 0,
            write_errors: 0,
        }));
        volterra_telemetry_free(tel_nulls); // must safely deallocate without segfault

        // Symbol 11: volterra_subscribe_events
        let ret = volterra_subscribe_events(std::ptr::null_mut(), None, std::ptr::null_mut());
        assert_eq!(ret, volterra_error_code::ErrInvalidArgument as i32);
        let ret = volterra_subscribe_events(
            std::ptr::null_mut(),
            Some(dummy_event_cb),
            std::ptr::null_mut(),
        );
        assert_eq!(ret, volterra_error_code::ErrInvalidArgument as i32);

        // Symbol 12: volterra_get_errors
        let errs = volterra_get_errors(std::ptr::null_mut());
        assert!(errs.is_null());

        // Symbol 13: volterra_error_list_free
        volterra_error_list_free(std::ptr::null_mut()); // safe no-op
        let empty_list = Box::into_raw(Box::new(volterra_error_list_t {
            count: 0,
            items: std::ptr::null_mut(),
        }));
        volterra_error_list_free(empty_list);

        // Symbol 14: volterra_skip_file
        assert_eq!(
            volterra_skip_file(std::ptr::null_mut(), 0),
            volterra_error_code::ErrInvalidArgument as i32
        );
        assert_eq!(
            volterra_skip_file(std::ptr::null_mut(), -1),
            volterra_error_code::ErrInvalidArgument as i32
        );

        // Symbol 15: volterra_retry_file
        assert_eq!(
            volterra_retry_file(std::ptr::null_mut(), 0),
            volterra_error_code::ErrInvalidArgument as i32
        );
        assert_eq!(
            volterra_retry_file(std::ptr::null_mut(), -1),
            volterra_error_code::ErrInvalidArgument as i32
        );

        // Symbol 16: volterra_defer_file
        assert_eq!(
            volterra_defer_file(std::ptr::null_mut(), 0),
            volterra_error_code::ErrInvalidArgument as i32
        );
        assert_eq!(
            volterra_defer_file(std::ptr::null_mut(), -1),
            volterra_error_code::ErrInvalidArgument as i32
        );

        // Symbol 17: volterra_skip_all_errors
        assert_eq!(
            volterra_skip_all_errors(std::ptr::null_mut()),
            volterra_error_code::ErrInvalidArgument as i32
        );

        // Symbol 18: volterra_retry_all_failed
        assert_eq!(
            volterra_retry_all_failed(std::ptr::null_mut()),
            volterra_error_code::ErrInvalidArgument as i32
        );

        // Symbol 19: volterra_discover_disks
        let disks = volterra_discover_disks(std::ptr::null_mut());
        assert!(
            !disks.is_null(),
            "discover_disks must work with out_err NULL"
        );

        // Symbol 20: volterra_disks_free
        volterra_disks_free(disks);
        volterra_disks_free(std::ptr::null_mut()); // safe no-op
        let empty_disks = Box::into_raw(Box::new(volterra_disk_list_t {
            count: 0,
            disks: std::ptr::null_mut(),
        }));
        volterra_disks_free(empty_disks);

        // Symbol 21: volterra_export_report
        assert!(volterra_export_report(std::ptr::null_mut(), 0).is_null());
        assert!(volterra_export_report(std::ptr::null_mut(), 1).is_null());
        assert!(volterra_export_report(std::ptr::null_mut(), -42).is_null());

        // Symbol 22: volterra_string_free
        volterra_string_free(std::ptr::null_mut()); // safe no-op

        // Symbol 23: volterra_error_free
        volterra_error_free(std::ptr::null_mut()); // safe no-op
        let err_null_msg = Box::into_raw(Box::new(volterra_error_t {
            code: 10,
            message: std::ptr::null_mut(),
        }));
        volterra_error_free(err_null_msg);

        // Symbol 24: volterra_version
        let ver_ptr = volterra_version();
        assert!(!ver_ptr.is_null());
        let ver_str = CStr::from_ptr(ver_ptr).to_str().unwrap();
        assert_eq!(ver_str, "1.0.0");
    }
}

/// 2. Out-of-Order Lifecycle Transitions, Flaw Detection, and Immediate Free on Active Jobs
#[test]
fn test_empirical_out_of_order_lifecycle() {
    let src_dir = tempdir().unwrap();
    let dst_dir = tempdir().unwrap();

    let f = src_dir.path().join("payload.dat");
    std::fs::write(&f, vec![0x55; 128 * 1024]).unwrap();

    let src_c = CString::new(src_dir.path().to_str().unwrap()).unwrap();
    let dst_c = CString::new(dst_dir.path().to_str().unwrap()).unwrap();

    let config = volterra_config_t {
        source_path: src_c.as_ptr(),
        dest_path: dst_c.as_ptr(),
        mode: volterra_mode::SmartHybrid as u32,
        chunk_size: 64 * 1024,
        direct_io: false,
        verify_readback: false,
        memory_budget: 16 * 1024 * 1024,
        allow_overwrite: true,
        journal_path: std::ptr::null(),
    };

    unsafe {
        let mut err: *mut volterra_error_t = std::ptr::null_mut();

        // 2a. Start on NULL handle -> returns ErrInvalidArgument (1)
        let res_null = volterra_job_start(std::ptr::null_mut(), &mut err);
        assert_eq!(res_null, volterra_error_code::ErrInvalidArgument as i32);
        assert!(!err.is_null());
        volterra_error_free(err);
        err = std::ptr::null_mut();

        // Create a real job
        let job = volterra_job_create(&config, &mut err);
        assert!(!job.is_null());

        // 2b. Pause before start (in Pending state) -> must reject with ErrInvalidState (13)
        let res = volterra_job_pause(job, &mut err);
        assert_eq!(
            res,
            volterra_error_code::ErrInvalidState as i32,
            "Pause before start must return ErrInvalidState"
        );
        assert!(!err.is_null());
        volterra_error_free(err);
        err = std::ptr::null_mut();

        // 2c. Resume when not paused (in Pending state) -> must reject with ErrInvalidState (13)
        let resume_ret = volterra_job_resume(job, &mut err);
        assert_eq!(
            resume_ret,
            volterra_error_code::ErrInvalidState as i32,
            "volterra_job_resume on Pending must return ErrInvalidState (13)"
        );
        assert!(!err.is_null());
        assert_eq!((*err).code, volterra_error_code::ErrInvalidState as i32);
        let err_msg = CStr::from_ptr((*err).message).to_str().unwrap();
        assert_eq!(
            err_msg,
            "Cannot resume job: job is not in Paused or WaitingForDisk state"
        );
        volterra_error_free(err);
        err = std::ptr::null_mut();

        let snap = volterra_get_snapshot(job);
        assert!(!snap.is_null());
        let current_state = (*snap).state;
        volterra_snapshot_free(snap);
        assert_eq!(
            current_state,
            volterra_state::Pending as u32,
            "State must remain Pending after rejected resume"
        );

        // Subsequent start succeeds cleanly and completes the copy
        let start_after_resume = volterra_job_start(job, &mut err);
        assert_eq!(
            start_after_resume, 0,
            "Subsequent start must succeed cleanly after rejected resume"
        );
        assert!(err.is_null());

        // Wait for copy completion
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut completed = false;
        while std::time::Instant::now() < deadline {
            let snap = volterra_get_snapshot(job);
            if !snap.is_null() {
                if (*snap).state == volterra_state::Completed as u32 {
                    completed = true;
                    volterra_snapshot_free(snap);
                    break;
                }
                volterra_snapshot_free(snap);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(completed, "Job should complete copying payload.dat");
        let dest_file = dst_dir.path().join("payload.dat");
        assert!(dest_file.exists());
        assert_eq!(
            std::fs::read(&dest_file).unwrap(),
            std::fs::read(src_dir.path().join("payload.dat")).unwrap()
        );

        volterra_job_free(job);

        // 2d. Cancel after completion verification
        let src_complete = tempdir().unwrap();
        let dst_complete = tempdir().unwrap();
        std::fs::write(src_complete.path().join("small.bin"), b"completion test").unwrap();
        let sc_c = CString::new(src_complete.path().to_str().unwrap()).unwrap();
        let dc_c = CString::new(dst_complete.path().to_str().unwrap()).unwrap();
        let cfg_comp = volterra_config_t {
            source_path: sc_c.as_ptr(),
            dest_path: dc_c.as_ptr(),
            mode: volterra_mode::SmartHybrid as u32,
            chunk_size: 64 * 1024,
            direct_io: false,
            verify_readback: false,
            memory_budget: 16 * 1024 * 1024,
            allow_overwrite: true,
            journal_path: std::ptr::null(),
        };

        let job_comp = volterra_job_create(&cfg_comp, &mut err);
        assert!(!job_comp.is_null());
        let st_ret = volterra_job_start(job_comp, &mut err);
        assert_eq!(st_ret, 0);

        // Wait for completion
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            let snap = volterra_get_snapshot(job_comp);
            if !snap.is_null() && (*snap).state == volterra_state::Completed as u32 {
                volterra_snapshot_free(snap);
                break;
            }
            if !snap.is_null() {
                volterra_snapshot_free(snap);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        // Cancel after completion -> must reject with ErrInvalidState
        let cancel_ret = volterra_job_cancel(job_comp, &mut err);
        assert_eq!(
            cancel_ret,
            volterra_error_code::ErrInvalidState as i32,
            "Cancel after completion must return ErrInvalidState"
        );
        if !err.is_null() {
            volterra_error_free(err);
            err = std::ptr::null_mut();
        }
        volterra_job_free(job_comp);

        // 2e. Cancelled job lifecycle controls
        let job_canc = volterra_job_create(&cfg_comp, &mut err);
        let canc_res = volterra_job_cancel(job_canc, &mut err);
        assert_eq!(canc_res, 0);

        // Cancel again after cancel:
        // In state.rs, self-transition (Cancelled, Cancelled) evaluates to true before the terminal check,
        // so calling cancel on an already cancelled job succeeds idempotently returning 0.
        let canc_again_res = volterra_job_cancel(job_canc, &mut err);
        assert_eq!(canc_again_res, 0, "Self transition on cancel is idempotent");

        // Start/Pause/Resume after cancel
        assert_eq!(
            volterra_job_start(job_canc, &mut err),
            volterra_error_code::ErrInvalidState as i32
        );
        if !err.is_null() {
            volterra_error_free(err);
            err = std::ptr::null_mut();
        }
        assert_eq!(
            volterra_job_pause(job_canc, &mut err),
            volterra_error_code::ErrInvalidState as i32
        );
        if !err.is_null() {
            volterra_error_free(err);
            err = std::ptr::null_mut();
        }
        assert_eq!(
            volterra_job_resume(job_canc, &mut err),
            volterra_error_code::ErrInvalidState as i32
        );
        if !err.is_null() {
            volterra_error_free(err);
        }
        volterra_job_free(job_canc);
    }

    // 2f. Stress test: Rapidly create, start, and free ACTIVE jobs (free during transfer)
    for iter in 0..15 {
        let src_iter = tempdir().unwrap();
        let dst_iter = tempdir().unwrap();
        for i in 0..5 {
            std::fs::write(
                src_iter.path().join(format!("active_{}_{}.bin", iter, i)),
                vec![0xAA; 1024 * 1024],
            )
            .unwrap();
        }

        let s_c = CString::new(src_iter.path().to_str().unwrap()).unwrap();
        let d_c = CString::new(dst_iter.path().to_str().unwrap()).unwrap();

        let cfg = volterra_config_t {
            source_path: s_c.as_ptr(),
            dest_path: d_c.as_ptr(),
            mode: volterra_mode::Turbo as u32,
            chunk_size: 1024 * 1024,
            direct_io: false,
            verify_readback: false,
            memory_budget: 32 * 1024 * 1024,
            allow_overwrite: true,
            journal_path: std::ptr::null(),
        };

        unsafe {
            let mut err: *mut volterra_error_t = std::ptr::null_mut();
            let job = volterra_job_create(&cfg, &mut err);
            assert!(!job.is_null());

            let event_counter = Arc::new(AtomicUsize::new(0));
            let ud = Arc::as_ptr(&event_counter) as *mut libc::c_void;
            let sub = volterra_subscribe_events(job, Some(dummy_event_cb), ud);
            assert_eq!(sub, 0);

            // Start copying
            let st = volterra_job_start(job, &mut err);
            assert_eq!(st, 0);

            // Sleep a small pseudo-random time while job is actively running
            std::thread::sleep(std::time::Duration::from_millis(5 + (iter % 10) as u64));

            // Immediately free the job while threads and I/O are in flight!
            volterra_job_free(job);
        }
    }
}

/// 3. Multi-Threaded Concurrent Access Stress (Snapshot, Telemetry, Reports, Errors, Pause/Resume)
#[test]
fn test_empirical_concurrent_stress() {
    let src_dir = tempdir().unwrap();
    let dst_dir = tempdir().unwrap();

    // Create 15 files of 512KB each
    for i in 0..15 {
        let file = src_dir.path().join(format!("stress_{}.bin", i));
        std::fs::write(&file, vec![0xCC; 512 * 1024]).unwrap();
    }

    let src_c = CString::new(src_dir.path().to_str().unwrap()).unwrap();
    let dst_c = CString::new(dst_dir.path().to_str().unwrap()).unwrap();

    let config = volterra_config_t {
        source_path: src_c.as_ptr(),
        dest_path: dst_c.as_ptr(),
        mode: volterra_mode::SmartHybrid as u32,
        chunk_size: 256 * 1024,
        direct_io: false,
        verify_readback: true,
        memory_budget: 32 * 1024 * 1024,
        allow_overwrite: true,
        journal_path: std::ptr::null(),
    };

    let mut err: *mut volterra_error_t = std::ptr::null_mut();
    let job = unsafe { volterra_job_create(&config, &mut err) };
    assert!(!job.is_null());

    let event_counter = Arc::new(AtomicUsize::new(0));
    let ud = Arc::as_ptr(&event_counter) as *mut libc::c_void;
    let sub = unsafe { volterra_subscribe_events(job, Some(dummy_event_cb), ud) };
    assert_eq!(sub, 0);

    let st = unsafe { volterra_job_start(job, &mut err) };
    assert_eq!(st, 0);

    let job_addr = job as usize;
    let running_flag = Arc::new(AtomicBool::new(true));

    let mut handles = Vec::new();

    // Worker 1 & 2: Rapid Snapshot queries
    for _ in 0..2 {
        let r_flag = running_flag.clone();
        handles.push(std::thread::spawn(move || {
            let j = job_addr as *mut volterra_job_t;
            let mut count = 0;
            while r_flag.load(Ordering::Relaxed) && count < 600 {
                unsafe {
                    let snap = volterra_get_snapshot(j);
                    if !snap.is_null() {
                        assert!(!(*snap).job_id.is_null());
                        volterra_snapshot_free(snap);
                    }
                }
                count += 1;
                std::thread::sleep(std::time::Duration::from_micros(200));
            }
        }));
    }

    // Worker 3 & 4: Rapid Telemetry queries
    for _ in 0..2 {
        let r_flag = running_flag.clone();
        handles.push(std::thread::spawn(move || {
            let j = job_addr as *mut volterra_job_t;
            let mut count = 0;
            while r_flag.load(Ordering::Relaxed) && count < 600 {
                unsafe {
                    let tel = volterra_get_telemetry(j);
                    if !tel.is_null() {
                        assert!(!(*tel).bottleneck_message.is_null());
                        volterra_telemetry_free(tel);
                    }
                }
                count += 1;
                std::thread::sleep(std::time::Duration::from_micros(200));
            }
        }));
    }

    // Worker 5: Rapid Report exports (JSON + CSV)
    {
        let r_flag = running_flag.clone();
        handles.push(std::thread::spawn(move || {
            let j = job_addr as *mut volterra_job_t;
            let mut count = 0;
            while r_flag.load(Ordering::Relaxed) && count < 200 {
                unsafe {
                    let json = volterra_export_report(j, 0);
                    if !json.is_null() {
                        let s = CStr::from_ptr(json).to_str().unwrap();
                        assert!(s.contains("job_id"));
                        volterra_string_free(json);
                    }
                    let csv = volterra_export_report(j, 1);
                    if !csv.is_null() {
                        let s = CStr::from_ptr(csv).to_str().unwrap();
                        assert!(s.starts_with("record_type"));
                        volterra_string_free(csv);
                    }
                }
                count += 1;
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }));
    }

    // Worker 6: Rapid Error inspection
    {
        let r_flag = running_flag.clone();
        handles.push(std::thread::spawn(move || {
            let j = job_addr as *mut volterra_job_t;
            let mut count = 0;
            while r_flag.load(Ordering::Relaxed) && count < 400 {
                unsafe {
                    let errs = volterra_get_errors(j);
                    if !errs.is_null() {
                        volterra_error_list_free(errs);
                    }
                }
                count += 1;
                std::thread::sleep(std::time::Duration::from_micros(300));
            }
        }));
    }

    // Worker 7: Interactive error actions with simulated file IDs
    {
        let r_flag = running_flag.clone();
        handles.push(std::thread::spawn(move || {
            let j = job_addr as *mut volterra_job_t;
            let mut count = 0;
            while r_flag.load(Ordering::Relaxed) && count < 200 {
                unsafe {
                    let _ = volterra_skip_file(j, 42);
                    let _ = volterra_retry_file(j, 42);
                    let _ = volterra_defer_file(j, 42);
                    let _ = volterra_skip_all_errors(j);
                    let _ = volterra_retry_all_failed(j);
                }
                count += 1;
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }));
    }

    // Worker 8: Intermittent Pause and Resume toggles
    {
        let r_flag = running_flag.clone();
        handles.push(std::thread::spawn(move || {
            let j = job_addr as *mut volterra_job_t;
            let mut count = 0;
            while r_flag.load(Ordering::Relaxed) && count < 20 {
                unsafe {
                    let _ = volterra_job_pause(j, std::ptr::null_mut());
                    std::thread::sleep(std::time::Duration::from_millis(2));
                    let _ = volterra_job_resume(j, std::ptr::null_mut());
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                count += 1;
            }
        }));
    }

    // Let stress run for 400ms
    std::thread::sleep(std::time::Duration::from_millis(400));
    running_flag.store(false, Ordering::Relaxed);

    for h in handles {
        h.join().unwrap();
    }

    // Clean up
    unsafe {
        volterra_job_free(job);
    }
}

/// 4. Rapid Multi-Threaded Spawning and Destruction Loops
#[test]
fn test_empirical_rapid_spawn_destroy_concurrency() {
    let mut thread_handles = Vec::new();

    // 4 concurrent spawning threads
    for _t_idx in 0..4 {
        thread_handles.push(std::thread::spawn(move || {
            for iter in 0..8 {
                let src_dir = tempdir().unwrap();
                let dst_dir = tempdir().unwrap();

                let f = src_dir.path().join("file.bin");
                std::fs::write(&f, vec![0x77; 128 * 1024]).unwrap();

                let src_c = CString::new(src_dir.path().to_str().unwrap()).unwrap();
                let dst_c = CString::new(dst_dir.path().to_str().unwrap()).unwrap();

                let config = volterra_config_t {
                    source_path: src_c.as_ptr(),
                    dest_path: dst_c.as_ptr(),
                    mode: (iter % 3) as u32,
                    chunk_size: 128 * 1024,
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

                    // Read snapshot & telemetry before start
                    let snap = volterra_get_snapshot(job);
                    assert!(!snap.is_null());
                    volterra_snapshot_free(snap);

                    let tel = volterra_get_telemetry(job);
                    assert!(!tel.is_null());
                    volterra_telemetry_free(tel);

                    // Subscribe events
                    let sub =
                        volterra_subscribe_events(job, Some(dummy_event_cb), std::ptr::null_mut());
                    assert_eq!(sub, 0);

                    // Start job
                    let st = volterra_job_start(job, &mut err);
                    assert_eq!(st, 0);

                    // Query snapshot while running
                    let snap2 = volterra_get_snapshot(job);
                    if !snap2.is_null() {
                        volterra_snapshot_free(snap2);
                    }

                    // Free job immediately (cancelling active transfer)
                    volterra_job_free(job);
                }
            }
        }));
    }

    // 5th thread continuously discovering disks in parallel
    let disk_thread = std::thread::spawn(|| {
        for _ in 0..10 {
            unsafe {
                let mut err: *mut volterra_error_t = std::ptr::null_mut();
                let disks = volterra_discover_disks(&mut err);
                if !disks.is_null() {
                    assert!((*disks).count > 0);
                    volterra_disks_free(disks);
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
    });

    for h in thread_handles {
        h.join().unwrap();
    }
    disk_thread.join().unwrap();
}
