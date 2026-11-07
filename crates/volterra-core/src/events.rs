use crate::snapshot::JobSnapshot;
use crate::state::JobLifecycleState;
use crossbeam_channel::{Receiver, Sender};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum JobEvent {
    StateChanged {
        old_state: JobLifecycleState,
        new_state: JobLifecycleState,
    },
    ScanningProgress {
        files_found: u64,
        bytes_found: u64,
    },
    ScanningComplete {
        total_files: u64,
        total_bytes: u64,
    },
    FileStarted {
        file_id: i64,
        path: PathBuf,
        size_bytes: u64,
    },
    ChunkProgress {
        file_id: i64,
        chunk_index: u32,
        bytes_copied: u64,
        total_bytes: u64,
    },
    FileCompleted {
        file_id: i64,
        digest: String,
        duration_ms: u64,
    },
    FileQuarantined {
        file_id: i64,
        error: String,
    },
    TelemetryUpdated {
        snapshot: Box<JobSnapshot>,
    },
    BottleneckDetected {
        bottleneck: String,
        confidence: f32,
    },
    DiskWarning {
        message: String,
    },
    JobCompleted {
        total_files: u64,
        total_bytes: u64,
        duration_ms: u64,
    },
    JobFailed {
        error: String,
    },
}

pub struct EventHub {
    subscribers: Arc<RwLock<Vec<Sender<JobEvent>>>>,
}

impl Default for EventHub {
    fn default() -> Self {
        Self::new()
    }
}

impl EventHub {
    pub fn new() -> Self {
        Self {
            subscribers: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Subscribes to the broadcast channel with a bounded capacity.
    pub fn subscribe(&self, capacity: usize) -> Receiver<JobEvent> {
        let (tx, rx) = crossbeam_channel::bounded(capacity.max(16));
        self.subscribers.write().push(tx);
        rx
    }

    /// Broadcasts an event to all subscribers without blocking the caller.
    /// Drops oldest message if a subscriber is full, and prunes disconnected receivers.
    pub fn broadcast(&self, event: JobEvent) {
        let mut subs = self.subscribers.write();
        subs.retain(|tx| match tx.try_send(event.clone()) {
            Ok(()) => true,
            Err(crossbeam_channel::TrySendError::Full(_)) => true, // slow subscriber, keep alive
            Err(crossbeam_channel::TrySendError::Disconnected(_)) => false, // receiver dropped
        });
    }

    pub fn subscriber_count(&self) -> usize {
        self.subscribers.read().len()
    }
}
