use crate::error::PlatformError;
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

pub const XATTR_NOFOLLOW: libc::c_int = 0x0001;

extern "C" {
    fn listxattr(
        path: *const libc::c_char,
        namebuf: *mut libc::c_char,
        size: libc::size_t,
        options: libc::c_int,
    ) -> libc::ssize_t;
    fn getxattr(
        path: *const libc::c_char,
        name: *const libc::c_char,
        value: *mut libc::c_void,
        size: libc::size_t,
        position: u32,
        options: libc::c_int,
    ) -> libc::ssize_t;
    fn setxattr(
        path: *const libc::c_char,
        name: *const libc::c_char,
        value: *const libc::c_void,
        size: libc::size_t,
        position: u32,
        options: libc::c_int,
    ) -> libc::c_int;
    fn removexattr(
        path: *const libc::c_char,
        name: *const libc::c_char,
        options: libc::c_int,
    ) -> libc::c_int;
}

pub struct PlatformXattr;

impl PlatformXattr {
    pub fn list_xattrs(path: &Path) -> Result<Vec<String>, PlatformError> {
        let c_path =
            CString::new(path.as_os_str().as_bytes()).map_err(|_| PlatformError::InvalidPath {
                path: path.to_path_buf(),
            })?;

        let size = unsafe { listxattr(c_path.as_ptr(), std::ptr::null_mut(), 0, XATTR_NOFOLLOW) };
        if size <= 0 {
            return Ok(Vec::new());
        }

        let mut buf = vec![0u8; size as usize];
        let ret = unsafe {
            listxattr(
                c_path.as_ptr(),
                buf.as_mut_ptr() as *mut libc::c_char,
                buf.len(),
                XATTR_NOFOLLOW,
            )
        };
        if ret <= 0 {
            return Ok(Vec::new());
        }

        let mut attrs = Vec::new();
        let mut start = 0;
        for i in 0..ret as usize {
            if buf[i] == 0 {
                if let Ok(s) = std::str::from_utf8(&buf[start..i]) {
                    if !s.is_empty() {
                        attrs.push(s.to_string());
                    }
                }
                start = i + 1;
            }
        }
        Ok(attrs)
    }

    pub fn get_xattr(path: &Path, name: &str) -> Result<Option<Vec<u8>>, PlatformError> {
        let c_path =
            CString::new(path.as_os_str().as_bytes()).map_err(|_| PlatformError::InvalidPath {
                path: path.to_path_buf(),
            })?;
        let c_name = CString::new(name.as_bytes()).map_err(|_| PlatformError::InvalidPath {
            path: path.to_path_buf(),
        })?;

        let size = unsafe {
            getxattr(
                c_path.as_ptr(),
                c_name.as_ptr(),
                std::ptr::null_mut(),
                0,
                0,
                XATTR_NOFOLLOW,
            )
        };
        if size < 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::ENOATTR) {
                return Ok(None);
            }
            return Err(PlatformError::SyscallFailed {
                op: "getxattr",
                source: err,
            });
        }

        let mut buf = vec![0u8; size as usize];
        let ret = unsafe {
            getxattr(
                c_path.as_ptr(),
                c_name.as_ptr(),
                buf.as_mut_ptr() as *mut libc::c_void,
                buf.len(),
                0,
                XATTR_NOFOLLOW,
            )
        };
        if ret < 0 {
            return Err(PlatformError::SyscallFailed {
                op: "getxattr",
                source: std::io::Error::last_os_error(),
            });
        }

        Ok(Some(buf))
    }

    pub fn set_xattr(path: &Path, name: &str, value: &[u8]) -> Result<(), PlatformError> {
        let c_path =
            CString::new(path.as_os_str().as_bytes()).map_err(|_| PlatformError::InvalidPath {
                path: path.to_path_buf(),
            })?;
        let c_name = CString::new(name.as_bytes()).map_err(|_| PlatformError::InvalidPath {
            path: path.to_path_buf(),
        })?;

        let ret = unsafe {
            setxattr(
                c_path.as_ptr(),
                c_name.as_ptr(),
                value.as_ptr() as *const libc::c_void,
                value.len(),
                0,
                XATTR_NOFOLLOW,
            )
        };

        if ret != 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::ENOTSUP) || err.raw_os_error() == Some(libc::EPERM)
            {
                return Err(PlatformError::NotSupported {
                    feature: "xattr",
                    path: path.display().to_string(),
                });
            }
            return Err(PlatformError::SyscallFailed {
                op: "setxattr",
                source: err,
            });
        }
        Ok(())
    }

    pub fn remove_xattr(path: &Path, name: &str) -> Result<(), PlatformError> {
        let c_path =
            CString::new(path.as_os_str().as_bytes()).map_err(|_| PlatformError::InvalidPath {
                path: path.to_path_buf(),
            })?;
        let c_name = CString::new(name.as_bytes()).map_err(|_| PlatformError::InvalidPath {
            path: path.to_path_buf(),
        })?;

        let ret = unsafe { removexattr(c_path.as_ptr(), c_name.as_ptr(), XATTR_NOFOLLOW) };
        if ret != 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::ENOATTR) {
                return Ok(());
            }
            return Err(PlatformError::SyscallFailed {
                op: "removexattr",
                source: err,
            });
        }
        Ok(())
    }

    pub fn copy_xattrs(src: &Path, dst: &Path) -> Result<usize, PlatformError> {
        let attrs = Self::list_xattrs(src)?;
        let mut count = 0;
        for attr in attrs {
            if let Ok(Some(val)) = Self::get_xattr(src, &attr) {
                if Self::set_xattr(dst, &attr, &val).is_ok() {
                    count += 1;
                }
            }
        }
        Ok(count)
    }
}
