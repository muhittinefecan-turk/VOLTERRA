use crate::classifier::ClassifiedError;
use crate::error::RecoveryError;
use crate::extent::ExtentMap;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuarantineState {
    Quarantined,
    Retrying,
    Deferred,
    Skipped,
    PartiallyRecovered,
    Resolved,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuarantinedFile {
    pub file_id: i64,
    pub job_id: String,
    pub relative_path: PathBuf,
    pub total_size_bytes: u64,
    pub failed_extents: ExtentMap,
    pub last_error: ClassifiedError,
    pub state: QuarantineState,
    pub attempts: u32,
    pub quarantined_at: u64,
}

pub struct ErrorQueue {
    files: RwLock<HashMap<i64, QuarantinedFile>>,
    order: RwLock<Vec<i64>>,
}

pub type QuarantineQueue = ErrorQueue;

impl Default for ErrorQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl ErrorQueue {
    pub fn new() -> Self {
        Self {
            files: RwLock::new(HashMap::new()),
            order: RwLock::new(Vec::new()),
        }
    }

    /// Isolates an unreadable or failing file into the quarantine queue.
    pub fn quarantine_file(&self, file: QuarantinedFile) -> Result<(), RecoveryError> {
        let file_id = file.file_id;
        let mut files = self.files.write();
        let mut order = self.order.write();

        if !files.contains_key(&file_id) {
            order.push(file_id);
        }
        files.insert(file_id, file);

        tracing::warn!(file_id, "File isolated into quarantine queue");
        Ok(())
    }

    /// Retrieves a quarantined file by ID.
    pub fn get_file(&self, file_id: i64) -> Option<QuarantinedFile> {
        self.files.read().get(&file_id).cloned()
    }

    /// Lists all files currently in active `Quarantined` state.
    pub fn list_quarantined(&self) -> Vec<QuarantinedFile> {
        let files = self.files.read();
        let order = self.order.read();

        order
            .iter()
            .filter_map(|id| files.get(id))
            .filter(|f| f.state == QuarantineState::Quarantined)
            .cloned()
            .collect()
    }

    /// Lists all files currently in `Deferred` state (second-pass pool).
    pub fn list_deferred(&self) -> Vec<QuarantinedFile> {
        let files = self.files.read();
        let order = self.order.read();

        order
            .iter()
            .filter_map(|id| files.get(id))
            .filter(|f| f.state == QuarantineState::Deferred)
            .cloned()
            .collect()
    }

    /// Lists all files across all quarantine states in original insertion order.
    pub fn list_all(&self) -> Vec<QuarantinedFile> {
        let files = self.files.read();
        let order = self.order.read();

        order
            .iter()
            .filter_map(|id| files.get(id))
            .cloned()
            .collect()
    }

    /// Updates the state of a quarantined file.
    pub fn update_state(&self, file_id: i64, state: QuarantineState) -> Result<(), RecoveryError> {
        let mut files = self.files.write();
        let file = files
            .get_mut(&file_id)
            .ok_or(RecoveryError::FileNotFound(file_id))?;
        file.state = state;
        Ok(())
    }

    /// Removes a file from the quarantine queue.
    pub fn remove(&self, file_id: i64) -> Option<QuarantinedFile> {
        let mut files = self.files.write();
        let mut order = self.order.write();
        order.retain(|&id| id != file_id);
        files.remove(&file_id)
    }

    /// Returns the total count of files in the queue.
    pub fn count(&self) -> usize {
        self.files.read().len()
    }

    /// Returns count of files in active `Quarantined` state.
    pub fn count_active_quarantine(&self) -> usize {
        self.files
            .read()
            .values()
            .filter(|f| f.state == QuarantineState::Quarantined)
            .count()
    }

    /// Returns true if the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.files.read().is_empty()
    }

    /// Clears the queue.
    pub fn clear(&self) {
        self.files.write().clear();
        self.order.write().clear();
    }
}
