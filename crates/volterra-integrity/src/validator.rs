use crate::error::IntegrityError;
use crate::readback::ReadbackVerifier;
use std::fs::File;
use std::os::unix::fs::MetadataExt;
use std::os::unix::io::AsRawFd;
use std::path::Path;

pub struct IntegrityValidator;

impl IntegrityValidator {
    /// Enforces fail-closed validation invariants before publication:
    /// 1. File size on disk matches expected source size exactly (raises TruncatedFile or OversizedFile).
    /// 2. If expected digest is supplied, verifies file digest via ReadbackVerifier (raises ChecksumMismatch).
    pub fn validate_file_for_publication(
        file: &File,
        path_hint: Option<&Path>,
        expected_size: u64,
        expected_digest: Option<&[u8; 32]>,
        verifier: &ReadbackVerifier,
    ) -> Result<(), IntegrityError> {
        let meta = file.metadata()?;
        let actual_size = meta.size();

        if actual_size < expected_size {
            return Err(IntegrityError::TruncatedFile {
                path: path_hint.map(|p| p.to_path_buf()),
                expected_bytes: expected_size,
                actual_bytes: actual_size,
            });
        }

        if actual_size > expected_size {
            return Err(IntegrityError::OversizedFile {
                path: path_hint.map(|p| p.to_path_buf()),
                expected_bytes: expected_size,
                actual_bytes: actual_size,
            });
        }

        if let Some(expected) = expected_digest {
            let matched = verifier.verify_file_fd(
                file.as_raw_fd(),
                expected_size,
                expected,
                8 * 1024 * 1024,
            )?;
            if !matched {
                // Compute actual digest for error reporting
                let mut actual_hasher = crate::blake3::Blake3FileHasher::new();
                let mut offset = 0;
                let mut buf = vec![0u8; 64 * 1024];
                while offset < expected_size {
                    let to_read = std::cmp::min(buf.len() as u64, expected_size - offset) as usize;
                    let n = unsafe {
                        libc::pread(
                            file.as_raw_fd(),
                            buf.as_mut_ptr() as *mut libc::c_void,
                            to_read,
                            offset as libc::off_t,
                        )
                    };
                    if n <= 0 {
                        break;
                    }
                    actual_hasher.update(&buf[..n as usize]);
                    offset += n as u64;
                }
                let actual = actual_hasher.finalize_digest();
                return Err(IntegrityError::ChecksumMismatch {
                    offset: 0,
                    len: expected_size as usize,
                    expected: crate::blake3::HashDigest::new(*expected).to_hex(),
                    actual: actual.to_hex(),
                });
            }
        }

        Ok(())
    }

    /// Verifies that all scheduled chunks have been verified.
    pub fn validate_chunks_complete(
        total_chunks: usize,
        verified_chunks: usize,
    ) -> Result<(), IntegrityError> {
        if verified_chunks < total_chunks {
            Err(IntegrityError::IncompleteChunkSet {
                total_chunks,
                verified_chunks,
            })
        } else {
            Ok(())
        }
    }
}
