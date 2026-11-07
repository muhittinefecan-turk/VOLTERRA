use crate::error::CoreError;
use crate::events::{EventHub, JobEvent};
use crate::snapshot::JobSnapshot;
use crate::state::JobLifecycleState;
use crossbeam_channel::Receiver;
use parking_lot::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use volterra_profiler::{PerformanceMonitor, TransferTelemetry};
use volterra_recovery::{InteractiveAction, RecoveryController};

pub struct JobHandle {
    pub(crate) job_id: String,
    pub(crate) state: Arc<RwLock<JobLifecycleState>>,
    pub(crate) paused: Arc<AtomicBool>,
    pub(crate) cancelled: Arc<AtomicBool>,
    pub(crate) event_hub: Arc<EventHub>,
    pub(crate) recovery: Arc<RecoveryController>,
    pub(crate) monitor: Arc<PerformanceMonitor>,
    pub(crate) snapshot_fn: Arc<dyn Fn() -> JobSnapshot + Send + Sync>,
    pub(crate) start_fn: Arc<dyn Fn() -> Result<(), CoreError> + Send + Sync>,
}

impl JobHandle {
    pub fn id(&self) -> &str {
        &self.job_id
    }

    pub fn state(&self) -> JobLifecycleState {
        *self.state.read()
    }

    pub fn start(&self) -> Result<(), CoreError> {
        (self.start_fn)()
    }

    pub fn pause(&self) -> Result<(), CoreError> {
        let mut state = self.state.write();
        if state.can_transition_to(JobLifecycleState::Paused) {
            let old = *state;
            *state = JobLifecycleState::Paused;
            self.paused.store(true, Ordering::SeqCst);
            self.event_hub.broadcast(JobEvent::StateChanged {
                old_state: old,
                new_state: JobLifecycleState::Paused,
            });
            Ok(())
        } else {
            Err(CoreError::InvalidStateTransition {
                current: *state,
                target: JobLifecycleState::Paused,
            })
        }
    }

    pub fn resume(&self) -> Result<(), CoreError> {
        let mut state = self.state.write();
        if *state != JobLifecycleState::Paused && *state != JobLifecycleState::WaitingForDisk {
            return Err(CoreError::InvalidStateTransition {
                current: *state,
                target: JobLifecycleState::Running,
            });
        }
        let old = *state;
        *state = JobLifecycleState::Running;
        self.paused.store(false, Ordering::SeqCst);
        self.event_hub.broadcast(JobEvent::StateChanged {
            old_state: old,
            new_state: JobLifecycleState::Running,
        });
        Ok(())
    }

    pub fn cancel(&self) -> Result<(), CoreError> {
        let mut state = self.state.write();
        if *state == JobLifecycleState::Cancelled {
            return Ok(());
        }
        if state.can_transition_to(JobLifecycleState::Cancelled) {
            let old = *state;
            *state = JobLifecycleState::Cancelled;
            self.cancelled.store(true, Ordering::SeqCst);
            self.event_hub.broadcast(JobEvent::StateChanged {
                old_state: old,
                new_state: JobLifecycleState::Cancelled,
            });
            Ok(())
        } else {
            Err(CoreError::InvalidStateTransition {
                current: *state,
                target: JobLifecycleState::Cancelled,
            })
        }
    }

    pub fn skip_file(&self, file_id: i64) -> Result<(), CoreError> {
        self.recovery
            .apply_action(InteractiveAction::SkipFile {
                file_id,
                reason: Some("User requested skip".to_string()),
            })
            .map_err(|e| CoreError::RecoveryActionFailed(e.to_string()))?;
        Ok(())
    }

    pub fn retry_file(&self, file_id: i64) -> Result<(), CoreError> {
        self.recovery
            .apply_action(InteractiveAction::RetryFile {
                file_id,
                reset_budget: true,
            })
            .map_err(|e| CoreError::RecoveryActionFailed(e.to_string()))?;
        Ok(())
    }

    pub fn skip_all_errors(&self) -> Result<(), CoreError> {
        self.recovery
            .apply_action(InteractiveAction::SkipAll)
            .map_err(|e| CoreError::RecoveryActionFailed(e.to_string()))?;
        Ok(())
    }

    pub fn retry_all_failed(&self) -> Result<(), CoreError> {
        self.recovery
            .apply_action(InteractiveAction::RetryAll)
            .map_err(|e| CoreError::RecoveryActionFailed(e.to_string()))?;
        Ok(())
    }

    pub fn snapshot(&self) -> JobSnapshot {
        (self.snapshot_fn)()
    }

    pub fn telemetry(&self) -> TransferTelemetry {
        self.monitor.get_telemetry()
    }

    pub fn subscribe_events(&self, capacity: usize) -> Receiver<JobEvent> {
        self.event_hub.subscribe(capacity)
    }

    pub fn recovery(&self) -> &Arc<RecoveryController> {
        &self.recovery
    }

    pub fn is_active(&self) -> bool {
        matches!(
            self.state(),
            JobLifecycleState::Running | JobLifecycleState::Scanning | JobLifecycleState::Paused
        )
    }
}

impl std::fmt::Debug for JobHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JobHandle")
            .field("job_id", &self.job_id)
            .field("state", &self.state())
            .finish()
    }
}
