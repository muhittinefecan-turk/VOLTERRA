use crate::error::TransferError;
use std::fs::{File, OpenOptions};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use volterra_platform_macos::PlatformIo;

/// Software fault injection hook for intercepting destination write operations.
pub trait WriteFaultInjector: Send + Sync {
    /// Called before `libc::pwrite`.
    /// - Return `Some(Ok(bytes))` to simulate short writes or zero byte writes (if bytes == 0).
    /// - Return `Some(Err(e))` to simulate disk full (`ENOSPC`) or disconnects (`ENODEV`).
    /// - Return `None` to proceed with normal write to disk.
    fn on_pwrite(
        &self,
        _offset: u64,
        _data: &[u8],
    ) -> Option<Result<usize, std::io::Error>> {
        None
    }

    /// Optional byte-level data transformer (e.g. to inject bit-flips or silent corruptions).
    fn transform_data(&self, _offset: u64, _data: &[u8]) -> Option<Vec<u8>> {
        None
    }
}

pub struct ChunkWriter {
    file: File,
    path: PathBuf,
    direct_io: bool,
    fault_injector: Option<Arc<dyn WriteFaultInjector>>,
}

#[inline]
fn is_all_zero(slice: &[u8]) -> bool {
    let (prefix, words, suffix) = unsafe { slice.align_to::<usize>() };
    prefix.iter().all(|&b| b == 0)
        && words.iter().all(|&w| w == 0)
        && suffix.iter().all(|&b| b == 0)
}

impl ChunkWriter {
    pub fn open_or_create_partial(path: &Path, direct_io: bool) -> Result<Self, std::io::Error> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(path)?;

        if direct_io {
            let _ = PlatformIo::set_direct_io(file.as_raw_fd());
        }
        Ok(Self {
            file,
            path: path.to_path_buf(),
            direct_io,
            fault_injector: None,
        })
    }

    pub fn with_fault_injector(mut self, injector: Arc<dyn WriteFaultInjector>) -> Self {
        self.fault_injector = Some(injector);
        self
    }

    pub fn set_len(&self, size: u64) -> Result<(), std::io::Error> {
        self.file.set_len(size)
    }

    pub fn write_chunk(&self, offset: u64, data: &[u8]) -> Result<(), TransferError> {
        let length = data.len();
        if length == 0 {
            return Ok(());
        }

        // If a fault injector is configured, bypass block-level coalescing so injected
        // errors / corruptions apply directly to the requested slice.
        if self.fault_injector.is_some() {
            return self.write_raw(offset, data);
        }

        const BLOCK_SIZE: usize = 4096;
        let mut blk_offset = 0;
        let mut span_start: Option<usize> = None;

        while blk_offset < length {
            let blk_len = std::cmp::min(BLOCK_SIZE, length - blk_offset);
            let block = &data[blk_offset..blk_offset + blk_len];
            let is_zero = is_all_zero(block);

            if is_zero {
                if let Some(start) = span_start {
                    self.write_raw(offset + start as u64, &data[start..blk_offset])?;
                    span_start = None;
                }
            } else if span_start.is_none() {
                span_start = Some(blk_offset);
            }

            blk_offset += blk_len;
        }

        if let Some(start) = span_start {
            self.write_raw(offset + start as u64, &data[start..length])?;
        }

        Ok(())
    }

    fn write_raw(&self, offset: u64, data: &[u8]) -> Result<(), TransferError> {
        let length = data.len();
        if length == 0 {
            return Ok(());
        }

        let transformed;
        let data = if let Some(ref fi) = self.fault_injector {
            if let Some(t) = fi.transform_data(offset, data) {
                transformed = t;
                &transformed[..]
            } else {
                data
            }
        } else {
            data
        };

        let mut bytes_written = 0;
        while bytes_written < length {
            let to_write = length - bytes_written;
            let (effective_to_write, injected_res) = if let Some(ref fi) = self.fault_injector {
                match fi.on_pwrite(offset + bytes_written as u64, &data[bytes_written..]) {
                    Some(Ok(clamped)) => {
                        if clamped == 0 {
                            (0, Some(Ok(0)))
                        } else {
                            (clamped.min(to_write), None)
                        }
                    }
                    Some(Err(e)) => (to_write, Some(Err(e))),
                    None => (to_write, None),
                }
            } else {
                (to_write, None)
            };

            let res = if let Some(injected) = injected_res {
                match injected {
                    Ok(n) => n as libc::ssize_t,
                    Err(e) => {
                        let raw = e.raw_os_error().unwrap_or(libc::EIO);
                        if raw == libc::EINTR {
                            continue;
                        }
                        if raw == libc::ENOSPC {
                            return Err(TransferError::DiskFull {
                                path: self.path.clone(),
                            });
                        }
                        return Err(TransferError::DestinationWriteError {
                            offset: offset + bytes_written as u64,
                            len: to_write,
                            source: e,
                        });
                    }
                }
            } else {
                unsafe {
                    libc::pwrite(
                        self.file.as_raw_fd(),
                        data[bytes_written..].as_ptr() as *const libc::c_void,
                        effective_to_write,
                        (offset + bytes_written as u64) as libc::off_t,
                    )
                }
            };

            if res > 0 {
                bytes_written += res as usize;
            } else if res == 0 {
                return Err(TransferError::ZeroByteWrite {
                    offset: offset + bytes_written as u64,
                });
            } else {
                let err = std::io::Error::last_os_error();
                if err.raw_os_error() == Some(libc::EINTR) {
                    continue;
                }
                if err.raw_os_error() == Some(libc::ENOSPC) {
                    return Err(TransferError::DiskFull {
                        path: self.path.clone(),
                    });
                }
                return Err(TransferError::DestinationWriteError {
                    offset: offset + bytes_written as u64,
                    len: to_write,
                    source: err,
                });
            }
        }

        Ok(())
    }

    pub fn sync_all(&self) -> Result<(), std::io::Error> {
        self.file.sync_all()
    }

    pub fn file(&self) -> &File {
        &self.file
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn is_direct_io(&self) -> bool {
        self.direct_io
    }
}
