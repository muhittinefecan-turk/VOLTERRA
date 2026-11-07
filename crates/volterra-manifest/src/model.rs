use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ItemType {
    RegularFile,
    Directory,
    Symlink,
    Special, // FIFO, socket, device node
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileState {
    Pending,
    Scanning,
    Copying,
    Verifying,
    Completed,
    Paused,
    Skipped,
    ReadError,
    WriteError,
    Partial,
    NeedsAttention,
    WaitingForDisk,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestItem {
    pub id: i64,
    pub job_id: String,
    pub relative_path: PathBuf,
    pub item_type: ItemType,
    pub size_bytes: u64,
    pub source_size: u64,
    pub mtime_sec: i64,
    pub mtime_nsec: i64,
    pub source_mtime_ns: i64,
    pub btime_nsec: Option<i64>,
    pub source_btime_ns: Option<i64>,
    pub inode: u64,
    pub dev: u64,
    pub source_file_id: String, // "dev:ino"
    pub posix_mode: u32,
    pub has_xattrs: bool,
    pub symlink_target: Option<PathBuf>,
    pub temp_path: Option<PathBuf>,
    pub state: FileState,
    pub verified_digest: Option<String>,
    pub error_code: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkSpec {
    pub file_id: i64,
    pub chunk_index: u32,
    pub offset_bytes: u64,
    pub length_bytes: u32,
    pub digest: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub root: PathBuf,
    pub job_id: String,
    pub items: Vec<ManifestItem>,
    pub total_bytes: u64,
    pub file_count: u64,
    pub dir_count: u64,
    pub symlink_count: u64,
    pub special_count: u64,
}
