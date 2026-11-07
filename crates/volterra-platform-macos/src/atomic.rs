use crate::error::PlatformError;
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

pub const RENAME_SWAP: libc::c_uint = 0x0000_0002;
pub const RENAME_EXCL: libc::c_uint = 0x0000_0004;

extern "C" {
    fn renamex_np(
        old: *const libc::c_char,
        new: *const libc::c_char,
        flags: libc::c_uint,
    ) -> libc::c_int;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AtomicReplaceResult {
    PublishedNew,
    SwappedExisting,
}

pub struct PlatformAtomic;

impl PlatformAtomic {
    pub fn make_partial_path(target_dest: &Path, job_id: &str) -> PathBuf {
        let parent = target_dest.parent().unwrap_or_else(|| Path::new("."));
        let file_name = target_dest
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| "unnamed".to_string());
        parent.join(format!(".volterra.{}.{}.partial", job_id, file_name))
    }

    pub fn atomic_replace(
        source_partial: &Path,
        target_dest: &Path,
        allow_overwrite: bool,
    ) -> Result<AtomicReplaceResult, PlatformError> {
        if !source_partial.exists() {
            return Err(PlatformError::NotFound(source_partial.to_path_buf()));
        }

        let c_src = CString::new(source_partial.as_os_str().as_bytes()).map_err(|_| {
            PlatformError::InvalidPath {
                path: source_partial.to_path_buf(),
            }
        })?;
        let c_dst = CString::new(target_dest.as_os_str().as_bytes()).map_err(|_| {
            PlatformError::InvalidPath {
                path: target_dest.to_path_buf(),
            }
        })?;

        if !allow_overwrite {
            // First check if target already exists to prevent accidental overwrite
            if target_dest.exists() {
                return Err(PlatformError::TargetExists(target_dest.to_path_buf()));
            }

            let ret = unsafe { renamex_np(c_src.as_ptr(), c_dst.as_ptr(), RENAME_EXCL) };
            if ret == 0 {
                return Ok(AtomicReplaceResult::PublishedNew);
            }

            let os_err = std::io::Error::last_os_error();
            match os_err.raw_os_error() {
                Some(libc::EEXIST) => Err(PlatformError::TargetExists(target_dest.to_path_buf())),
                Some(libc::ENOTSUP) | Some(libc::EINVAL) => {
                    if target_dest.exists() {
                        return Err(PlatformError::TargetExists(target_dest.to_path_buf()));
                    }
                    std::fs::rename(source_partial, target_dest)?;
                    Ok(AtomicReplaceResult::PublishedNew)
                }
                _ => Err(PlatformError::SyscallFailed {
                    op: "renamex_np(RENAME_EXCL)",
                    source: os_err,
                }),
            }
        } else {
            // Overwrite allowed: if destination exists, use atomic swap
            if target_dest.exists() {
                let ret = unsafe { renamex_np(c_src.as_ptr(), c_dst.as_ptr(), RENAME_SWAP) };
                if ret == 0 {
                    // source_partial now holds old destination; unlink it safely
                    let _ = std::fs::remove_file(source_partial);
                    Ok(AtomicReplaceResult::SwappedExisting)
                } else {
                    // Fallback to standard replace
                    std::fs::rename(source_partial, target_dest)?;
                    Ok(AtomicReplaceResult::SwappedExisting)
                }
            } else {
                let ret = unsafe { renamex_np(c_src.as_ptr(), c_dst.as_ptr(), RENAME_EXCL) };
                if ret == 0 {
                    Ok(AtomicReplaceResult::PublishedNew)
                } else {
                    std::fs::rename(source_partial, target_dest)?;
                    Ok(AtomicReplaceResult::PublishedNew)
                }
            }
        }
    }
}
