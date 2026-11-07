use crate::state::JobLifecycleState;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum CoreError {
    #[error("Missing required parameter: {0}")]
    MissingParameter(&'static str),

    #[error("Source path does not exist: '{0}'")]
    SourceNotFound(PathBuf),

    #[error("Invalid state transition from {current:?} to {target:?}")]
    InvalidStateTransition {
        current: JobLifecycleState,
        target: JobLifecycleState,
    },

    #[error("Job already running")]
    AlreadyRunning,

    #[error("Job not in runnable state: {0:?}")]
    NotRunnable(JobLifecycleState),

    #[error("Recovery action failed: {0}")]
    RecoveryActionFailed(String),

    #[error("Manifest error: {0}")]
    ManifestFailed(#[from] volterra_manifest::ManifestError),

    #[error("Journal error: {0}")]
    JournalFailed(#[from] volterra_journal::JournalError),

    #[error("Transfer error: {0}")]
    TransferFailed(#[from] volterra_transfer::TransferError),

    #[error("Profiler error: {0}")]
    ProfilerFailed(#[from] volterra_profiler::ProfilerError),

    #[error("Internal core error: {0}")]
    Internal(String),
}
