use crate::blake3::{Blake3FileHasher, Blake3Hasher, HashDigest};
use crate::error::IntegrityError;
use std::fs::File;
use std::os::unix::fs::MetadataExt;
use std::os::unix::io::{AsRawFd, RawFd};
use std::path::Path;

pub const F_NOCACHE: libc::c_int = 48;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationOutcome {
    Match,
    Mismatch {
        expected: HashDigest,
        actual: HashDigest,
    },
}

pub struct ReadbackVerifier {
    enable_direct_io: bool,
}

impl Default for ReadbackVerifier {
    fn default() -> Self {
        Self::new()
    }
}

impl ReadbackVerifier {
    pub fn new() -> Self {
        Self {
            enable_direct_io: true,
        }
    }

    pub fn with_direct_io(enable: bool) -> Self {
        Self {
            enable_direct_io: enable,
        }
    }

    pub fn is_direct_io_enabled(&self) -> bool {
        self.enable_direct_io
    }

    /// Verifies a specific byte range on storage media by reading via pread
    /// and comparing against expected 256-bit BLAKE3 digest.
    /// Returns Ok(true) if match, Ok(false) if mismatch.
    pub fn verify_range(
        &self,
        fd: RawFd,
        offset: u64,
        expected_digest: &[u8; 32],
        len: usize,
    ) -> Result<bool, IntegrityError> {
        if self.enable_direct_io {
            unsafe {
                libc::fcntl(fd, F_NOCACHE, 1);
            }
        }

        let mut buffer = vec![0u8; len];
        let mut bytes_read = 0;

        while bytes_read < len {
            let res = unsafe {
                libc::pread(
                    fd,
                    buffer[bytes_read..].as_mut_ptr() as *mut libc::c_void,
                    len - bytes_read,
                    (offset + bytes_read as u64) as libc::off_t,
                )
            };

            if res < 0 {
                let err = std::io::Error::last_os_error();
                if err.kind() == std::io::ErrorKind::Interrupted {
                    continue; // EINTR retry
                }
                return Err(IntegrityError::Io(err));
            }

            if res == 0 {
                // Premature EOF before requested chunk range is satisfied
                return Err(IntegrityError::UnexpectedEof {
                    offset,
                    expected_bytes: len as u64,
                    actual_bytes: bytes_read as u64,
                });
            }

            bytes_read += res as usize;
        }

        let actual_digest = Blake3Hasher::hash_chunk(&buffer);
        Ok(actual_digest == *expected_digest)
    }

    /// Verifies a byte range, returning detailed VerificationOutcome.
    pub fn verify_range_outcome(
        &self,
        fd: RawFd,
        offset: u64,
        expected_digest: &HashDigest,
        len: usize,
    ) -> Result<VerificationOutcome, IntegrityError> {
        if self.enable_direct_io {
            unsafe {
                libc::fcntl(fd, F_NOCACHE, 1);
            }
        }

        let mut buffer = vec![0u8; len];
        let mut bytes_read = 0;

        while bytes_read < len {
            let res = unsafe {
                libc::pread(
                    fd,
                    buffer[bytes_read..].as_mut_ptr() as *mut libc::c_void,
                    len - bytes_read,
                    (offset + bytes_read as u64) as libc::off_t,
                )
            };

            if res < 0 {
                let err = std::io::Error::last_os_error();
                if err.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(IntegrityError::Io(err));
            }

            if res == 0 {
                return Err(IntegrityError::UnexpectedEof {
                    offset,
                    expected_bytes: len as u64,
                    actual_bytes: bytes_read as u64,
                });
            }

            bytes_read += res as usize;
        }

        let actual_digest = Blake3Hasher::hash_to_digest(&buffer);
        if actual_digest == *expected_digest {
            Ok(VerificationOutcome::Match)
        } else {
            Ok(VerificationOutcome::Mismatch {
                expected: *expected_digest,
                actual: actual_digest,
            })
        }
    }

    /// Verifies the complete file on physical media given a file path and expected digest.
    pub fn verify_file(
        &self,
        path: &Path,
        expected_digest: &[u8; 32],
    ) -> Result<bool, IntegrityError> {
        let file = File::open(path)?;
        let total_size = file.metadata()?.size();
        self.verify_file_fd(
            file.as_raw_fd(),
            total_size,
            expected_digest,
            8 * 1024 * 1024,
        )
    }

    /// Verifies the file from an open raw descriptor in chunked passes.
    pub fn verify_file_fd(
        &self,
        fd: RawFd,
        total_size: u64,
        expected_digest: &[u8; 32],
        chunk_size: usize,
    ) -> Result<bool, IntegrityError> {
        if self.enable_direct_io {
            unsafe {
                libc::fcntl(fd, F_NOCACHE, 1);
            }
        }

        if total_size == 0 {
            let empty_digest = Blake3Hasher::hash_chunk(&[]);
            return Ok(empty_digest == *expected_digest);
        }

        let mut file_hasher = Blake3FileHasher::new();
        let mut offset: u64 = 0;
        let mut buffer = vec![0u8; chunk_size];

        while offset < total_size {
            let to_read = std::cmp::min(chunk_size as u64, total_size - offset) as usize;
            let mut chunk_bytes_read = 0;

            while chunk_bytes_read < to_read {
                let res = unsafe {
                    libc::pread(
                        fd,
                        buffer[chunk_bytes_read..to_read].as_mut_ptr() as *mut libc::c_void,
                        to_read - chunk_bytes_read,
                        (offset + chunk_bytes_read as u64) as libc::off_t,
                    )
                };

                if res < 0 {
                    let err = std::io::Error::last_os_error();
                    if err.kind() == std::io::ErrorKind::Interrupted {
                        continue;
                    }
                    return Err(IntegrityError::Io(err));
                }

                if res == 0 {
                    return Err(IntegrityError::UnexpectedEof {
                        offset,
                        expected_bytes: total_size,
                        actual_bytes: offset + chunk_bytes_read as u64,
                    });
                }

                chunk_bytes_read += res as usize;
            }

            file_hasher.update(&buffer[..to_read]);
            offset += to_read as u64;
        }

        let actual_digest = file_hasher.finalize();
        Ok(actual_digest == *expected_digest)
    }
}
