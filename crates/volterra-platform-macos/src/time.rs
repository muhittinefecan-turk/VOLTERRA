use crate::error::PlatformError;
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

const ATTR_BIT_MAP_COUNT: u16 = 5;
const ATTR_CMN_CRTIME: u32 = 0x0000_0200;
const ATTR_CMN_MODTIME: u32 = 0x0000_0400;

#[repr(C)]
struct SetTimeAttrBuf {
    crtime: libc::timespec,
    modtime: libc::timespec,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FileTimestamps {
    pub btime_sec: i64,
    pub btime_nsec: i64,
    pub mtime_sec: i64,
    pub mtime_nsec: i64,
}

pub struct PlatformTime;

impl PlatformTime {
    pub fn get_timestamps(path: &Path) -> Result<FileTimestamps, PlatformError> {
        let c_path =
            CString::new(path.as_os_str().as_bytes()).map_err(|_| PlatformError::InvalidPath {
                path: path.to_path_buf(),
            })?;

        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        let ret = unsafe { libc::stat(c_path.as_ptr(), &mut st) };
        if ret != 0 {
            return Err(PlatformError::SyscallFailed {
                op: "stat",
                source: std::io::Error::last_os_error(),
            });
        }

        Ok(FileTimestamps {
            btime_sec: st.st_birthtime,
            btime_nsec: st.st_birthtime_nsec,
            mtime_sec: st.st_mtime,
            mtime_nsec: st.st_mtime_nsec,
        })
    }

    pub fn set_timestamps(path: &Path, ts: &FileTimestamps) -> Result<(), PlatformError> {
        let c_path =
            CString::new(path.as_os_str().as_bytes()).map_err(|_| PlatformError::InvalidPath {
                path: path.to_path_buf(),
            })?;

        let mut al: libc::attrlist = unsafe { std::mem::zeroed() };
        al.bitmapcount = ATTR_BIT_MAP_COUNT;
        al.commonattr = ATTR_CMN_CRTIME | ATTR_CMN_MODTIME;

        let buf = SetTimeAttrBuf {
            crtime: libc::timespec {
                tv_sec: ts.btime_sec,
                tv_nsec: ts.btime_nsec,
            },
            modtime: libc::timespec {
                tv_sec: ts.mtime_sec,
                tv_nsec: ts.mtime_nsec,
            },
        };

        let ret = unsafe {
            libc::setattrlist(
                c_path.as_ptr(),
                &mut al as *mut _ as *mut libc::c_void,
                &buf as *const _ as *mut libc::c_void,
                std::mem::size_of::<SetTimeAttrBuf>(),
                0,
            )
        };

        if ret != 0 {
            // Fallback: POSIX utimensat for mtime if setattrlist is not fully supported
            let times = [
                libc::timespec {
                    tv_sec: ts.mtime_sec,
                    tv_nsec: ts.mtime_nsec,
                },
                libc::timespec {
                    tv_sec: ts.mtime_sec,
                    tv_nsec: ts.mtime_nsec,
                },
            ];
            let ret2 =
                unsafe { libc::utimensat(libc::AT_FDCWD, c_path.as_ptr(), times.as_ptr(), 0) };
            if ret2 != 0 {
                return Err(PlatformError::SyscallFailed {
                    op: "utimensat",
                    source: std::io::Error::last_os_error(),
                });
            }
        }

        Ok(())
    }

    pub fn copy_timestamps(src: &Path, dst: &Path) -> Result<(), PlatformError> {
        let ts = Self::get_timestamps(src)?;
        Self::set_timestamps(dst, &ts)
    }
}
