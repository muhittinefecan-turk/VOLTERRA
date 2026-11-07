use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ProfilerError {
    #[error("Platform error: {0}")]
    PlatformError(#[from] volterra_platform_macos::PlatformError),

    #[error("Disk discovery failed for path '{path}': {reason}")]
    DiscoveryFailed { path: PathBuf, reason: String },

    #[error("Path not found or inaccessible: '{0}'")]
    PathNotFound(PathBuf),

    #[error("Active benchmarking prohibited on failing or error-prone disk: {reason}")]
    FailingDiskBenchmarkProhibited { reason: String },

    #[error("Insufficient free space on '{path}' for safe benchmark (required: {required_bytes} bytes, free: {available_bytes} bytes)")]
    InsufficientSpaceForBenchmark {
        path: PathBuf,
        required_bytes: u64,
        available_bytes: u64,
    },

    #[error("Benchmark I/O error: {0}")]
    BenchmarkIoError(#[from] std::io::Error),

    #[error("Benchmark timed out after {seconds} seconds")]
    BenchmarkTimeout { seconds: u64 },

    #[error("System error: {0}")]
    SystemError(String),
}
