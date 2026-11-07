use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferProgress {
    pub file_id: i64,
    pub chunk_index: u32,
    pub bytes_copied: u64,
    pub total_bytes: u64,
    pub elapsed_ms: u64,
}
