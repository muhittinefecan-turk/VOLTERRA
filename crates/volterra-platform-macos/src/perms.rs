use crate::error::PlatformError;
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FilePermissions {
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
}

pub struct PlatformPerms;

impl PlatformPerms {
    pub fn get_permissions(path: &Path) -> Result<FilePermissions, PlatformError> {
        let c_path =
            CString::new(path.as_os_str().as_bytes()).map_err(|_| PlatformError::InvalidPath {
                path: path.to_path_buf(),
            })?;

        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        let ret = unsafe { libc::lstat(c_path.as_ptr(), &mut st) };
        if ret != 0 {
            return Err(PlatformError::SyscallFailed {
                op: "lstat",
                source: std::io::Error::last_os_error(),
            });
        }

        Ok(FilePermissions {
            mode: (st.st_mode & 0o7777) as u32,
            uid: st.st_uid,
            gid: st.st_gid,
        })
    }

    pub fn set_permissions(path: &Path, mode: u32) -> Result<(), PlatformError> {
        let c_path =
            CString::new(path.as_os_str().as_bytes()).map_err(|_| PlatformError::InvalidPath {
                path: path.to_path_buf(),
            })?;

        let ret = unsafe { libc::chmod(c_path.as_ptr(), (mode & 0o7777) as libc::mode_t) };
        if ret != 0 {
            return Err(PlatformError::SyscallFailed {
                op: "chmod",
                source: std::io::Error::last_os_error(),
            });
        }
        Ok(())
    }

    pub fn copy_permissions(src: &Path, dst: &Path) -> Result<(), PlatformError> {
        let perms = Self::get_permissions(src)?;
        Self::set_permissions(dst, perms.mode)?;

        // Attempt chown; ignore EPERM if non-root user
        let c_dst =
            CString::new(dst.as_os_str().as_bytes()).map_err(|_| PlatformError::InvalidPath {
                path: dst.to_path_buf(),
            })?;
        let _ = unsafe { libc::chown(c_dst.as_ptr(), perms.uid, perms.gid) };
        Ok(())
    }
}
