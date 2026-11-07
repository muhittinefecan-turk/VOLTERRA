use crate::budget::RetryBudget;
use crate::error::RecoveryError;
use crate::queue::{ErrorQueue, QuarantineState, QuarantinedFile};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use volterra_journal::JournalStorage;
use volterra_manifest::FileState;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum InteractiveAction {
    SkipFile {
        file_id: i64,
        reason: Option<String>,
    },
    RetryFile {
        file_id: i64,
        #[serde(default = "default_true")]
        reset_budget: bool,
    },
    DeferFile {
        file_id: i64,
    },
    SkipAll,
    RetryAll,
}

fn default_true() -> bool {
    true
}

pub struct RecoveryController {
    queue: Arc<ErrorQueue>,
    budget: Mutex<RetryBudget>,
    journal: Option<Arc<dyn JournalStorage>>,
}

impl RecoveryController {
    pub fn new(queue: Arc<ErrorQueue>, budget: RetryBudget) -> Self {
        Self {
            queue,
            budget: Mutex::new(budget),
            journal: None,
        }
    }

    pub fn with_journal(
        queue: Arc<ErrorQueue>,
        budget: RetryBudget,
        journal: Arc<dyn JournalStorage>,
    ) -> Self {
        Self {
            queue,
            budget: Mutex::new(budget),
            journal: Some(journal),
        }
    }

    pub fn queue(&self) -> &Arc<ErrorQueue> {
        &self.queue
    }

    /// Applies an interactive user action, orchestrating state machine transitions,
    /// budget resets, and durable journal synchronization.
    pub fn apply_action(
        &self,
        action: InteractiveAction,
    ) -> Result<Vec<QuarantinedFile>, RecoveryError> {
        match action {
            InteractiveAction::SkipFile { file_id, reason } => {
                let mut file = self
                    .queue
                    .get_file(file_id)
                    .ok_or(RecoveryError::FileNotFound(file_id))?;

                self.queue.update_state(file_id, QuarantineState::Skipped)?;
                file.state = QuarantineState::Skipped;

                if let Some(journal) = &self.journal {
                    let _ = journal.update_file_status(file_id, FileState::Skipped);
                }

                tracing::info!(
                    file_id,
                    reason = reason.as_deref().unwrap_or("User skipped file"),
                    "File skipped via interactive action"
                );

                Ok(vec![file])
            }

            InteractiveAction::RetryFile {
                file_id,
                reset_budget,
            } => {
                let mut file = self
                    .queue
                    .get_file(file_id)
                    .ok_or(RecoveryError::FileNotFound(file_id))?;

                if reset_budget {
                    let mut budget = self.budget.lock();
                    budget.reset_file(file_id);
                    budget.reset();
                }

                self.queue
                    .update_state(file_id, QuarantineState::Retrying)?;
                file.state = QuarantineState::Retrying;

                if let Some(journal) = &self.journal {
                    let _ = journal.update_file_status(file_id, FileState::Pending);
                }

                tracing::info!(file_id, "File re-queued for retry via interactive action");
                Ok(vec![file])
            }

            InteractiveAction::DeferFile { file_id } => {
                let mut file = self
                    .queue
                    .get_file(file_id)
                    .ok_or(RecoveryError::FileNotFound(file_id))?;

                self.queue
                    .update_state(file_id, QuarantineState::Deferred)?;
                file.state = QuarantineState::Deferred;

                if let Some(journal) = &self.journal {
                    let _ = journal.update_file_status(file_id, FileState::NeedsAttention);
                }

                tracing::info!(file_id, "File deferred for second-pass recovery");
                Ok(vec![file])
            }

            InteractiveAction::SkipAll => {
                let files = self.queue.list_all();
                let mut skipped = Vec::new();

                for mut file in files {
                    if file.state == QuarantineState::Quarantined
                        || file.state == QuarantineState::Deferred
                    {
                        self.queue
                            .update_state(file.file_id, QuarantineState::Skipped)?;
                        file.state = QuarantineState::Skipped;

                        if let Some(journal) = &self.journal {
                            let _ = journal.update_file_status(file.file_id, FileState::Skipped);
                        }

                        skipped.push(file);
                    }
                }

                tracing::info!(count = skipped.len(), "Bulk SkipAll executed");
                Ok(skipped)
            }

            InteractiveAction::RetryAll => {
                let files = self.queue.list_all();
                let mut retrying = Vec::new();

                {
                    let mut budget = self.budget.lock();
                    for file in &files {
                        budget.reset_file(file.file_id);
                    }
                    budget.reset();
                }

                for mut file in files {
                    if file.state == QuarantineState::Quarantined
                        || file.state == QuarantineState::Deferred
                    {
                        self.queue
                            .update_state(file.file_id, QuarantineState::Retrying)?;
                        file.state = QuarantineState::Retrying;

                        if let Some(journal) = &self.journal {
                            let _ = journal.update_file_status(file.file_id, FileState::Pending);
                        }

                        retrying.push(file);
                    }
                }

                tracing::info!(count = retrying.len(), "Bulk RetryAll executed");
                Ok(retrying)
            }
        }
    }
}
