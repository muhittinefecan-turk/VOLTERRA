use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum TransferError {
    #[error("Preflight capacity exceeded: required {required_bytes} bytes, available {available_bytes} bytes on '{target_path}'")]
    PreflightCapacityExceeded {
        required_bytes: u64,
        available_bytes: u64,
        target_path: PathBuf,
    },

    #[error("Target volume is read-only: '{0}'")]
    TargetReadOnly(PathBuf),

    #[error("Destination file collision: '{0}' already exists and overwrite is disabled")]
    DestinationCollision(PathBuf),

    #[error("Source read I/O error at offset {offset} (len {len}): {source}")]
    SourceReadError {
        offset: u64,
        len: usize,
        #[source]
        source: std::io::Error,
    },

    #[error("Destination write I/O error at offset {offset} (len {len}): {source}")]
    DestinationWriteError {
        offset: u64,
        len: usize,
        #[source]
        source: std::io::Error,
    },

    #[error("Destination storage full (ENOSPC) writing to '{path}'")]
    DiskFull { path: PathBuf },

    #[error(
        "Unexpected EOF reading source at offset {offset}: expected {expected} bytes, got {actual}"
    )]
    UnexpectedEof {
        offset: u64,
        expected: usize,
        actual: usize,
    },

    #[error("Zero bytes written to target at offset {offset}")]
    ZeroByteWrite { offset: u64 },

    #[error("Source file mutated during transfer: {0:?}")]
    SourceMutated(volterra_manifest::MutationStatus),

    #[error("Integrity validation failed: {0}")]
    IntegrityFailed(#[from] volterra_integrity::IntegrityError),

    #[error("Platform syscall failed: {0}")]
    PlatformFailed(#[from] volterra_platform_macos::PlatformError),

    #[error("Journal error: {0}")]
    JournalFailed(#[from] volterra_journal::JournalError),

    #[error("Recovery error: {0}")]
    RecoveryFailed(#[from] volterra_recovery::RecoveryError),

    #[error("Profiler error: {0}")]
    ProfilerFailed(#[from] volterra_profiler::ProfilerError),

    #[error("Transfer was cancelled by user")]
    Cancelled,

    #[error("Transfer paused")]
    Paused,
}
