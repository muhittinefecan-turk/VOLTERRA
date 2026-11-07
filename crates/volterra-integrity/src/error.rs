use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum IntegrityError {
    #[error(
        "Checksum mismatch at offset {offset} (len {len}): expected {expected}, actual {actual}"
    )]
    ChecksumMismatch {
        offset: u64,
        len: usize,
        expected: String,
        actual: String,
    },

    #[error("Digest mismatch at offset {offset}: expected {expected}, got {actual}")]
    DigestMismatch {
        offset: u64,
        expected: String,
        actual: String,
    },

    #[error("File truncated: expected {expected_bytes} bytes, found {actual_bytes} bytes (file: {path:?})")]
    TruncatedFile {
        path: Option<PathBuf>,
        expected_bytes: u64,
        actual_bytes: u64,
    },

    #[error("File oversized: expected {expected_bytes} bytes, found {actual_bytes} bytes (file: {path:?})")]
    OversizedFile {
        path: Option<PathBuf>,
        expected_bytes: u64,
        actual_bytes: u64,
    },

    #[error("Short read during verification: expected {expected} bytes, read {actual} bytes")]
    ShortRead { expected: usize, actual: usize },

    #[error("Unexpected EOF during readback at offset {offset}: expected {expected_bytes} bytes, read {actual_bytes} bytes")]
    UnexpectedEof {
        offset: u64,
        expected_bytes: u64,
        actual_bytes: u64,
    },

    #[error("Incomplete chunk set: {verified_chunks} verified out of {total_chunks} total chunks")]
    IncompleteChunkSet {
        total_chunks: usize,
        verified_chunks: usize,
    },

    #[error("Invalid hex digest string: {0}")]
    InvalidDigestFormat(String),

    #[error("Underlying I/O error during integrity verification: {0}")]
    Io(#[from] std::io::Error),
}
