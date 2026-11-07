use crate::error::TransferError;
use std::fs::File;
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::sync::Arc;
use volterra_integrity::Blake3Hasher;
use volterra_platform_macos::PlatformIo;

/// Software fault injection hook for intercepting source read operations.
pub trait ReadFaultInjector: Send + Sync {
    /// Called before `libc::pread`.
    /// - Return `Some(Ok(bytes))` to simulate short reads or EOF (if bytes == 0).
    /// - Return `Some(Err(e))` to simulate hardware bad sectors (`EIO`), permissions (`EACCES`), or disconnects (`ENODEV`).
    /// - Return `None` to proceed with normal read from disk.
    fn on_pread(
        &self,
        _offset: u64,
        _length: usize,
        _buffer: &mut [u8],
    ) -> Option<Result<usize, std::io::Error>> {
        None
    }
}

pub struct ChunkReader {
    file: File,
    direct_io: bool,
    fault_injector: Option<Arc<dyn ReadFaultInjector>>,
}

impl ChunkReader {
    pub fn open(path: &Path, direct_io: bool) -> Result<Self, std::io::Error> {
        let file = std::fs::OpenOptions::new().read(true).open(path)?;
        if direct_io {
            let _ = PlatformIo::set_direct_io(file.as_raw_fd());
        }
        let _ = PlatformIo::advise_sequential(file.as_raw_fd());
        Ok(Self {
            file,
            direct_io,
            fault_injector: None,
        })
    }

    pub fn with_fault_injector(mut self, injector: Arc<dyn ReadFaultInjector>) -> Self {
        self.fault_injector = Some(injector);
        self
    }

    pub fn read_chunk(
        &self,
        offset: u64,
        length: usize,
        buffer: &mut [u8],
    ) -> Result<[u8; 32], TransferError> {
        if length > buffer.len() {
            return Err(TransferError::SourceReadError {
                offset,
                len: length,
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "Buffer too small for chunk length",
                ),
            });
        }

        if length == 0 {
            return Ok(Blake3Hasher::hash_chunk(&[]));
        }

        let _ = PlatformIo::advise_readahead(self.file.as_raw_fd(), offset, length);

        let mut bytes_read = 0;
        while bytes_read < length {
            let to_read = length - bytes_read;
            let (effective_to_read, injected_res) = if let Some(ref fi) = self.fault_injector {
                match fi.on_pread(offset + bytes_read as u64, to_read, &mut buffer[bytes_read..]) {
                    Some(Ok(clamped)) => {
                        if clamped == 0 {
                            (0, Some(Ok(0)))
                        } else {
                            (clamped.min(to_read), None)
                        }
                    }
                    Some(Err(e)) => (to_read, Some(Err(e))),
                    None => (to_read, None),
                }
            } else {
                (to_read, None)
            };

            let res = if let Some(injected) = injected_res {
                match injected {
                    Ok(n) => n as libc::ssize_t,
                    Err(e) => {
                        let raw = e.raw_os_error().unwrap_or(libc::EIO);
                        if raw == libc::EINTR {
                            continue;
                        }
                        return Err(TransferError::SourceReadError {
                            offset: offset + bytes_read as u64,
                            len: to_read,
                            source: e,
                        });
                    }
                }
            } else {
                unsafe {
                    libc::pread(
                        self.file.as_raw_fd(),
                        buffer[bytes_read..].as_mut_ptr() as *mut libc::c_void,
                        effective_to_read,
                        (offset + bytes_read as u64) as libc::off_t,
                    )
                }
            };

            if res > 0 {
                bytes_read += res as usize;
            } else if res == 0 {
                return Err(TransferError::UnexpectedEof {
                    offset: offset + bytes_read as u64,
                    expected: length,
                    actual: bytes_read,
                });
            } else {
                let err = std::io::Error::last_os_error();
                if err.raw_os_error() == Some(libc::EINTR) {
                    continue;
                }
                return Err(TransferError::SourceReadError {
                    offset: offset + bytes_read as u64,
                    len: to_read,
                    source: err,
                });
            }
        }

        let digest = Blake3Hasher::hash_chunk(&buffer[..length]);
        Ok(digest)
    }

    pub fn file(&self) -> &File {
        &self.file
    }

    pub fn is_direct_io(&self) -> bool {
        self.direct_io
    }
}
