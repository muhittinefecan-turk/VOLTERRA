use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorDomain {
    Source,
    Destination,
    Integrity,
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorSeverity {
    Transient,    // Immediate retry candidate (e.g. EINTR, EBUSY)
    FileSpecific, // Quarantine file, proceed with healthy files (e.g. EIO, EACCES)
    JobFatal,     // Pause/abort transfer job immediately (e.g. ENOSPC, ENODEV)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorCategory {
    DeviceIo,            // EIO, hardware bad sector
    StorageFull,         // ENOSPC, disk full
    PermissionDenied,    // EACCES, EPERM
    DeviceDisconnected,  // ENODEV, ENXIO, ENOENT
    ReadOnlyFileSystem,  // EROFS
    ResourceBusy,        // EBUSY
    TimedOut,            // ETIMEDOUT
    Interrupted,         // EINTR
    ShortRead,           // Unexpected early EOF or truncated read
    IntegrityMismatch,   // BLAKE3 hash mismatch
    SourceMutated,       // File modified during read (size, mtime, inode)
    TargetCollision,     // Target exists without overwrite consent
    UnsupportedMetadata, // Darwin xattr or ACL unsupported on destination
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionRecommendation {
    RetryWithBackoff,
    QuarantineAndProceed,
    PauseJobWaitingForUser,
    CheckPermissions,
    MountDisk,
    SkipFile,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassifiedError {
    pub category: ErrorCategory,
    pub domain: ErrorDomain,
    pub severity: ErrorSeverity,
    pub errno_code: Option<i32>,
    pub message: String,
    pub is_retryable: bool,
    pub suggested_action: ActionRecommendation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecoveryErrorClass {
    ReadEio,
    WriteEnospc,
    PermissionDenied,
    DeviceDisconnected,
    ShortRead,
    SourceMutated,
    TargetCollision,
    IntegrityMismatch,
    UnsupportedMetadata,
    Unknown(i32),
}

impl RecoveryErrorClass {
    pub fn classify_io_error(err: &std::io::Error) -> Self {
        if let Some(raw_errno) = err.raw_os_error() {
            match raw_errno {
                libc::EIO => Self::ReadEio,
                libc::ENOSPC => Self::WriteEnospc,
                libc::EACCES | libc::EPERM => Self::PermissionDenied,
                libc::ENODEV | libc::ENOENT => Self::DeviceDisconnected,
                _ => Self::Unknown(raw_errno),
            }
        } else {
            match err.kind() {
                std::io::ErrorKind::UnexpectedEof => Self::ShortRead,
                std::io::ErrorKind::PermissionDenied => Self::PermissionDenied,
                _ => Self::Unknown(-1),
            }
        }
    }
}

pub struct RecoveryClassifier;

impl RecoveryClassifier {
    /// Classifies a standard std::io::Error into structured domain and severity taxonomy.
    pub fn classify_io_error(err: &std::io::Error, domain: ErrorDomain) -> ClassifiedError {
        if let Some(raw_errno) = err.raw_os_error() {
            Self::classify_errno(raw_errno, domain, &err.to_string())
        } else {
            match err.kind() {
                std::io::ErrorKind::UnexpectedEof => ClassifiedError {
                    category: ErrorCategory::ShortRead,
                    domain,
                    severity: ErrorSeverity::FileSpecific,
                    errno_code: None,
                    message: format!("Short read: {}", err),
                    is_retryable: true,
                    suggested_action: ActionRecommendation::RetryWithBackoff,
                },
                std::io::ErrorKind::PermissionDenied => ClassifiedError {
                    category: ErrorCategory::PermissionDenied,
                    domain,
                    severity: ErrorSeverity::FileSpecific,
                    errno_code: Some(libc::EACCES),
                    message: format!("Permission denied: {}", err),
                    is_retryable: false,
                    suggested_action: ActionRecommendation::CheckPermissions,
                },
                _ => ClassifiedError {
                    category: ErrorCategory::Other,
                    domain,
                    severity: ErrorSeverity::FileSpecific,
                    errno_code: None,
                    message: err.to_string(),
                    is_retryable: false,
                    suggested_action: ActionRecommendation::QuarantineAndProceed,
                },
            }
        }
    }

    /// Classifies a raw POSIX errno.
    pub fn classify_errno(
        raw_errno: i32,
        domain: ErrorDomain,
        context_msg: &str,
    ) -> ClassifiedError {
        match raw_errno {
            libc::EIO => ClassifiedError {
                category: ErrorCategory::DeviceIo,
                domain,
                severity: ErrorSeverity::FileSpecific,
                errno_code: Some(raw_errno),
                message: if context_msg.is_empty() {
                    "Hardware I/O bad sector encountered".to_string()
                } else {
                    format!("Hardware I/O error (EIO): {}", context_msg)
                },
                is_retryable: true,
                suggested_action: ActionRecommendation::QuarantineAndProceed,
            },
            libc::ENOSPC => ClassifiedError {
                category: ErrorCategory::StorageFull,
                domain: ErrorDomain::Destination,
                severity: ErrorSeverity::JobFatal,
                errno_code: Some(raw_errno),
                message: "Target storage device is completely full".to_string(),
                is_retryable: false,
                suggested_action: ActionRecommendation::PauseJobWaitingForUser,
            },
            libc::EACCES | libc::EPERM => ClassifiedError {
                category: ErrorCategory::PermissionDenied,
                domain,
                severity: ErrorSeverity::FileSpecific,
                errno_code: Some(raw_errno),
                message: if context_msg.is_empty() {
                    "File permission or sandbox denial (EACCES/EPERM)".to_string()
                } else {
                    format!("Permission denied: {}", context_msg)
                },
                is_retryable: false,
                suggested_action: ActionRecommendation::CheckPermissions,
            },
            libc::ENODEV | libc::ENXIO => ClassifiedError {
                category: ErrorCategory::DeviceDisconnected,
                domain,
                severity: ErrorSeverity::JobFatal,
                errno_code: Some(raw_errno),
                message: "Underlying storage drive was unexpectedly unmounted or disconnected"
                    .to_string(),
                is_retryable: false,
                suggested_action: ActionRecommendation::MountDisk,
            },
            libc::EROFS => ClassifiedError {
                category: ErrorCategory::ReadOnlyFileSystem,
                domain: ErrorDomain::Destination,
                severity: ErrorSeverity::JobFatal,
                errno_code: Some(raw_errno),
                message: "Destination filesystem is mounted read-only".to_string(),
                is_retryable: false,
                suggested_action: ActionRecommendation::PauseJobWaitingForUser,
            },
            libc::EBUSY => ClassifiedError {
                category: ErrorCategory::ResourceBusy,
                domain,
                severity: ErrorSeverity::Transient,
                errno_code: Some(raw_errno),
                message: "Resource busy; temporarily locked by another process".to_string(),
                is_retryable: true,
                suggested_action: ActionRecommendation::RetryWithBackoff,
            },
            libc::EINTR => ClassifiedError {
                category: ErrorCategory::Interrupted,
                domain,
                severity: ErrorSeverity::Transient,
                errno_code: Some(raw_errno),
                message: "System call was interrupted by signal".to_string(),
                is_retryable: true,
                suggested_action: ActionRecommendation::RetryWithBackoff,
            },
            libc::ETIMEDOUT => ClassifiedError {
                category: ErrorCategory::TimedOut,
                domain,
                severity: ErrorSeverity::Transient,
                errno_code: Some(raw_errno),
                message: "Disk or bus controller operation timed out".to_string(),
                is_retryable: true,
                suggested_action: ActionRecommendation::RetryWithBackoff,
            },
            _ => ClassifiedError {
                category: ErrorCategory::Other,
                domain,
                severity: ErrorSeverity::FileSpecific,
                errno_code: Some(raw_errno),
                message: format!("OS error {}: {}", raw_errno, context_msg),
                is_retryable: false,
                suggested_action: ActionRecommendation::QuarantineAndProceed,
            },
        }
    }

    /// Classifies an unexpected short read.
    pub fn classify_short_read(offset: u64, expected: usize, actual: usize) -> ClassifiedError {
        ClassifiedError {
            category: ErrorCategory::ShortRead,
            domain: ErrorDomain::Source,
            severity: ErrorSeverity::FileSpecific,
            errno_code: None,
            message: format!(
                "Unexpected EOF at offset {}: requested {} bytes, received {} bytes",
                offset, expected, actual
            ),
            is_retryable: true,
            suggested_action: ActionRecommendation::RetryWithBackoff,
        }
    }

    /// Classifies a cryptographic digest mismatch.
    pub fn classify_integrity_mismatch(
        offset: u64,
        expected: &str,
        actual: &str,
    ) -> ClassifiedError {
        ClassifiedError {
            category: ErrorCategory::IntegrityMismatch,
            domain: ErrorDomain::Integrity,
            severity: ErrorSeverity::FileSpecific,
            errno_code: None,
            message: format!(
                "BLAKE3 checksum mismatch at offset {}: expected {}, computed {}",
                offset, expected, actual
            ),
            is_retryable: true,
            suggested_action: ActionRecommendation::RetryWithBackoff,
        }
    }

    /// Classifies a source mutation detected during transfer.
    pub fn classify_source_mutated(path: &Path, reason: &str) -> ClassifiedError {
        ClassifiedError {
            category: ErrorCategory::SourceMutated,
            domain: ErrorDomain::Source,
            severity: ErrorSeverity::FileSpecific,
            errno_code: None,
            message: format!(
                "Source file {} was concurrently modified during read: {}",
                path.display(),
                reason
            ),
            is_retryable: true,
            suggested_action: ActionRecommendation::RetryWithBackoff,
        }
    }
}
