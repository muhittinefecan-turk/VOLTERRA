use serde::{Deserialize, Serialize};

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TransferPolicy {
    #[default]
    Automatic,
    Performance,
    Balanced,
    SafeRecovery,
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct ErrorRecoveryState {
    pub consecutive_errors: u32,
    pub is_in_recovery: bool,
    pub fallback_triggered_at: Option<String>,
}

impl ErrorRecoveryState {
    pub fn record_error(&mut self) {
        self.consecutive_errors += 1;
        self.is_in_recovery = true;
        if self.fallback_triggered_at.is_none() {
            self.fallback_triggered_at = Some(chrono::Utc::now().to_rfc3339());
        }
    }

    pub fn reset_recovery(&mut self) {
        self.consecutive_errors = 0;
        self.is_in_recovery = false;
        self.fallback_triggered_at = None;
    }
}
