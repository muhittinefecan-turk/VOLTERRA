use crate::classifier::ClassifiedError;
use crate::error::RecoveryError;
use std::collections::HashMap;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub max_retries_per_file: u32,
    pub max_job_retries: u32,
    pub base_delay_ms: u64,
    pub max_delay_ms: u64,
    pub backoff_multiplier: f64,
    pub jitter_factor: f64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries_per_file: 3,
            max_job_retries: 50,
            base_delay_ms: 100,
            max_delay_ms: 5000,
            backoff_multiplier: 2.0,
            jitter_factor: 0.2,
        }
    }
}

pub struct RetryBudget {
    pub max_retries: u32,
    pub current_attempts: u32,
    pub base_delay_ms: u64,
    pub max_delay_ms: u64,
    pub jitter_factor: f64,
    pub policy: RetryPolicy,
    file_attempts: HashMap<i64, u32>,
    total_job_attempts: u32,
    prng_state: u64,
}

impl Default for RetryBudget {
    fn default() -> Self {
        let policy = RetryPolicy::default();
        Self {
            max_retries: policy.max_retries_per_file,
            current_attempts: 0,
            base_delay_ms: policy.base_delay_ms,
            max_delay_ms: policy.max_delay_ms,
            jitter_factor: policy.jitter_factor,
            policy,
            file_attempts: HashMap::new(),
            total_job_attempts: 0,
            prng_state: 0x853c49e6748fea9b,
        }
    }
}

impl RetryBudget {
    pub fn new(policy: RetryPolicy) -> Self {
        Self {
            max_retries: policy.max_retries_per_file,
            current_attempts: 0,
            base_delay_ms: policy.base_delay_ms,
            max_delay_ms: policy.max_delay_ms,
            jitter_factor: policy.jitter_factor,
            policy,
            file_attempts: HashMap::new(),
            total_job_attempts: 0,
            prng_state: 0x853c49e6748fea9b,
        }
    }

    /// Checks if a generic retry is permitted under max_retries.
    pub fn can_retry(&self) -> bool {
        self.current_attempts < self.max_retries
    }

    /// Checks if a retry is permitted for a specific file and error.
    pub fn can_retry_file(&self, file_id: i64, error: &ClassifiedError) -> bool {
        if !error.is_retryable {
            return false;
        }

        if self.total_job_attempts >= self.policy.max_job_retries {
            return false;
        }

        let attempts = self.file_attempts.get(&file_id).copied().unwrap_or(0);
        attempts < self.policy.max_retries_per_file
    }

    /// Records an attempt and computes exponential backoff delay with jitter.
    /// Returns Err(()) if retry budget is exhausted.
    #[allow(clippy::result_unit_err)]
    pub fn record_attempt(&mut self) -> Result<Duration, ()> {
        if !self.can_retry() {
            return Err(());
        }

        self.current_attempts += 1;
        self.total_job_attempts = self.total_job_attempts.saturating_add(1);

        let exp = 2u64.saturating_pow(self.current_attempts.saturating_sub(1));
        let raw_delay = self
            .base_delay_ms
            .saturating_mul(exp)
            .min(self.max_delay_ms);

        let jitter_span = (raw_delay as f64 * self.jitter_factor) as u64;
        let jitter = if jitter_span > 0 {
            self.next_prng() % jitter_span
        } else {
            0
        };

        let delay_ms = raw_delay.saturating_add(jitter / 2);
        Ok(Duration::from_millis(delay_ms))
    }

    /// Records an attempt for a specific file and calculates jittered exponential backoff.
    pub fn record_file_attempt(&mut self, file_id: i64) -> Result<Duration, RecoveryError> {
        let attempts = self.file_attempts.entry(file_id).or_insert(0);
        if *attempts >= self.policy.max_retries_per_file {
            return Err(RecoveryError::RetryBudgetExhausted {
                attempts: *attempts,
                max_retries: self.policy.max_retries_per_file,
            });
        }

        *attempts += 1;
        self.total_job_attempts = self.total_job_attempts.saturating_add(1);

        let exp = 2u64.saturating_pow(attempts.saturating_sub(1));
        let raw_delay = self
            .policy
            .base_delay_ms
            .saturating_mul(exp)
            .min(self.policy.max_delay_ms);

        let jitter_span = (raw_delay as f64 * self.policy.jitter_factor) as u64;
        let jitter = if jitter_span > 0 {
            self.next_prng() % jitter_span
        } else {
            0
        };

        let delay_ms = raw_delay.saturating_add(jitter / 2);
        Ok(Duration::from_millis(delay_ms))
    }

    /// Resets global attempts counter.
    pub fn reset(&mut self) {
        self.current_attempts = 0;
    }

    /// Resets attempts counter for a specific file.
    pub fn reset_file(&mut self, file_id: i64) {
        self.file_attempts.remove(&file_id);
    }

    /// Returns remaining global job retries.
    pub fn remaining_job_retries(&self) -> u32 {
        self.policy
            .max_job_retries
            .saturating_sub(self.total_job_attempts)
    }

    /// Deterministic xorshift64 PRNG to avoid external PRNG crates.
    fn next_prng(&mut self) -> u64 {
        let mut x = self.prng_state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.prng_state = x;
        x
    }
}
