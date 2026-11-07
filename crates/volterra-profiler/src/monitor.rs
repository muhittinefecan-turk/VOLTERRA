use crate::telemetry::TransferTelemetry;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Instant;

const LATENCY_RESERVOIR_SIZE: usize = 1024;
const BUCKET_COUNT: usize = 300; // 300 buckets x 100ms = 30.0 seconds
const BUCKET_DURATION_MS: u64 = 100;

#[derive(Default, Clone, Copy)]
struct IoBucket {
    timestamp_ms: u64,
    bytes_read: u64,
    bytes_written: u64,
    read_ops: u64,
    write_ops: u64,
}

pub struct PerformanceMonitor {
    start_time: Instant,
    last_update: Mutex<Instant>,
    last_io_time: Mutex<Instant>,

    // Monotonic counters
    total_bytes_read: AtomicU64,
    total_bytes_written: AtomicU64,
    total_read_ops: AtomicU64,
    total_write_ops: AtomicU64,
    read_errors: AtomicU64,
    write_errors: AtomicU64,

    // Latency reservoirs (circular rings)
    read_latencies: Mutex<Vec<u64>>,
    write_latencies: Mutex<Vec<u64>>,
    read_ring_head: AtomicUsize,
    write_ring_head: AtomicUsize,

    // Discrete 100ms rolling buckets
    buckets: Mutex<[IoBucket; BUCKET_COUNT]>,

    // EWMA state (MB/s)
    ewma_1s: Mutex<f64>,
    ewma_5s: Mutex<f64>,
    ewma_30s: Mutex<f64>,

    // Resource state
    buffer_used_bytes: AtomicUsize,
    buffer_capacity_bytes: AtomicUsize,
    active_concurrency: AtomicUsize,
    current_chunk_size: AtomicUsize,

    // Cache discriminator state
    cache_burst_detected: Mutex<bool>,
}

impl Default for PerformanceMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl PerformanceMonitor {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            start_time: now,
            last_update: Mutex::new(now),
            last_io_time: Mutex::new(now),

            total_bytes_read: AtomicU64::new(0),
            total_bytes_written: AtomicU64::new(0),
            total_read_ops: AtomicU64::new(0),
            total_write_ops: AtomicU64::new(0),
            read_errors: AtomicU64::new(0),
            write_errors: AtomicU64::new(0),

            read_latencies: Mutex::new(vec![0; LATENCY_RESERVOIR_SIZE]),
            write_latencies: Mutex::new(vec![0; LATENCY_RESERVOIR_SIZE]),
            read_ring_head: AtomicUsize::new(0),
            write_ring_head: AtomicUsize::new(0),

            buckets: Mutex::new([IoBucket::default(); BUCKET_COUNT]),

            ewma_1s: Mutex::new(0.0),
            ewma_5s: Mutex::new(0.0),
            ewma_30s: Mutex::new(0.0),

            buffer_used_bytes: AtomicUsize::new(0),
            buffer_capacity_bytes: AtomicUsize::new(128 * 1024 * 1024),
            active_concurrency: AtomicUsize::new(1),
            current_chunk_size: AtomicUsize::new(8 * 1024 * 1024),

            cache_burst_detected: Mutex::new(false),
        }
    }

    /// Records a completed read operation.
    pub fn record_read(&self, bytes: usize, latency_ns: u64) {
        let now = Instant::now();
        self.total_bytes_read
            .fetch_add(bytes as u64, Ordering::Relaxed);
        self.total_read_ops.fetch_add(1, Ordering::Relaxed);

        let head = self.read_ring_head.fetch_add(1, Ordering::Relaxed) % LATENCY_RESERVOIR_SIZE;
        {
            let mut ring = self.read_latencies.lock();
            ring[head] = latency_ns;
        }

        self.update_bucket(bytes as u64, 0, 1, 0, now);
        *self.last_io_time.lock() = now;
    }

    /// Records a completed write operation.
    pub fn record_write(&self, bytes: usize, latency_ns: u64) {
        let now = Instant::now();
        self.total_bytes_written
            .fetch_add(bytes as u64, Ordering::Relaxed);
        self.total_write_ops.fetch_add(1, Ordering::Relaxed);

        let head = self.write_ring_head.fetch_add(1, Ordering::Relaxed) % LATENCY_RESERVOIR_SIZE;
        {
            let mut ring = self.write_latencies.lock();
            ring[head] = latency_ns;
        }

        // Cache discriminator check: Latency < 25us (25,000 ns) indicates Darwin UBC RAM cache hit
        if latency_ns < 25_000 && bytes >= 64 * 1024 {
            *self.cache_burst_detected.lock() = true;
        }

        self.update_bucket(0, bytes as u64, 0, 1, now);
        *self.last_io_time.lock() = now;
    }

    pub fn record_read_error(&self) {
        self.read_errors.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_write_error(&self) {
        self.write_errors.fetch_add(1, Ordering::Relaxed);
    }

    pub fn update_buffer_usage(&self, used_bytes: usize, total_budget: usize) {
        self.buffer_used_bytes.store(used_bytes, Ordering::Relaxed);
        self.buffer_capacity_bytes
            .store(total_budget, Ordering::Relaxed);
    }

    pub fn update_concurrency(&self, active_workers: usize) {
        self.active_concurrency
            .store(active_workers, Ordering::Relaxed);
    }

    pub fn update_chunk_size(&self, chunk_size_bytes: usize) {
        self.current_chunk_size
            .store(chunk_size_bytes, Ordering::Relaxed);
    }

    fn update_bucket(&self, r_bytes: u64, w_bytes: u64, r_ops: u64, w_ops: u64, now: Instant) {
        let elapsed_ms = now.duration_since(self.start_time).as_millis() as u64;
        let bucket_idx = ((elapsed_ms / BUCKET_DURATION_MS) as usize) % BUCKET_COUNT;

        let mut buckets = self.buckets.lock();
        let bucket = &mut buckets[bucket_idx];
        let current_bucket_time = (elapsed_ms / BUCKET_DURATION_MS) * BUCKET_DURATION_MS;

        if bucket.timestamp_ms != current_bucket_time {
            // New bucket window initialized
            *bucket = IoBucket {
                timestamp_ms: current_bucket_time,
                bytes_read: r_bytes,
                bytes_written: w_bytes,
                read_ops: r_ops,
                write_ops: w_ops,
            };
        } else {
            bucket.bytes_read += r_bytes;
            bucket.bytes_written += w_bytes;
            bucket.read_ops += r_ops;
            bucket.write_ops += w_ops;
        }
    }

    /// Computes and returns the complete TransferTelemetry snapshot.
    pub fn get_telemetry(&self) -> TransferTelemetry {
        let now = Instant::now();
        let now_ms = now.duration_since(self.start_time).as_millis() as u64;

        // Calculate rolling throughput sums from discrete buckets
        let (r_1s, w_1s, _r_5s, w_5s, _r_30s, w_30s) = {
            let buckets = self.buckets.lock();
            let mut r1 = 0u64;
            let mut w1 = 0u64;
            let mut r5 = 0u64;
            let mut w5 = 0u64;
            let mut r30 = 0u64;
            let mut w30 = 0u64;

            for b in buckets.iter() {
                if b.timestamp_ms == 0 {
                    continue;
                }
                if now_ms >= b.timestamp_ms {
                    let age = now_ms - b.timestamp_ms;
                    if age <= 1_000 {
                        r1 += b.bytes_read;
                        w1 += b.bytes_written;
                    }
                    if age <= 5_000 {
                        r5 += b.bytes_read;
                        w5 += b.bytes_written;
                    }
                    if age <= 30_000 {
                        r30 += b.bytes_read;
                        w30 += b.bytes_written;
                    }
                }
            }
            (r1, w1, r5, w5, r30, w30)
        };

        let inst_mbps = (w_1s as f64) / 1_048_576.0;
        let source_read_mbps = (r_1s as f64) / 1_048_576.0;
        let dest_write_mbps = inst_mbps;

        // Continuous-time EWMA smoothing
        let mut last_up = self.last_update.lock();
        let dt_secs = now.duration_since(*last_up).as_secs_f64().max(0.001);
        *last_up = now;

        let alpha_1s = 1.0 - (-dt_secs / 1.0).exp();
        let alpha_5s = 1.0 - (-dt_secs / 5.0).exp();
        let alpha_30s = 1.0 - (-dt_secs / 30.0).exp();

        let mut e1 = self.ewma_1s.lock();
        let mut e5 = self.ewma_5s.lock();
        let mut e30 = self.ewma_30s.lock();

        *e1 = alpha_1s * inst_mbps + (1.0 - alpha_1s) * *e1;
        *e5 = alpha_5s * ((w_5s as f64 / 5.0) / 1_048_576.0) + (1.0 - alpha_5s) * *e5;
        *e30 = alpha_30s * ((w_30s as f64 / 30.0) / 1_048_576.0) + (1.0 - alpha_30s) * *e30;

        let ewma_1s_mbps = *e1;
        let ewma_5s_mbps = *e5;
        let ewma_30s_mbps = *e30;

        // Latency percentiles
        let (p50_r, p95_r, p99_r, max_r) = self.calculate_percentiles(
            &self.read_latencies,
            self.total_read_ops.load(Ordering::Relaxed),
        );
        let (p50_w, p95_w, p99_w, max_w) = self.calculate_percentiles(
            &self.write_latencies,
            self.total_write_ops.load(Ordering::Relaxed),
        );

        // Memory buffer usage
        let used = self.buffer_used_bytes.load(Ordering::Relaxed);
        let cap = self.buffer_capacity_bytes.load(Ordering::Relaxed);
        let buf_pct = if cap > 0 {
            (used as f64 / cap as f64) * 100.0
        } else {
            0.0
        };

        // Stall detector: If active concurrency > 0 and no I/O for > 2 seconds
        let last_io = *self.last_io_time.lock();
        let active_workers = self.active_concurrency.load(Ordering::Relaxed);
        let is_stalled = active_workers > 0 && now.duration_since(last_io).as_secs_f64() > 2.0;

        // Cache burst discriminator:
        let is_burst = *self.cache_burst_detected.lock();
        // If cache burst was detected, physical throughput is discounted based on realistic medium write rate
        let physical_throughput = if is_burst && p95_w > 100.0 {
            // Under kernel flush stall, physical rate is the smoothed 30s rate
            ewma_30s_mbps
        } else {
            dest_write_mbps
        };

        TransferTelemetry {
            ewma_1s_mbps,
            ewma_5s_mbps,
            ewma_30s_mbps,
            instantaneous_mbps: inst_mbps,
            source_read_mbps,
            dest_write_mbps,
            physical_throughput_mbps: physical_throughput,
            p50_read_latency_ms: p50_r,
            p95_read_latency_ms: p95_r,
            p99_read_latency_ms: p99_r,
            max_read_latency_ms: max_r,
            p50_write_latency_ms: p50_w,
            p95_write_latency_ms: p95_w,
            p99_write_latency_ms: p99_w,
            max_write_latency_ms: max_w,
            buffer_utilization_pct: buf_pct,
            buffer_bytes_used: used,
            buffer_capacity_bytes: cap,
            active_concurrency: active_workers,
            current_chunk_size: self.current_chunk_size.load(Ordering::Relaxed),
            total_bytes_read: self.total_bytes_read.load(Ordering::Relaxed),
            total_bytes_written: self.total_bytes_written.load(Ordering::Relaxed),
            total_read_ops: self.total_read_ops.load(Ordering::Relaxed),
            total_write_ops: self.total_write_ops.load(Ordering::Relaxed),
            read_errors: self.read_errors.load(Ordering::Relaxed),
            write_errors: self.write_errors.load(Ordering::Relaxed),
            is_cache_burst: is_burst,
            is_stalled,
            timestamp_ms: now_ms,
        }
    }

    fn calculate_percentiles(
        &self,
        ring: &Mutex<Vec<u64>>,
        total_ops: u64,
    ) -> (f64, f64, f64, f64) {
        if total_ops == 0 {
            return (0.0, 0.0, 0.0, 0.0);
        }

        let mut samples = {
            let guard = ring.lock();
            let count = (total_ops as usize).min(LATENCY_RESERVOIR_SIZE);
            guard[..count].to_vec()
        };

        if samples.is_empty() {
            return (0.0, 0.0, 0.0, 0.0);
        }

        samples.sort_unstable();
        let len = samples.len();

        let p50_idx = ((len as f64 * 0.50) as usize).min(len - 1);
        let p95_idx = ((len as f64 * 0.95) as usize).min(len - 1);
        let p99_idx = ((len as f64 * 0.99) as usize).min(len - 1);
        let max_idx = len - 1;

        let to_ms = |ns: u64| (ns as f64) / 1_000_000.0;

        (
            to_ms(samples[p50_idx]),
            to_ms(samples[p95_idx]),
            to_ms(samples[p99_idx]),
            to_ms(samples[max_idx]),
        )
    }

    pub fn reset(&self) {
        let now = Instant::now();
        *self.last_update.lock() = now;
        *self.last_io_time.lock() = now;
        self.total_bytes_read.store(0, Ordering::Relaxed);
        self.total_bytes_written.store(0, Ordering::Relaxed);
        self.total_read_ops.store(0, Ordering::Relaxed);
        self.total_write_ops.store(0, Ordering::Relaxed);
        self.read_errors.store(0, Ordering::Relaxed);
        self.write_errors.store(0, Ordering::Relaxed);
        *self.ewma_1s.lock() = 0.0;
        *self.ewma_5s.lock() = 0.0;
        *self.ewma_30s.lock() = 0.0;
        *self.cache_burst_detected.lock() = false;
        *self.buckets.lock() = [IoBucket::default(); BUCKET_COUNT];
    }
}
