use crate::state::JobLifecycleState;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobSnapshot {
    pub job_id: String,
    pub state: JobLifecycleState,
    pub total_files: u64,
    pub completed_files: u64,
    pub failed_files: u64,
    pub skipped_files: u64,
    pub in_flight_files: u64,
    pub total_bytes: u64,
    pub transferred_bytes: u64,
    pub verified_bytes: u64,
    pub remaining_bytes: u64,
    pub percent_complete: f64,
    pub current_throughput_mbps: f64,
    pub ewma_1s_mbps: f64,
    pub ewma_5s_mbps: f64,
    pub ewma_30s_mbps: f64,
    pub p50_read_latency_ms: f64,
    pub p95_read_latency_ms: f64,
    pub p50_write_latency_ms: f64,
    pub p95_write_latency_ms: f64,
    pub active_chunk_size_bytes: usize,
    pub active_concurrency: usize,
    pub buffer_utilization_pct: f64,
    pub estimated_remaining_seconds: Option<u64>,
    pub active_bottleneck: Option<String>,
    pub quarantined_errors_count: usize,
}
