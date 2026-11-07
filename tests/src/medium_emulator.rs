//! Calibrated Disk Medium Emulator
//! Models rotational seek latency, mechanical head settlement, and sequential transfer bandwidth.
//! Designed for deterministic benchmarks comparing fixed vs adaptive chunk strategies.

#[derive(Debug, Clone)]
pub struct CalibratedIoMedium {
    pub seek_overhead_ms: f64,
    pub raw_bandwidth_mbps: f64,
    pub write_ratio: f64,
}

impl CalibratedIoMedium {
    /// Models an HDD-to-SSD migration workload:
    /// - HDD source: ~6.0 ms rotational seek + 160 MB/s transfer bandwidth
    /// - Fast SSD destination: write latency is ~40% of HDD read time
    pub fn new_hdd_to_ssd() -> Self {
        Self {
            seek_overhead_ms: 6.0,
            raw_bandwidth_mbps: 160.0,
            write_ratio: 0.4,
        }
    }

    /// Models an SSD-to-SSD migration workload:
    /// - Very low seek: ~0.05 ms
    /// - High bandwidth: ~1200 MB/s
    pub fn new_ssd_to_ssd() -> Self {
        Self {
            seek_overhead_ms: 0.05,
            raw_bandwidth_mbps: 1200.0,
            write_ratio: 0.9,
        }
    }

    /// Calculates operation latency in nanoseconds for a given chunk size.
    /// Formula: t_total = t_seek + (size_mb / bandwidth_mbps)
    pub fn simulate_io(&self, chunk_bytes: usize) -> u64 {
        let size_mb = (chunk_bytes as f64) / (1024.0 * 1024.0);
        let transfer_time_ms = (size_mb / self.raw_bandwidth_mbps) * 1000.0;
        let total_time_ms = self.seek_overhead_ms + transfer_time_ms;
        (total_time_ms * 1_000_000.0) as u64
    }

    /// Calculates read latency in nanoseconds for a chunk read.
    pub fn simulate_read(&self, chunk_bytes: usize) -> u64 {
        self.simulate_io(chunk_bytes)
    }

    /// Calculates write latency in nanoseconds for a chunk write.
    pub fn simulate_write(&self, chunk_bytes: usize) -> u64 {
        let read_lat = self.simulate_io(chunk_bytes) as f64;
        (read_lat * self.write_ratio) as u64
    }
}
