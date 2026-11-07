use serde::{Deserialize, Serialize};

/// High-fidelity snapshot of real-time I/O performance and physical medium metrics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferTelemetry {
    // Smoothed throughput windows (MB/s)
    pub ewma_1s_mbps: f64,
    pub ewma_5s_mbps: f64,
    pub ewma_30s_mbps: f64,
    pub instantaneous_mbps: f64,

    // Directional throughput (MB/s)
    pub source_read_mbps: f64,
    pub dest_write_mbps: f64,
    pub physical_throughput_mbps: f64,

    // Read latency distributions (milliseconds)
    pub p50_read_latency_ms: f64,
    pub p95_read_latency_ms: f64,
    pub p99_read_latency_ms: f64,
    pub max_read_latency_ms: f64,

    // Write latency distributions (milliseconds)
    pub p50_write_latency_ms: f64,
    pub p95_write_latency_ms: f64,
    pub p99_write_latency_ms: f64,
    pub max_write_latency_ms: f64,

    // Memory buffer utilization
    pub buffer_utilization_pct: f64,
    pub buffer_bytes_used: usize,
    pub buffer_capacity_bytes: usize,

    // Scheduler and engine state
    pub active_concurrency: usize,
    pub current_chunk_size: usize,

    // Monotonic counters
    pub total_bytes_read: u64,
    pub total_bytes_written: u64,
    pub total_read_ops: u64,
    pub total_write_ops: u64,
    pub read_errors: u64,
    pub write_errors: u64,

    // Physical cache & health discriminator flags
    pub is_cache_burst: bool,
    pub is_stalled: bool,
    pub timestamp_ms: u64,
}

impl Default for TransferTelemetry {
    fn default() -> Self {
        Self {
            ewma_1s_mbps: 0.0,
            ewma_5s_mbps: 0.0,
            ewma_30s_mbps: 0.0,
            instantaneous_mbps: 0.0,
            source_read_mbps: 0.0,
            dest_write_mbps: 0.0,
            physical_throughput_mbps: 0.0,
            p50_read_latency_ms: 0.0,
            p95_read_latency_ms: 0.0,
            p99_read_latency_ms: 0.0,
            max_read_latency_ms: 0.0,
            p50_write_latency_ms: 0.0,
            p95_write_latency_ms: 0.0,
            p99_write_latency_ms: 0.0,
            max_write_latency_ms: 0.0,
            buffer_utilization_pct: 0.0,
            buffer_bytes_used: 0,
            buffer_capacity_bytes: 0,
            active_concurrency: 1,
            current_chunk_size: 8 * 1024 * 1024,
            total_bytes_read: 0,
            total_bytes_written: 0,
            total_read_ops: 0,
            total_write_ops: 0,
            read_errors: 0,
            write_errors: 0,
            is_cache_burst: false,
            is_stalled: false,
            timestamp_ms: 0,
        }
    }
}
