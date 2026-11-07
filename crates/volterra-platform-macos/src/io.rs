use crate::error::PlatformError;
use std::os::unix::io::RawFd;

pub const F_NOCACHE: libc::c_int = 48;
pub const F_GLOBAL_NOCACHE: libc::c_int = 55;
pub const F_RDADVISE: libc::c_int = 44;
pub const F_RDAHEAD: libc::c_int = 45;

#[repr(C)]
pub struct Radvisory {
    pub ra_offset: libc::off_t,
    pub ra_count: libc::c_int,
}

pub struct PlatformIo;

impl PlatformIo {
    /// Configures direct I/O for the specified file descriptor, bypassing Darwin UBC page cache.
    pub fn set_direct_io(fd: RawFd) -> Result<(), PlatformError> {
        let ret = unsafe { libc::fcntl(fd, F_NOCACHE, 1) };
        if ret == -1 {
            return Err(PlatformError::SyscallFailed {
                op: "fcntl(F_NOCACHE)",
                source: std::io::Error::last_os_error(),
            });
        }
        let _ = unsafe { libc::fcntl(fd, F_GLOBAL_NOCACHE, 1) };
        Ok(())
    }

    /// Disables caching globally for all descriptors referencing the underlying vnode.
    pub fn set_global_nocache(fd: RawFd) -> Result<(), PlatformError> {
        let ret = unsafe { libc::fcntl(fd, F_GLOBAL_NOCACHE, 1) };
        if ret == -1 {
            Err(PlatformError::SyscallFailed {
                op: "fcntl(F_GLOBAL_NOCACHE)",
                source: std::io::Error::last_os_error(),
            })
        } else {
            Ok(())
        }
    }

    /// Supplies read-ahead advisory via Darwin fcntl(F_RDADVISE).
    /// Edge-case guard: if length is 0, silently returns Ok(()) without issuing syscall.
    pub fn advise_readahead(fd: RawFd, offset: u64, length: usize) -> Result<(), PlatformError> {
        if length == 0 {
            return Ok(());
        }

        let clamped_len = length.min(i32::MAX as usize) as libc::c_int;
        let radv = Radvisory {
            ra_offset: offset as libc::off_t,
            ra_count: clamped_len,
        };
        let ret = unsafe { libc::fcntl(fd, F_RDADVISE, &radv) };
        if ret == -1 {
            // Non-fatal advisory; return Err if invalid fd
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::EBADF) {
                return Err(PlatformError::SyscallFailed {
                    op: "fcntl(F_RDADVISE)",
                    source: err,
                });
            }
        }
        Ok(())
    }

    /// Enables sequential read-ahead optimization via Darwin fcntl(F_RDAHEAD).
    pub fn advise_sequential(fd: RawFd) -> Result<(), PlatformError> {
        let ret = unsafe { libc::fcntl(fd, F_RDAHEAD, 1) };
        if ret == -1 {
            Err(PlatformError::SyscallFailed {
                op: "fcntl(F_RDAHEAD)",
                source: std::io::Error::last_os_error(),
            })
        } else {
            Ok(())
        }
    }

    /// Alias for advise_sequential.
    pub fn enable_readahead(fd: RawFd) -> Result<(), PlatformError> {
        Self::advise_sequential(fd)
    }
}
