use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PlatformError {
    #[error("I/O error at {path:?}: {source}")]
    IoWithPath {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Darwin syscall '{op}' failed: {source}")]
    SyscallFailed {
        op: &'static str,
        source: std::io::Error,
    },

    #[error("Invalid path or UTF-8 conversion: {path}")]
    InvalidPath { path: PathBuf },

    #[error("Volume UUID query failed for {path}: {reason}")]
    VolumeUuidError { path: String, reason: String },

    #[error("Destination target already exists: {0:?}")]
    TargetExists(PathBuf),

    #[error("Destination file already exists: {0:?}")]
    AlreadyExists(PathBuf),

    #[error("Path not found: {0:?}")]
    NotFound(PathBuf),

    #[error("Permission denied: {0:?}")]
    PermissionDenied(PathBuf),

    #[error("Target volume is mounted read-only: {path}")]
    ReadOnlyFilesystem { path: String },

    #[error("Filesystem feature '{feature}' not supported on {path}")]
    NotSupported { feature: &'static str, path: String },

    #[error("Darwin system error (code {code}): {message}")]
    SystemError { code: i32, message: String },

    #[error("Symlink operation failed for {path}: {reason}")]
    SymlinkError { path: String, reason: String },
}
