use thiserror::Error;

#[derive(Debug, Error)]
pub enum RecoveryError {
    #[error("File not found in recovery manager: id {0}")]
    FileNotFound(i64),

    #[error("Action not permitted in state {current:?}")]
    InvalidStateTransition {
        current: volterra_manifest::FileState,
    },

    #[error("Retry budget exhausted: {attempts} attempts exceeded limit {max_retries}")]
    RetryBudgetExhausted { attempts: u32, max_retries: u32 },

    #[error("Invalid extent interval: [{offset}, {}) - {reason}", .offset.saturating_add(*.length))]
    InvalidExtent {
        offset: u64,
        length: u64,
        reason: String,
    },

    #[error("Fail-closed barrier check failed: file incomplete ({verified_bytes}/{total_size} bytes verified, {missing_extents} missing extents)")]
    FailClosedBarrierFailed {
        total_size: u64,
        verified_bytes: u64,
        missing_extents: usize,
    },

    #[error("I/O error during recovery: {0}")]
    Io(#[from] std::io::Error),

    #[error("Journal storage error: {0}")]
    Journal(String),

    #[error("JSON serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}
