use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub use volterra_manifest::FileState;
pub type FileStatus = FileState;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobMode {
    Turbo,
    SmartHybrid,
    SafeRecovery,
}

impl JobMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Turbo => "Turbo",
            Self::SmartHybrid => "SmartHybrid",
            Self::SafeRecovery => "SafeRecovery",
        }
    }
}

impl std::str::FromStr for JobMode {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Turbo" => Ok(Self::Turbo),
            "SmartHybrid" => Ok(Self::SmartHybrid),
            "SafeRecovery" => Ok(Self::SafeRecovery),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobState {
    Pending,
    Scanning,
    Running,
    Paused,
    Completed,
    Failed,
    WaitingForDisk,
    Cancelled,
}

pub type JobStatus = JobState;

impl JobState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "Pending",
            Self::Scanning => "Scanning",
            Self::Running => "Running",
            Self::Paused => "Paused",
            Self::Completed => "Completed",
            Self::Failed => "Failed",
            Self::WaitingForDisk => "WaitingForDisk",
            Self::Cancelled => "Cancelled",
        }
    }
}

impl std::str::FromStr for JobState {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Pending" => Ok(Self::Pending),
            "Scanning" => Ok(Self::Scanning),
            "Running" => Ok(Self::Running),
            "Paused" => Ok(Self::Paused),
            "Completed" => Ok(Self::Completed),
            "Failed" => Ok(Self::Failed),
            "WaitingForDisk" => Ok(Self::WaitingForDisk),
            "Cancelled" => Ok(Self::Cancelled),
            _ => Err(()),
        }
    }
}

pub fn file_state_as_str(state: &FileState) -> &'static str {
    match state {
        FileState::Pending => "Pending",
        FileState::Scanning => "Scanning",
        FileState::Copying => "Copying",
        FileState::Verifying => "Verifying",
        FileState::Completed => "Completed",
        FileState::Paused => "Paused",
        FileState::Skipped => "Skipped",
        FileState::ReadError => "ReadError",
        FileState::WriteError => "WriteError",
        FileState::Partial => "Partial",
        FileState::NeedsAttention => "NeedsAttention",
        FileState::WaitingForDisk => "WaitingForDisk",
        FileState::Cancelled => "Cancelled",
    }
}

pub fn file_state_from_str(s: &str) -> Option<FileState> {
    match s {
        "Pending" => Some(FileState::Pending),
        "Scanning" => Some(FileState::Scanning),
        "Copying" => Some(FileState::Copying),
        "Verifying" => Some(FileState::Verifying),
        "Completed" => Some(FileState::Completed),
        "Paused" => Some(FileState::Paused),
        "Skipped" => Some(FileState::Skipped),
        "ReadError" => Some(FileState::ReadError),
        "WriteError" => Some(FileState::WriteError),
        "Partial" => Some(FileState::Partial),
        "NeedsAttention" => Some(FileState::NeedsAttention),
        "WaitingForDisk" => Some(FileState::WaitingForDisk),
        "Cancelled" => Some(FileState::Cancelled),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChunkState {
    Pending,
    Copying,
    Verified,
    Failed,
}

impl ChunkState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "Pending",
            Self::Copying => "Copying",
            Self::Verified => "Verified",
            Self::Failed => "Failed",
        }
    }
}

impl std::str::FromStr for ChunkState {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Pending" => Ok(Self::Pending),
            "Copying" => Ok(Self::Copying),
            "Verified" => Ok(Self::Verified),
            "Failed" => Ok(Self::Failed),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobRecord {
    pub id: String,
    pub source_root: PathBuf,
    pub target_root: PathBuf,
    pub source_volume_uuid: Option<String>,
    pub target_volume_uuid: Option<String>,
    pub mode: JobMode,
    pub state: JobState,
    pub total_files: u64,
    pub total_bytes: u64,
    pub copied_bytes: u64,
    pub created_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRecord {
    pub id: i64,
    pub job_id: String,
    pub relative_path: PathBuf,
    pub source_size: u64,
    pub source_mtime_ns: i64,
    pub source_btime_ns: Option<i64>,
    pub posix_mode: u32,
    pub source_file_id: String,
    pub temp_path: Option<PathBuf>,
    pub state: FileState,
    pub verified_digest: Option<String>,
    pub error_code: Option<i32>,
    pub recovered_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkRecord {
    pub file_id: i64,
    pub chunk_index: u32,
    pub offset_bytes: u64,
    pub length_bytes: u32,
    pub digest: Option<String>,
    pub state: ChunkState,
    pub attempts: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtentErrorRecord {
    pub id: Option<i64>,
    pub job_id: String,
    pub file_id: Option<i64>,
    pub offset_bytes: u64,
    pub length_bytes: u64,
    pub phase: String,
    pub errno_code: i32,
    pub error_type: String,
    pub attempts: u32,
    pub message: String,
    pub action_taken: Option<String>,
    pub recorded_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobRecoveryState {
    pub job: JobRecord,
    pub interrupted_files: Vec<FileRecord>,
    pub completed_file_count: u64,
    pub failed_file_count: u64,
    pub pending_file_count: u64,
    pub total_copied_bytes: u64,
    pub open_errors: Vec<ExtentErrorRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckpointMode {
    Passive,
    Full,
    Restart,
    Truncate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReconciliationReport {
    pub job_id: String,
    pub reconciled_files_count: usize,
    pub reverted_chunks_count: usize,
    pub verified_chunks_count: usize,
    pub recovered_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileResumptionPlan {
    pub file_id: i64,
    pub relative_path: PathBuf,
    pub total_size: u64,
    pub verified_extents: Vec<(u64, u64)>, // (offset, length)
    pub missing_extents: Vec<(u64, u64)>,  // (offset, length)
    pub already_verified_bytes: u64,
    pub remaining_bytes: u64,
}
