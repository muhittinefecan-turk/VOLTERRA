use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum JobLifecycleState {
    Pending,
    Scanning,
    Running,
    Paused,
    Completed,
    Failed,
    WaitingForDisk,
    Cancelled,
}

impl JobLifecycleState {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }

    pub fn can_transition_to(&self, next: JobLifecycleState) -> bool {
        use JobLifecycleState::*;
        match (self, next) {
            // Terminal states cannot transition to anything
            (Completed, _) | (Failed, _) | (Cancelled, _) => false,
            // Self transitions are idempotent
            (a, b) if *a == b => true,
            // Pending can start scanning or be cancelled
            (Pending, Scanning) | (Pending, Cancelled) => true,
            // Scanning can run, fail, or be cancelled
            (Scanning, Running) | (Scanning, Failed) | (Scanning, Cancelled) => true,
            // Running can pause, complete, fail, wait for disk, or cancel
            (Running, Paused)
            | (Running, Completed)
            | (Running, Failed)
            | (Running, WaitingForDisk)
            | (Running, Cancelled) => true,
            // Paused can resume running or be cancelled
            (Paused, Running) | (Paused, Cancelled) => true,
            // WaitingForDisk can resume running or be cancelled
            (WaitingForDisk, Running) | (WaitingForDisk, Cancelled) => true,
            _ => false,
        }
    }
}
