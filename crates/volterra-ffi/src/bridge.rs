//! Bridge implementation connecting C ABI calls to the VOLTERRA core engine.

use crate::memory::alloc_c_string;
use crate::types::*;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::Ordering;

/// Shields exported C ABI functions against unwinding panics using AssertUnwindSafe.
/// Returns `default` and logs an error if a panic occurs.
pub fn panic_shield<F, R>(default: R, f: F) -> R
where
    F: FnOnce() -> R,
{
    match std::panic::catch_unwind(AssertUnwindSafe(f)) {
        Ok(res) => res,
        Err(err) => {
            let msg = if let Some(s) = err.downcast_ref::<&str>() {
                s.to_string()
            } else if let Some(s) = err.downcast_ref::<String>() {
                s.clone()
            } else {
                "Unknown internal panic in VOLTERRA core".to_string()
            };
            tracing::error!("Caught panic at C ABI boundary: {}", msg);
            default
        }
    }
}

/// Allocates a new `volterra_error_t` on the heap with the given error code and description.
pub fn make_error(code: i32, msg: &str) -> *mut volterra_error_t {
    let message = alloc_c_string(msg);
    Box::into_raw(Box::new(volterra_error_t { code, message }))
}

/// Populates `*out_err` if `out_err` is non-null.
pub unsafe fn set_out_error(out_err: *mut *mut volterra_error_t, code: i32, msg: &str) {
    if !out_err.is_null() {
        *out_err = make_error(code, msg);
    }
}

/// Sets `*out_err = NULL` if `out_err` is non-null.
pub unsafe fn clear_out_error(out_err: *mut *mut volterra_error_t) {
    if !out_err.is_null() {
        *out_err = std::ptr::null_mut();
    }
}

/// Spawns a dedicated background listener thread consuming from `EventHub` and invoking `cb`.
pub fn start_event_bridge(
    job: &volterra_job_t,
    cb: volterra_event_callback_fn,
    user_data: *mut libc::c_void,
) -> Result<(), volterra_error_code> {
    let rx = job.handle.subscribe_events(4096);
    let stop_signal = job.stop_signal.clone();
    let user_data_addr = user_data as usize;

    let thread_handle = std::thread::Builder::new()
        .name("volterra-event-bridge".to_string())
        .spawn(move || {
            let udata = user_data_addr as *mut libc::c_void;
            while !stop_signal.load(Ordering::Relaxed) {
                match rx.recv_timeout(std::time::Duration::from_millis(50)) {
                    Ok(event) => {
                        if let Some(callback) = cb {
                            dispatch_single_event(&event, callback, udata);
                        }
                    }
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                        continue;
                    }
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                        break;
                    }
                }
            }
        })
        .map_err(|_| volterra_error_code::ErrInvalidState)?;

    *job.event_thread.lock() = Some(thread_handle);
    Ok(())
}

fn dispatch_single_event(
    event: &volterra_core::JobEvent,
    callback: unsafe extern "C" fn(event: *const volterra_event_t, user_data: *mut libc::c_void),
    udata: *mut libc::c_void,
) {
    let (event_type, file_id, progress_bytes, total_bytes, message_str) = match event {
        volterra_core::JobEvent::StateChanged {
            old_state,
            new_state,
        } => (
            volterra_event_kind::StateChanged as u32,
            -1i64,
            0u64,
            0u64,
            format!("State changed from {:?} to {:?}", old_state, new_state),
        ),
        volterra_core::JobEvent::ScanningProgress {
            files_found,
            bytes_found,
        } => (
            volterra_event_kind::ScanningProgress as u32,
            -1i64,
            *bytes_found,
            *files_found,
            format!("Scanning: {} files, {} bytes", files_found, bytes_found),
        ),
        volterra_core::JobEvent::ScanningComplete {
            total_files,
            total_bytes,
        } => (
            volterra_event_kind::ScanningComplete as u32,
            -1i64,
            *total_bytes,
            *total_files,
            format!(
                "Scan complete: {} files, {} bytes",
                total_files, total_bytes
            ),
        ),
        volterra_core::JobEvent::FileStarted {
            file_id,
            path,
            size_bytes,
        } => (
            volterra_event_kind::FileStarted as u32,
            *file_id,
            0u64,
            *size_bytes,
            path.to_string_lossy().to_string(),
        ),
        volterra_core::JobEvent::ChunkProgress {
            file_id,
            chunk_index,
            bytes_copied,
            total_bytes,
        } => (
            volterra_event_kind::ChunkProgress as u32,
            *file_id,
            *bytes_copied,
            *total_bytes,
            format!("Chunk {} progress", chunk_index),
        ),
        volterra_core::JobEvent::FileCompleted {
            file_id,
            digest,
            duration_ms,
        } => (
            volterra_event_kind::FileCompleted as u32,
            *file_id,
            *duration_ms,
            0u64,
            digest.clone(),
        ),
        volterra_core::JobEvent::FileQuarantined { file_id, error } => (
            volterra_event_kind::FileQuarantined as u32,
            *file_id,
            0u64,
            0u64,
            error.clone(),
        ),
        volterra_core::JobEvent::TelemetryUpdated { snapshot } => (
            volterra_event_kind::TelemetryUpdated as u32,
            -1i64,
            snapshot.transferred_bytes,
            snapshot.total_bytes,
            format!("{:.2} MB/s", snapshot.current_throughput_mbps),
        ),
        volterra_core::JobEvent::BottleneckDetected {
            bottleneck,
            confidence,
        } => (
            volterra_event_kind::BottleneckDetected as u32,
            -1i64,
            (*confidence * 100.0) as u64,
            0u64,
            bottleneck.clone(),
        ),
        volterra_core::JobEvent::DiskWarning { message } => (
            volterra_event_kind::DiskWarning as u32,
            -1i64,
            0u64,
            0u64,
            message.clone(),
        ),
        volterra_core::JobEvent::JobCompleted {
            total_files,
            total_bytes,
            duration_ms,
        } => (
            volterra_event_kind::JobCompleted as u32,
            -1i64,
            *total_bytes,
            *total_files,
            format!("Job completed: {} files in {} ms", total_files, duration_ms),
        ),
        volterra_core::JobEvent::JobFailed { error } => (
            volterra_event_kind::JobFailed as u32,
            -1i64,
            0u64,
            0u64,
            error.clone(),
        ),
    };

    let c_msg = match std::ffi::CString::new(message_str) {
        Ok(s) => s,
        Err(_) => std::ffi::CString::new("").unwrap(),
    };

    let c_event = volterra_event_t {
        event_type,
        file_id,
        progress_bytes,
        total_bytes,
        message: c_msg.as_ptr(),
    };

    unsafe {
        callback(&c_event, udata);
    }
}
