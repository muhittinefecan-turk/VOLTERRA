//! Memory allocation helpers and dedicated deallocators for the VOLTERRA C ABI.

use crate::types::*;
use std::ffi::CString;
use std::os::raw::c_char;
use std::sync::atomic::Ordering;

/// Converts a Rust string slice into a heap-allocated C string.
/// The caller (or deallocator) takes ownership of the returned pointer.
pub fn alloc_c_string(s: &str) -> *mut c_char {
    match CString::new(s) {
        Ok(c_str) => c_str.into_raw(),
        Err(_) => {
            // String contains interior null bytes; sanitize by replacing nulls
            let sanitized: String = s.chars().map(|c| if c == '\0' { ' ' } else { c }).collect();
            CString::new(sanitized)
                .unwrap_or_else(|_| CString::new("").unwrap())
                .into_raw()
        }
    }
}

/// Converts an optional Rust string slice into a heap-allocated C string, or null if None.
pub fn alloc_opt_c_string(opt: Option<&str>) -> *mut c_char {
    match opt {
        Some(s) => alloc_c_string(s),
        None => std::ptr::null_mut(),
    }
}

/// Deallocates an opaque job controller handle.
/// Safe no-op if `job` is null.
#[no_mangle]
pub unsafe extern "C" fn volterra_job_free(job: *mut volterra_job_t) {
    if job.is_null() {
        return;
    }

    let boxed = Box::from_raw(job);
    boxed.stop_signal.store(true, Ordering::SeqCst);

    // Cancel in-flight background worker pipeline if active
    let _ = boxed.handle.cancel();

    // Wait for the background event-bridge thread to finish
    let thread_handle = boxed.event_thread.lock().take();
    if let Some(handle) = thread_handle {
        let _ = handle.join();
    }
    // `boxed` drops here, cleaning up JobHandle and all internal Arcs
}

/// Deallocates a snapshot structure and its nested strings.
/// Safe no-op if `snap` is null.
#[no_mangle]
pub unsafe extern "C" fn volterra_snapshot_free(snap: *mut volterra_snapshot_t) {
    if snap.is_null() {
        return;
    }

    let snap = Box::from_raw(snap);
    if !snap.job_id.is_null() {
        drop(CString::from_raw(snap.job_id));
    }
    if !snap.active_bottleneck.is_null() {
        drop(CString::from_raw(snap.active_bottleneck));
    }
    // Box drops
}

/// Deallocates a telemetry structure and its nested strings.
/// Safe no-op if `tel` is null.
#[no_mangle]
pub unsafe extern "C" fn volterra_telemetry_free(tel: *mut volterra_telemetry_t) {
    if tel.is_null() {
        return;
    }

    let tel = Box::from_raw(tel);
    if !tel.bottleneck_message.is_null() {
        drop(CString::from_raw(tel.bottleneck_message));
    }
    // Box drops
}

/// Deallocates an error list, its error items, and nested extent arrays and strings.
/// Safe no-op if `list` is null.
#[no_mangle]
pub unsafe extern "C" fn volterra_error_list_free(list: *mut volterra_error_list_t) {
    if list.is_null() {
        return;
    }

    let list = Box::from_raw(list);
    if !list.items.is_null() && list.count > 0 {
        let items_slice = std::slice::from_raw_parts_mut(list.items, list.count);
        for item in items_slice.iter_mut() {
            if !item.path.is_null() {
                drop(CString::from_raw(item.path));
            }
            if !item.error_message.is_null() {
                drop(CString::from_raw(item.error_message));
            }
            if !item.extents.is_null() && item.failed_extents_count > 0 {
                let extents_slice =
                    std::slice::from_raw_parts_mut(item.extents, item.failed_extents_count);
                let extents_box = Box::from_raw(extents_slice as *mut [volterra_extent_t]);
                drop(extents_box);
            }
        }
        let items_box = Box::from_raw(items_slice as *mut [volterra_error_item_t]);
        drop(items_box);
    }
    // Box drops
}

/// Deallocates a disk list, disk structures, and nested profile strings.
/// Safe no-op if `disks` is null.
#[no_mangle]
pub unsafe extern "C" fn volterra_disks_free(disks: *mut volterra_disk_list_t) {
    if disks.is_null() {
        return;
    }

    let disks = Box::from_raw(disks);
    if !disks.disks.is_null() && disks.count > 0 {
        let slice = std::slice::from_raw_parts_mut(disks.disks, disks.count);
        for d in slice.iter_mut() {
            if !d.device_id.is_null() {
                drop(CString::from_raw(d.device_id));
            }
            if !d.bsd_name.is_null() {
                drop(CString::from_raw(d.bsd_name));
            }
            if !d.mount_point.is_null() {
                drop(CString::from_raw(d.mount_point));
            }
            if !d.volume_name.is_null() {
                drop(CString::from_raw(d.volume_name));
            }
            if !d.fs_type.is_null() {
                drop(CString::from_raw(d.fs_type));
            }
            if !d.parent_bsd_name.is_null() {
                drop(CString::from_raw(d.parent_bsd_name));
            }
            if !d.model.is_null() {
                drop(CString::from_raw(d.model));
            }
            if !d.vendor.is_null() {
                drop(CString::from_raw(d.vendor));
            }
            if !d.transport.is_null() {
                drop(CString::from_raw(d.transport));
            }
        }
        let slice_box = Box::from_raw(slice as *mut [volterra_disk_t]);
        drop(slice_box);
    }
    // Box drops
}

/// Deallocates a Rust-allocated C string (e.g. from `volterra_export_report`).
/// Safe no-op if `str` is null.
#[no_mangle]
pub unsafe extern "C" fn volterra_string_free(str: *mut c_char) {
    if !str.is_null() {
        drop(CString::from_raw(str));
    }
}

/// Deallocates a `volterra_error_t` and its nested message string.
/// Safe no-op if `err` is null.
#[no_mangle]
pub unsafe extern "C" fn volterra_error_free(err: *mut volterra_error_t) {
    if !err.is_null() {
        let err = Box::from_raw(err);
        if !err.message.is_null() {
            drop(CString::from_raw(err.message));
        }
    }
}
