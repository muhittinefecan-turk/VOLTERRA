use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("Source root does not exist: {0:?}")]
    RootNotFound(PathBuf),

    #[error("Permission denied scanning {0:?}")]
    PermissionDenied(PathBuf),

    #[error("Source file deleted prior to publication: {path:?}")]
    SourceDeleted { path: PathBuf },

    #[error("Source file mutated during transfer ({path:?}): {reason}")]
    SourceMutated { path: PathBuf, reason: String },

    #[error("Filesystem loop detected at {0:?}")]
    SymlinkLoop(PathBuf),

    #[error("Filesystem loop detected at {path:?}")]
    CycleDetected { path: PathBuf },

    #[error("I/O error at {path:?}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("I/O error: {0}")]
    IoDirect(#[from] std::io::Error),

    #[error("Integrity error: {0}")]
    Integrity(#[from] volterra_integrity::error::IntegrityError),
}
