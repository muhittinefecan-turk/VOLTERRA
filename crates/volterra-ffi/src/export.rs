//! Exported C ABI symbols for the VOLTERRA framework.

use crate::bridge::*;
use crate::memory::*;
use crate::types::*;

/// Creates a new transfer job instance from configuration.
/// Returns an opaque `volterra_job_t` pointer on success, or NULL on error.
#[no_mangle]
pub unsafe extern "C" fn volterra_job_create(
    config: *const volterra_config_t,
    out_err: *mut *mut volterra_error_t,
) -> *mut volterra_job_t {
    clear_out_error(out_err);

    panic_shield(std::ptr::null_mut(), || {
        if config.is_null() {
            set_out_error(
                out_err,
                volterra_error_code::ErrInvalidArgument as i32,
                "Config pointer is null",
            );
            return std::ptr::null_mut();
        }

        let cfg = &*config;
        if cfg.source_path.is_null() {
            set_out_error(
                out_err,
                volterra_error_code::ErrInvalidArgument as i32,
                "source_path is null",
            );
            return std::ptr::null_mut();
        }
        if cfg.dest_path.is_null() {
            set_out_error(
                out_err,
                volterra_error_code::ErrInvalidArgument as i32,
                "dest_path is null",
            );
            return std::ptr::null_mut();
        }

        let src_str = match std::ffi::CStr::from_ptr(cfg.source_path).to_str() {
            Ok(s) => s,
            Err(_) => {
                set_out_error(
                    out_err,
                    volterra_error_code::ErrInvalidArgument as i32,
                    "source_path contains invalid UTF-8",
                );
                return std::ptr::null_mut();
            }
        };

        let dst_str = match std::ffi::CStr::from_ptr(cfg.dest_path).to_str() {
            Ok(s) => s,
            Err(_) => {
                set_out_error(
                    out_err,
                    volterra_error_code::ErrInvalidArgument as i32,
                    "dest_path contains invalid UTF-8",
                );
                return std::ptr::null_mut();
            }
        };

        let src_path = std::path::Path::new(src_str);
        if !src_path.exists() {
            set_out_error(
                out_err,
                volterra_error_code::ErrSourceNotFound as i32,
                &format!("Source path does not exist: {}", src_str),
            );
            return std::ptr::null_mut();
        }

        let mode = match cfg.mode {
            0 => volterra_journal::JobMode::Turbo,
            1 => volterra_journal::JobMode::SmartHybrid,
            2 => volterra_journal::JobMode::SafeRecovery,
            _ => volterra_journal::JobMode::SmartHybrid,
        };

        let chunk_size = if cfg.chunk_size == 0 {
            8 * 1024 * 1024
        } else {
            cfg.chunk_size
        };

        let memory_budget = if cfg.memory_budget == 0 {
            128 * 1024 * 1024
        } else {
            cfg.memory_budget
        };

        let journal_path = if !cfg.journal_path.is_null() {
            std::ffi::CStr::from_ptr(cfg.journal_path)
                .to_str()
                .ok()
                .map(std::path::PathBuf::from)
        } else {
            None
        };

        let mut builder = volterra_core::JobBuilder::new()
            .source(src_path)
            .destination(dst_str)
            .mode(mode)
            .chunk_size(chunk_size)
            .direct_io(cfg.direct_io)
            .verify_readback(cfg.verify_readback)
            .memory_budget(memory_budget)
            .allow_overwrite(cfg.allow_overwrite);

        if let Some(jnl) = journal_path {
            builder = builder.journal_path(jnl);
        }

        match builder.build() {
            Ok(handle) => Box::into_raw(Box::new(volterra_job_t {
                handle,
                event_thread: parking_lot::Mutex::new(None),
                stop_signal: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            })),
            Err(e) => {
                set_out_error(
                    out_err,
                    volterra_error_code::ErrInvalidState as i32,
                    &e.to_string(),
                );
                std::ptr::null_mut()
            }
        }
    })
}

/// Starts execution of a transfer job.
/// Returns 0 on success, or non-zero error code on failure.
#[no_mangle]
pub unsafe extern "C" fn volterra_job_start(
    job: *mut volterra_job_t,
    out_err: *mut *mut volterra_error_t,
) -> libc::c_int {
    clear_out_error(out_err);

    panic_shield(volterra_error_code::ErrInternalPanic as i32, || {
        if job.is_null() {
            set_out_error(
                out_err,
                volterra_error_code::ErrInvalidArgument as i32,
                "Job handle is null",
            );
            return volterra_error_code::ErrInvalidArgument as i32;
        }

        match (*job).handle.start() {
            Ok(()) => volterra_error_code::Ok as i32,
            Err(e) => {
                set_out_error(
                    out_err,
                    volterra_error_code::ErrInvalidState as i32,
                    &e.to_string(),
                );
                volterra_error_code::ErrInvalidState as i32
            }
        }
    })
}

/// Pauses an active transfer job.
/// Returns 0 on success, or non-zero error code on failure.
#[no_mangle]
pub unsafe extern "C" fn volterra_job_pause(
    job: *mut volterra_job_t,
    out_err: *mut *mut volterra_error_t,
) -> libc::c_int {
    clear_out_error(out_err);

    panic_shield(volterra_error_code::ErrInternalPanic as i32, || {
        if job.is_null() {
            set_out_error(
                out_err,
                volterra_error_code::ErrInvalidArgument as i32,
                "Job handle is null",
            );
            return volterra_error_code::ErrInvalidArgument as i32;
        }

        match (*job).handle.pause() {
            Ok(()) => volterra_error_code::Ok as i32,
            Err(e) => {
                set_out_error(
                    out_err,
                    volterra_error_code::ErrInvalidState as i32,
                    &e.to_string(),
                );
                volterra_error_code::ErrInvalidState as i32
            }
        }
    })
}

/// Resumes a paused transfer job.
/// Returns 0 on success, or non-zero error code on failure.
#[no_mangle]
pub unsafe extern "C" fn volterra_job_resume(
    job: *mut volterra_job_t,
    out_err: *mut *mut volterra_error_t,
) -> libc::c_int {
    clear_out_error(out_err);

    panic_shield(volterra_error_code::ErrInternalPanic as i32, || {
        if job.is_null() {
            set_out_error(
                out_err,
                volterra_error_code::ErrInvalidArgument as i32,
                "Job handle is null",
            );
            return volterra_error_code::ErrInvalidArgument as i32;
        }

        let state = (*job).handle.state();
        if state != volterra_core::JobLifecycleState::Paused
            && state != volterra_core::JobLifecycleState::WaitingForDisk
        {
            set_out_error(
                out_err,
                volterra_error_code::ErrInvalidState as i32,
                "Cannot resume job: job is not in Paused or WaitingForDisk state",
            );
            return volterra_error_code::ErrInvalidState as i32;
        }

        match (*job).handle.resume() {
            Ok(()) => volterra_error_code::Ok as i32,
            Err(e) => {
                set_out_error(
                    out_err,
                    volterra_error_code::ErrInvalidState as i32,
                    &e.to_string(),
                );
                volterra_error_code::ErrInvalidState as i32
            }
        }
    })
}

/// Cancels a running or paused transfer job.
/// Returns 0 on success, or non-zero error code on failure.
#[no_mangle]
pub unsafe extern "C" fn volterra_job_cancel(
    job: *mut volterra_job_t,
    out_err: *mut *mut volterra_error_t,
) -> libc::c_int {
    clear_out_error(out_err);

    panic_shield(volterra_error_code::ErrInternalPanic as i32, || {
        if job.is_null() {
            set_out_error(
                out_err,
                volterra_error_code::ErrInvalidArgument as i32,
                "Job handle is null",
            );
            return volterra_error_code::ErrInvalidArgument as i32;
        }

        match (*job).handle.cancel() {
            Ok(()) => volterra_error_code::Ok as i32,
            Err(e) => {
                set_out_error(
                    out_err,
                    volterra_error_code::ErrInvalidState as i32,
                    &e.to_string(),
                );
                volterra_error_code::ErrInvalidState as i32
            }
        }
    })
}

/// Captures a point-in-time snapshot of the transfer job.
/// Caller must free the returned pointer with `volterra_snapshot_free`.
#[no_mangle]
pub unsafe extern "C" fn volterra_get_snapshot(
    job: *mut volterra_job_t,
) -> *mut volterra_snapshot_t {
    panic_shield(std::ptr::null_mut(), || {
        if job.is_null() {
            return std::ptr::null_mut();
        }

        let snap = (*job).handle.snapshot();
        let state_val = match snap.state {
            volterra_core::JobLifecycleState::Pending => volterra_state::Pending as u32,
            volterra_core::JobLifecycleState::Scanning => volterra_state::Scanning as u32,
            volterra_core::JobLifecycleState::Running => volterra_state::Running as u32,
            volterra_core::JobLifecycleState::Paused => volterra_state::Paused as u32,
            volterra_core::JobLifecycleState::Completed => volterra_state::Completed as u32,
            volterra_core::JobLifecycleState::Failed => volterra_state::Failed as u32,
            volterra_core::JobLifecycleState::WaitingForDisk => {
                volterra_state::WaitingForDisk as u32
            }
            volterra_core::JobLifecycleState::Cancelled => volterra_state::Cancelled as u32,
        };

        Box::into_raw(Box::new(volterra_snapshot_t {
            job_id: alloc_c_string(&snap.job_id),
            state: state_val,
            total_files: snap.total_files,
            processed_files: snap.completed_files,
            failed_files: snap.failed_files,
            skipped_files: snap.skipped_files,
            total_bytes: snap.total_bytes,
            processed_bytes: snap.transferred_bytes,
            speed_bps: snap.current_throughput_mbps * 1_048_576.0,
            rolling_speed_bps: snap.ewma_5s_mbps * 1_048_576.0,
            eta_seconds: snap.estimated_remaining_seconds.unwrap_or(0),
            error_count: (snap.quarantined_errors_count as u64) + snap.failed_files,
            percent_complete: snap.percent_complete,
            read_latency_p50_ms: snap.p50_read_latency_ms,
            read_latency_p95_ms: snap.p95_read_latency_ms,
            write_latency_p50_ms: snap.p50_write_latency_ms,
            write_latency_p95_ms: snap.p95_write_latency_ms,
            active_chunk_size_bytes: snap.active_chunk_size_bytes,
            active_concurrency: snap.active_concurrency,
            active_bottleneck: alloc_opt_c_string(snap.active_bottleneck.as_deref()),
        }))
    })
}

/// Retrieves real-time performance telemetry.
/// Caller must free the returned pointer with `volterra_telemetry_free`.
#[no_mangle]
pub unsafe extern "C" fn volterra_get_telemetry(
    job: *mut volterra_job_t,
) -> *mut volterra_telemetry_t {
    panic_shield(std::ptr::null_mut(), || {
        if job.is_null() {
            return std::ptr::null_mut();
        }

        let tel = (*job).handle.telemetry();
        let bottleneck = volterra_optimizer::BottleneckDetector::detect_bottleneck(&tel);
        let bottleneck_code = match bottleneck.bottleneck_type {
            volterra_optimizer::BottleneckType::Nominal => volterra_bottleneck::Nominal as u32,
            volterra_optimizer::BottleneckType::SourceReadLimited => {
                volterra_bottleneck::SourceRead as u32
            }
            volterra_optimizer::BottleneckType::DestinationWriteLimited => {
                volterra_bottleneck::DestWrite as u32
            }
            volterra_optimizer::BottleneckType::InterfaceConstrained => {
                volterra_bottleneck::Interface as u32
            }
            volterra_optimizer::BottleneckType::MetadataConstrained => {
                volterra_bottleneck::Metadata as u32
            }
            volterra_optimizer::BottleneckType::SmallFileOverhead => {
                volterra_bottleneck::SmallFile as u32
            }
            volterra_optimizer::BottleneckType::CpuIntegrityConstrained => {
                volterra_bottleneck::CpuIntegrity as u32
            }
            volterra_optimizer::BottleneckType::Contention => {
                volterra_bottleneck::Contention as u32
            }
            volterra_optimizer::BottleneckType::ErrorDegradation => {
                volterra_bottleneck::ErrorDegradation as u32
            }
            volterra_optimizer::BottleneckType::Unknown => volterra_bottleneck::Unknown as u32,
        };

        Box::into_raw(Box::new(volterra_telemetry_t {
            ewma_1s_mbps: tel.ewma_1s_mbps,
            ewma_5s_mbps: tel.ewma_5s_mbps,
            ewma_30s_mbps: tel.ewma_30s_mbps,
            instantaneous_mbps: tel.instantaneous_mbps,
            source_read_mbps: tel.source_read_mbps,
            dest_write_mbps: tel.dest_write_mbps,
            physical_throughput_mbps: tel.physical_throughput_mbps,
            p50_read_latency_ms: tel.p50_read_latency_ms,
            p95_read_latency_ms: tel.p95_read_latency_ms,
            p50_write_latency_ms: tel.p50_write_latency_ms,
            p95_write_latency_ms: tel.p95_write_latency_ms,
            buffer_utilization_pct: tel.buffer_utilization_pct,
            active_chunk_size_bytes: tel.current_chunk_size,
            active_concurrency: tel.active_concurrency,
            bottleneck_code,
            bottleneck_message: alloc_c_string(&bottleneck.user_message),
            is_cache_burst: tel.is_cache_burst,
            is_stalled: tel.is_stalled,
            total_bytes_read: tel.total_bytes_read,
            total_bytes_written: tel.total_bytes_written,
            read_errors: tel.read_errors,
            write_errors: tel.write_errors,
        }))
    })
}

/// Registers an asynchronous event callback.
/// Spawns a background bridge thread consuming events from the engine.
#[no_mangle]
pub unsafe extern "C" fn volterra_subscribe_events(
    job: *mut volterra_job_t,
    cb: volterra_event_callback_fn,
    user_data: *mut libc::c_void,
) -> libc::c_int {
    panic_shield(volterra_error_code::ErrInternalPanic as i32, || {
        if job.is_null() {
            return volterra_error_code::ErrInvalidArgument as i32;
        }

        match crate::bridge::start_event_bridge(&*job, cb, user_data) {
            Ok(()) => volterra_error_code::Ok as i32,
            Err(e) => e as i32,
        }
    })
}

/// Retrieves the current list of quarantined or failing files.
/// Caller must free the returned pointer with `volterra_error_list_free`.
#[no_mangle]
pub unsafe extern "C" fn volterra_get_errors(
    job: *mut volterra_job_t,
) -> *mut volterra_error_list_t {
    panic_shield(std::ptr::null_mut(), || {
        if job.is_null() {
            return std::ptr::null_mut();
        }

        let quarantined = (*job).handle.recovery().queue().list_all();
        let mut items = Vec::with_capacity(quarantined.len());

        for q in quarantined {
            let extents_vec: Vec<volterra_extent_t> = q
                .failed_extents
                .intervals()
                .iter()
                .map(|ext| volterra_extent_t {
                    offset: ext.offset,
                    length: ext.length,
                    error_code: q.last_error.errno_code.unwrap_or(libc::EIO),
                })
                .collect();

            let extents_count = extents_vec.len();
            let extents_ptr = if extents_count > 0 {
                Box::into_raw(extents_vec.into_boxed_slice()) as *mut volterra_extent_t
            } else {
                std::ptr::null_mut()
            };

            items.push(volterra_error_item_t {
                file_id: q.file_id,
                path: alloc_c_string(&q.relative_path.to_string_lossy()),
                error_kind: q.last_error.category as i32,
                error_message: alloc_c_string(&q.last_error.message),
                total_size_bytes: q.total_size_bytes,
                state: q.state as u32,
                attempts: q.attempts,
                failed_extents_count: extents_count,
                extents: extents_ptr,
            });
        }

        let count = items.len();
        let items_ptr = if count > 0 {
            Box::into_raw(items.into_boxed_slice()) as *mut volterra_error_item_t
        } else {
            std::ptr::null_mut()
        };

        Box::into_raw(Box::new(volterra_error_list_t {
            count,
            items: items_ptr,
        }))
    })
}

/// Skips an unreadable or failing file.
#[no_mangle]
pub unsafe extern "C" fn volterra_skip_file(job: *mut volterra_job_t, file_id: i64) -> libc::c_int {
    panic_shield(volterra_error_code::ErrInternalPanic as i32, || {
        if job.is_null() {
            return volterra_error_code::ErrInvalidArgument as i32;
        }

        match (*job).handle.skip_file(file_id) {
            Ok(()) => volterra_error_code::Ok as i32,
            Err(_) => volterra_error_code::ErrRecoveryFailed as i32,
        }
    })
}

/// Retries an unreadable or failing file.
#[no_mangle]
pub unsafe extern "C" fn volterra_retry_file(
    job: *mut volterra_job_t,
    file_id: i64,
) -> libc::c_int {
    panic_shield(volterra_error_code::ErrInternalPanic as i32, || {
        if job.is_null() {
            return volterra_error_code::ErrInvalidArgument as i32;
        }

        match (*job).handle.retry_file(file_id) {
            Ok(()) => volterra_error_code::Ok as i32,
            Err(_) => volterra_error_code::ErrRecoveryFailed as i32,
        }
    })
}

/// Defers an unreadable file to the second-pass recovery pool.
#[no_mangle]
pub unsafe extern "C" fn volterra_defer_file(
    job: *mut volterra_job_t,
    file_id: i64,
) -> libc::c_int {
    panic_shield(volterra_error_code::ErrInternalPanic as i32, || {
        if job.is_null() {
            return volterra_error_code::ErrInvalidArgument as i32;
        }

        match (*job)
            .handle
            .recovery()
            .apply_action(volterra_recovery::InteractiveAction::DeferFile { file_id })
        {
            Ok(_) => volterra_error_code::Ok as i32,
            Err(_) => volterra_error_code::ErrRecoveryFailed as i32,
        }
    })
}

/// Bulk skips all currently quarantined files.
#[no_mangle]
pub unsafe extern "C" fn volterra_skip_all_errors(job: *mut volterra_job_t) -> libc::c_int {
    panic_shield(volterra_error_code::ErrInternalPanic as i32, || {
        if job.is_null() {
            return volterra_error_code::ErrInvalidArgument as i32;
        }

        match (*job).handle.skip_all_errors() {
            Ok(()) => volterra_error_code::Ok as i32,
            Err(_) => volterra_error_code::ErrRecoveryFailed as i32,
        }
    })
}

/// Bulk retries all failed files with reset retry budgets.
#[no_mangle]
pub unsafe extern "C" fn volterra_retry_all_failed(job: *mut volterra_job_t) -> libc::c_int {
    panic_shield(volterra_error_code::ErrInternalPanic as i32, || {
        if job.is_null() {
            return volterra_error_code::ErrInvalidArgument as i32;
        }

        match (*job).handle.retry_all_failed() {
            Ok(()) => volterra_error_code::Ok as i32,
            Err(_) => volterra_error_code::ErrRecoveryFailed as i32,
        }
    })
}

/// Discovers mounted disk volumes, media types, and hardware capabilities.
/// Caller must free the returned pointer with `volterra_disks_free`.
#[no_mangle]
pub unsafe extern "C" fn volterra_discover_disks(
    out_err: *mut *mut volterra_error_t,
) -> *mut volterra_disk_list_t {
    clear_out_error(out_err);

    panic_shield(std::ptr::null_mut(), || {
        match volterra_profiler::DiskDiscoveryEngine::discover_disks() {
            Ok(profiles) => {
                let mut disks = Vec::with_capacity(profiles.len());
                for p in profiles {
                    let is_rotational =
                        p.media_type == volterra_profiler::DiskMediaType::Rotational;
                    let is_removable = !p.is_internal;
                    let is_apfs_synthesized = p.bsd_name != p.physical_parent_id
                        && p.filesystem == volterra_platform_macos::FsType::Apfs;
                    let transport_str = format!("{:?}", p.transport);
                    let fs_str = format!("{:?}", p.filesystem);

                    disks.push(volterra_disk_t {
                        device_id: alloc_c_string(&p.device_id),
                        bsd_name: alloc_c_string(&p.bsd_name),
                        mount_point: alloc_c_string(&p.mount_point.to_string_lossy()),
                        volume_name: alloc_c_string(&p.volume_name),
                        fs_type: alloc_c_string(&fs_str),
                        total_bytes: p.capacity_bytes,
                        free_bytes: p.available_bytes,
                        is_rotational,
                        is_removable,
                        is_apfs_synthesized,
                        parent_bsd_name: alloc_c_string(&p.physical_parent_id),
                        model: alloc_c_string(&p.model),
                        vendor: alloc_c_string(&p.vendor),
                        transport: alloc_c_string(&transport_str),
                    });
                }

                let count = disks.len();
                let disks_ptr = if count > 0 {
                    Box::into_raw(disks.into_boxed_slice()) as *mut volterra_disk_t
                } else {
                    std::ptr::null_mut()
                };

                Box::into_raw(Box::new(volterra_disk_list_t {
                    count,
                    disks: disks_ptr,
                }))
            }
            Err(e) => {
                set_out_error(
                    out_err,
                    volterra_error_code::ErrInvalidState as i32,
                    &e.to_string(),
                );
                std::ptr::null_mut()
            }
        }
    })
}

/// Generates an audit report in JSON or CSV format.
/// Caller must free the returned string with `volterra_string_free`.
#[no_mangle]
pub unsafe extern "C" fn volterra_export_report(
    job: *mut volterra_job_t,
    format_type: libc::c_int,
) -> *mut libc::c_char {
    panic_shield(std::ptr::null_mut(), || {
        if job.is_null() {
            return std::ptr::null_mut();
        }

        let snap = (*job).handle.snapshot();
        let tel = (*job).handle.telemetry();
        let errors = (*job).handle.recovery().queue().list_all();

        let report_str = if format_type == volterra_report_format::Json as libc::c_int {
            let error_json_items: Vec<serde_json::Value> = errors
                .iter()
                .map(|q| {
                    let extents_json: Vec<serde_json::Value> = q
                        .failed_extents
                        .intervals()
                        .iter()
                        .map(|e| {
                            serde_json::json!({
                                "offset": e.offset,
                                "length": e.length,
                            })
                        })
                        .collect();

                    serde_json::json!({
                        "file_id": q.file_id,
                        "relative_path": q.relative_path.to_string_lossy(),
                        "total_size_bytes": q.total_size_bytes,
                        "state": format!("{:?}", q.state),
                        "attempts": q.attempts,
                        "error_category": format!("{:?}", q.last_error.category),
                        "error_message": q.last_error.message,
                        "failed_extents": extents_json,
                    })
                })
                .collect();

            let doc = serde_json::json!({
                "volterra_version": "1.0.0",
                "job_id": snap.job_id,
                "state": format!("{:?}", snap.state),
                "summary": {
                    "total_files": snap.total_files,
                    "completed_files": snap.completed_files,
                    "failed_files": snap.failed_files,
                    "skipped_files": snap.skipped_files,
                    "total_bytes": snap.total_bytes,
                    "transferred_bytes": snap.transferred_bytes,
                    "percent_complete": snap.percent_complete,
                    "current_throughput_mbps": snap.current_throughput_mbps,
                    "ewma_5s_mbps": snap.ewma_5s_mbps,
                    "estimated_remaining_seconds": snap.estimated_remaining_seconds,
                    "active_bottleneck": snap.active_bottleneck,
                },
                "telemetry": {
                    "p50_read_latency_ms": tel.p50_read_latency_ms,
                    "p95_read_latency_ms": tel.p95_read_latency_ms,
                    "p50_write_latency_ms": tel.p50_write_latency_ms,
                    "p95_write_latency_ms": tel.p95_write_latency_ms,
                    "buffer_utilization_pct": tel.buffer_utilization_pct,
                    "active_chunk_size_bytes": tel.current_chunk_size,
                    "active_concurrency": tel.active_concurrency,
                },
                "quarantined_errors": error_json_items,
            });

            serde_json::to_string_pretty(&doc).unwrap_or_default()
        } else {
            // CSV format
            let mut csv = String::new();
            csv.push_str("record_type,file_id,path,size_bytes,status,error_code,error_message,failed_offset,failed_length\n");
            csv.push_str(&format!(
                "SUMMARY,-1,all,{},{:?},0,\"{}\",0,0\n",
                snap.total_bytes,
                snap.state,
                snap.active_bottleneck.unwrap_or_default()
            ));

            for q in &errors {
                let path = q.relative_path.to_string_lossy();
                let errno = q.last_error.errno_code.unwrap_or(libc::EIO);
                let msg = q.last_error.message.replace('"', "\"\"");

                if q.failed_extents.is_empty() {
                    csv.push_str(&format!(
                        "FILE_ERROR,{},\"{}\",{},{:?},{},\"{}\",0,0\n",
                        q.file_id, path, q.total_size_bytes, q.state, errno, msg
                    ));
                } else {
                    for ext in q.failed_extents.intervals() {
                        csv.push_str(&format!(
                            "FILE_ERROR,{},\"{}\",{},{:?},{},\"{}\",{},{}\n",
                            q.file_id,
                            path,
                            q.total_size_bytes,
                            q.state,
                            errno,
                            msg,
                            ext.offset,
                            ext.length
                        ));
                    }
                }
            }
            csv
        };

        alloc_c_string(&report_str)
    })
}

/// Returns the static version string of the VOLTERRA framework.
#[no_mangle]
pub unsafe extern "C" fn volterra_version() -> *const libc::c_char {
    static VERSION: &[u8] = b"1.0.0\0";
    VERSION.as_ptr() as *const libc::c_char
}
