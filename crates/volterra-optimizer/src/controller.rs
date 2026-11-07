use crate::bottleneck::{BottleneckClassification, BottleneckDetector};
use crate::chunk_engine::ChunkSize;
use crate::policy::{ErrorRecoveryState, TransferPolicy};
use crate::scheduler::{IoScheduler, SchedulerProfile};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use volterra_profiler::{DiskRelationship, TransferTelemetry};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ControllerPhase {
    Warmup,
    Settled,
    Probing,
    Evaluating,
    Cooldown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TunableParameter {
    ChunkSize,
    Concurrency,
    QueueDepth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProbeDirection {
    Increase,
    Decrease,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizationDecision {
    pub chunk_size_bytes: usize,
    pub queue_depth: usize,
    pub concurrency: usize,
    pub read_ahead_mb: usize,
    pub phase: ControllerPhase,
    pub action_description: String,
}

pub struct TransferOptimizer {
    policy: TransferPolicy,
    relationship: DiskRelationship,
    scheduler_profile: SchedulerProfile,
    memory_budget_bytes: usize,

    // Closed-loop controller state
    phase: ControllerPhase,
    samples_count: usize,
    current_chunk_size: ChunkSize,
    current_concurrency: usize,
    current_queue_depth: usize,
    current_read_ahead_mb: usize,

    // Probe tracking
    baseline_throughput_mbps: f64,
    baseline_p95_write_ms: f64,
    probing_parameter: Option<TunableParameter>,
    probing_direction: Option<ProbeDirection>,
    eval_steps_remaining: usize,
    cooldown_steps_remaining: usize,

    // Flip-flop anti-oscillation tracker (counts consecutive reversions per parameter)
    reversion_counts: HashMap<TunableParameter, usize>,
    quarantined_params: HashMap<TunableParameter, usize>,

    recovery_state: ErrorRecoveryState,
}

impl TransferOptimizer {
    pub fn new(
        relationship: DiskRelationship,
        memory_budget_bytes: usize,
        policy: TransferPolicy,
    ) -> Self {
        let sched = IoScheduler::create_profile(&relationship, memory_budget_bytes);
        let initial_chunk = sched.initial_chunk_size;
        let initial_conc = sched.reader_concurrency.max(sched.writer_concurrency);
        let initial_qd = sched.queue_depth;
        let initial_ra = sched.read_ahead_mb;

        Self {
            policy,
            relationship,
            scheduler_profile: sched,
            memory_budget_bytes,

            phase: ControllerPhase::Warmup,
            samples_count: 0,
            current_chunk_size: initial_chunk,
            current_concurrency: initial_conc,
            current_queue_depth: initial_qd,
            current_read_ahead_mb: initial_ra,

            baseline_throughput_mbps: 0.0,
            baseline_p95_write_ms: 0.0,
            probing_parameter: None,
            probing_direction: None,
            eval_steps_remaining: 0,
            cooldown_steps_remaining: 0,

            reversion_counts: HashMap::new(),
            quarantined_params: HashMap::new(),

            recovery_state: ErrorRecoveryState::default(),
        }
    }

    pub fn set_policy(&mut self, policy: TransferPolicy) {
        self.policy = policy;
        if policy == TransferPolicy::SafeRecovery {
            self.current_chunk_size = ChunkSize::MiB1;
            self.current_concurrency = 1;
            self.current_queue_depth = 1;
            self.current_read_ahead_mb = 0;
            self.phase = ControllerPhase::Settled;
        }
    }

    pub fn detect_bottleneck(&self, telemetry: &TransferTelemetry) -> BottleneckClassification {
        BottleneckDetector::detect_bottleneck(telemetry)
    }

    /// Evaluates real-time telemetry and returns the next optimization decision
    /// enforcing 5% hysteresis, flip-flop lockout prevention, and error fallbacks.
    pub fn evaluate_step(&mut self, telemetry: &TransferTelemetry) -> OptimizationDecision {
        self.samples_count += 1;

        // Decrement quarantine counters
        for remaining in self.quarantined_params.values_mut() {
            if *remaining > 0 {
                *remaining -= 1;
            }
        }
        self.quarantined_params.retain(|_, count| *count > 0);

        // 1. Error Fallback (Highest Priority)
        if telemetry.read_errors > 0
            || telemetry.write_errors > 0
            || self.policy == TransferPolicy::SafeRecovery
        {
            self.recovery_state.record_error();
            self.current_chunk_size = ChunkSize::MiB1;
            self.current_concurrency = 1;
            self.current_queue_depth = 1;
            self.current_read_ahead_mb = 0;
            self.phase = ControllerPhase::Settled;

            return OptimizationDecision {
                chunk_size_bytes: self.current_chunk_size.as_bytes(),
                queue_depth: self.current_queue_depth,
                concurrency: self.current_concurrency,
                read_ahead_mb: self.current_read_ahead_mb,
                phase: ControllerPhase::Settled,
                action_description:
                    "Error detected: Clamped to 1 MiB chunk and 1 worker for safe sector recovery."
                        .into(),
            };
        }

        // 2. Policy-specific static constraints
        if self.policy == TransferPolicy::Balanced {
            return OptimizationDecision {
                chunk_size_bytes: ChunkSize::MiB8.as_bytes(),
                queue_depth: 2,
                concurrency: 1,
                read_ahead_mb: 8,
                phase: ControllerPhase::Settled,
                action_description: "Balanced policy active: Fixed 8 MiB chunk and single worker."
                    .into(),
            };
        }

        // 3. Controller Phase Machine
        match self.phase {
            ControllerPhase::Warmup => {
                if self.samples_count >= 4 && telemetry.ewma_5s_mbps > 1.0 {
                    self.phase = ControllerPhase::Settled;
                    self.baseline_throughput_mbps = telemetry.ewma_5s_mbps;
                    self.baseline_p95_write_ms = telemetry.p95_write_latency_ms;
                }
                OptimizationDecision {
                    chunk_size_bytes: self.current_chunk_size.as_bytes(),
                    queue_depth: self.current_queue_depth,
                    concurrency: self.current_concurrency,
                    read_ahead_mb: self.current_read_ahead_mb,
                    phase: ControllerPhase::Warmup,
                    action_description: "Warmup phase: Gathering baseline throughput data.".into(),
                }
            }
            ControllerPhase::Settled => {
                self.baseline_throughput_mbps = telemetry.ewma_5s_mbps;
                self.baseline_p95_write_ms = telemetry.p95_write_latency_ms;

                // Attempt to probe a parameter if available and not quarantined
                if let Some((param, dir)) = self.select_probe_candidate(telemetry) {
                    self.probing_parameter = Some(param);
                    self.probing_direction = Some(dir);
                    self.apply_probe_step(param, dir);
                    self.phase = ControllerPhase::Probing;
                    self.eval_steps_remaining = 3; // Evaluate for 3 steps (~1.5 to 3 seconds)

                    OptimizationDecision {
                        chunk_size_bytes: self.current_chunk_size.as_bytes(),
                        queue_depth: self.current_queue_depth,
                        concurrency: self.current_concurrency,
                        read_ahead_mb: self.current_read_ahead_mb,
                        phase: ControllerPhase::Probing,
                        action_description: format!("Probing {:?} {:?}", dir, param),
                    }
                } else {
                    OptimizationDecision {
                        chunk_size_bytes: self.current_chunk_size.as_bytes(),
                        queue_depth: self.current_queue_depth,
                        concurrency: self.current_concurrency,
                        read_ahead_mb: self.current_read_ahead_mb,
                        phase: ControllerPhase::Settled,
                        action_description: "Pipeline stable at optimal parameters.".into(),
                    }
                }
            }
            ControllerPhase::Probing => {
                if self.eval_steps_remaining > 0 {
                    self.eval_steps_remaining -= 1;
                }
                if self.eval_steps_remaining == 0 {
                    self.phase = ControllerPhase::Evaluating;
                }
                OptimizationDecision {
                    chunk_size_bytes: self.current_chunk_size.as_bytes(),
                    queue_depth: self.current_queue_depth,
                    concurrency: self.current_concurrency,
                    read_ahead_mb: self.current_read_ahead_mb,
                    phase: ControllerPhase::Probing,
                    action_description: "Evaluating probe in progress...".into(),
                }
            }
            ControllerPhase::Evaluating => {
                let current_speed = telemetry.ewma_5s_mbps;
                let current_lat = telemetry.p95_write_latency_ms;
                let gain_pct = if self.baseline_throughput_mbps > 0.0 {
                    ((current_speed - self.baseline_throughput_mbps)
                        / self.baseline_throughput_mbps)
                        * 100.0
                } else {
                    0.0
                };

                let lat_ratio = if self.baseline_p95_write_ms > 0.0 {
                    current_lat / self.baseline_p95_write_ms
                } else {
                    1.0
                };

                // 5% Hysteresis & Latency Guard
                let adopted = gain_pct >= 5.0 && lat_ratio <= 1.25;
                let param = self
                    .probing_parameter
                    .take()
                    .unwrap_or(TunableParameter::ChunkSize);
                let dir = self
                    .probing_direction
                    .take()
                    .unwrap_or(ProbeDirection::Increase);

                let action_msg = if adopted {
                    self.baseline_throughput_mbps = current_speed;
                    self.baseline_p95_write_ms = current_lat;
                    self.reversion_counts.remove(&param);
                    format!(
                        "Adopted probe {:?} {:?}: +{:.1}% speed improvement.",
                        dir, param, gain_pct
                    )
                } else {
                    // Revert the step!
                    self.revert_probe_step(param, dir);
                    let rev_count = self.reversion_counts.entry(param).or_insert(0);
                    *rev_count += 1;
                    if *rev_count >= 2 {
                        // Flip-flop detected: quarantine parameter for 30 cycles
                        self.quarantined_params.insert(param, 30);
                        self.reversion_counts.remove(&param);
                    }
                    format!("Reverted probe {:?} {:?}: Gain ({:.1}%) below +5% threshold or latency degraded.", dir, param, gain_pct)
                };

                self.phase = ControllerPhase::Cooldown;
                self.cooldown_steps_remaining = 5;

                OptimizationDecision {
                    chunk_size_bytes: self.current_chunk_size.as_bytes(),
                    queue_depth: self.current_queue_depth,
                    concurrency: self.current_concurrency,
                    read_ahead_mb: self.current_read_ahead_mb,
                    phase: ControllerPhase::Evaluating,
                    action_description: action_msg,
                }
            }
            ControllerPhase::Cooldown => {
                if self.cooldown_steps_remaining > 0 {
                    self.cooldown_steps_remaining -= 1;
                }
                if self.cooldown_steps_remaining == 0 {
                    self.phase = ControllerPhase::Settled;
                }
                OptimizationDecision {
                    chunk_size_bytes: self.current_chunk_size.as_bytes(),
                    queue_depth: self.current_queue_depth,
                    concurrency: self.current_concurrency,
                    read_ahead_mb: self.current_read_ahead_mb,
                    phase: ControllerPhase::Cooldown,
                    action_description: "Cooldown stabilization after probe evaluation.".into(),
                }
            }
        }
    }

    fn select_probe_candidate(
        &self,
        telemetry: &TransferTelemetry,
    ) -> Option<(TunableParameter, ProbeDirection)> {
        // Rotational disk constraint: Concurrency is strictly prohibited on HDD
        let can_tune_concurrency = !self.scheduler_profile.serialized_batching
            && self.relationship.direction == volterra_profiler::TransferDirection::SsdToSsd;

        // Try ChunkSize increase first if memory budget permits
        if !self
            .quarantined_params
            .contains_key(&TunableParameter::ChunkSize)
        {
            let next_chunk = self.current_chunk_size.step_up();
            if next_chunk > self.current_chunk_size
                && next_chunk.as_bytes() * self.current_concurrency <= self.memory_budget_bytes
                && telemetry.p95_read_latency_ms < 60.0
            {
                return Some((TunableParameter::ChunkSize, ProbeDirection::Increase));
            }
        }

        // Try Concurrency increase on SSD-to-SSD
        if can_tune_concurrency
            && !self
                .quarantined_params
                .contains_key(&TunableParameter::Concurrency)
            && self.current_concurrency < 4
            && self.current_chunk_size.as_bytes() * (self.current_concurrency + 1)
                <= self.memory_budget_bytes
        {
            return Some((TunableParameter::Concurrency, ProbeDirection::Increase));
        }

        None
    }

    fn apply_probe_step(&mut self, param: TunableParameter, dir: ProbeDirection) {
        match (param, dir) {
            (TunableParameter::ChunkSize, ProbeDirection::Increase) => {
                self.current_chunk_size = self.current_chunk_size.step_up();
            }
            (TunableParameter::ChunkSize, ProbeDirection::Decrease) => {
                self.current_chunk_size = self.current_chunk_size.step_down();
            }
            (TunableParameter::Concurrency, ProbeDirection::Increase) => {
                self.current_concurrency = (self.current_concurrency + 1).min(4);
            }
            (TunableParameter::Concurrency, ProbeDirection::Decrease) => {
                self.current_concurrency = (self.current_concurrency.saturating_sub(1)).max(1);
            }
            (TunableParameter::QueueDepth, ProbeDirection::Increase) => {
                self.current_queue_depth += 1;
            }
            (TunableParameter::QueueDepth, ProbeDirection::Decrease) => {
                self.current_queue_depth = (self.current_queue_depth.saturating_sub(1)).max(1);
            }
        }
    }

    fn revert_probe_step(&mut self, param: TunableParameter, dir: ProbeDirection) {
        let rev_dir = match dir {
            ProbeDirection::Increase => ProbeDirection::Decrease,
            ProbeDirection::Decrease => ProbeDirection::Increase,
        };
        self.apply_probe_step(param, rev_dir);
    }
}
