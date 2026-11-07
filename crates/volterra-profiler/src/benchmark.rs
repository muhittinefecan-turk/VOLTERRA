use crate::discovery::HardwareHealth;
use crate::error::ProfilerError;
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::io::{AsRawFd, RawFd};
use std::path::{Path, PathBuf};
use std::time::Instant;
use volterra_platform_macos::{PlatformFs, PlatformIo};

pub struct PassiveProfiler {
    observed_read_mbps: f64,
    observed_write_mbps: f64,
}

impl Default for PassiveProfiler {
    fn default() -> Self {
        Self::new()
    }
}

impl PassiveProfiler {
    pub fn new() -> Self {
        Self {
            observed_read_mbps: 0.0,
            observed_write_mbps: 0.0,
        }
    }

    pub fn record_sample(&mut self, read_mbps: f64, write_mbps: f64) {
        if read_mbps > 0.0 {
            self.observed_read_mbps = (self.observed_read_mbps * 0.8) + (read_mbps * 0.2);
        }
        if write_mbps > 0.0 {
            self.observed_write_mbps = (self.observed_write_mbps * 0.8) + (write_mbps * 0.2);
        }
    }

    pub fn sustainable_read_speed(&self) -> f64 {
        self.observed_read_mbps
    }

    pub fn sustainable_write_speed(&self) -> f64 {
        self.observed_write_mbps
    }
}

#[derive(Debug, Clone)]
pub struct BenchmarkOptions {
    pub test_size_bytes: usize,
    pub chunk_size_bytes: usize,
    pub direct_io: bool,
    pub is_failing_disk: bool,
    pub hardware_health: HardwareHealth,
}

impl Default for BenchmarkOptions {
    fn default() -> Self {
        Self {
            test_size_bytes: 32 * 1024 * 1024, // 32 MiB safe default
            chunk_size_bytes: 4 * 1024 * 1024, // 4 MiB chunk
            direct_io: true,
            is_failing_disk: false,
            hardware_health: HardwareHealth::Clean,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkResult {
    pub write_speed_mbps: f64,
    pub read_speed_mbps: f64,
    pub sync_latency_ms: f64,
    pub p50_write_latency_ms: f64,
    pub p95_write_latency_ms: f64,
    pub p50_read_latency_ms: f64,
    pub p95_read_latency_ms: f64,
    pub test_size_bytes: u64,
}

/// RAII Drop Guard ensuring test files are always deleted from disk on completion, error, or panic.
pub struct BenchmarkFileGuard {
    pub path: PathBuf,
    pub fd: Option<RawFd>,
}

impl Drop for BenchmarkFileGuard {
    fn drop(&mut self) {
        if let Some(fd) = self.fd.take() {
            unsafe {
                libc::close(fd);
            }
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

pub struct SafeBenchmarker;

impl SafeBenchmarker {
    /// Executes a controlled, opt-in active disk benchmark with guaranteed cleanup and failing disk safeguards.
    pub fn run_active_benchmark(
        target_dir: &Path,
        options: BenchmarkOptions,
    ) -> Result<BenchmarkResult, ProfilerError> {
        // 1. Strict Failing Disk Guard
        if options.is_failing_disk || options.hardware_health == HardwareHealth::Failing {
            return Err(ProfilerError::FailingDiskBenchmarkProhibited {
                reason: "Disk has reported hardware errors or failing SMART status. Active stress testing is strictly prohibited.".to_string(),
            });
        }

        // 2. Pre-flight Capacity & Space Check
        let stats = PlatformFs::query_stats(target_dir)?;
        if stats.is_readonly {
            return Err(ProfilerError::DiscoveryFailed {
                path: target_dir.to_path_buf(),
                reason: "Target volume is read-only".to_string(),
            });
        }

        let min_required = (options.test_size_bytes as u64)
            .saturating_mul(2)
            .max(500 * 1024 * 1024);
        if stats.available_bytes < min_required {
            return Err(ProfilerError::InsufficientSpaceForBenchmark {
                path: target_dir.to_path_buf(),
                required_bytes: min_required,
                available_bytes: stats.available_bytes,
            });
        }

        // 3. Create dedicated temporary benchmark file with exclusive creation
        let temp_filename = format!(".volterra.bench.{}.tmp", uuid::Uuid::new_v4());
        let temp_path = target_dir.join(&temp_filename);

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&temp_path)
            .map_err(ProfilerError::BenchmarkIoError)?;

        let raw_fd = file.as_raw_fd();

        // 4. Register RAII Drop Guard for guaranteed cleanup
        let _guard = BenchmarkFileGuard {
            path: temp_path.clone(),
            fd: None, // file owns fd; guard handles path unlinking
        };

        if options.direct_io {
            let _ = PlatformIo::set_direct_io(raw_fd);
        }

        let chunk_size = options.chunk_size_bytes.max(64 * 1024);
        let test_size = options.test_size_bytes.max(chunk_size);
        let chunks_count = test_size / chunk_size;

        // Deterministic synthetic buffer
        let mut buffer = vec![0x5Au8; chunk_size];
        let mut write_latencies = Vec::with_capacity(chunks_count);
        let mut read_latencies = Vec::with_capacity(chunks_count);

        // 5. Sequential Write Benchmark Phase
        let mut writer = file;
        let start_write = Instant::now();
        for _ in 0..chunks_count {
            let t0 = Instant::now();
            writer
                .write_all(&buffer)
                .map_err(ProfilerError::BenchmarkIoError)?;
            let elapsed_ns = t0.elapsed().as_nanos() as u64;
            write_latencies.push(elapsed_ns);
        }

        let t_sync = Instant::now();
        writer.sync_all().map_err(ProfilerError::BenchmarkIoError)?;
        let sync_latency_ms = t_sync.elapsed().as_secs_f64() * 1000.0;
        let total_write_time = start_write.elapsed().as_secs_f64();

        let write_speed_mbps = ((test_size as f64) / 1_048_576.0) / total_write_time.max(0.0001);

        // 6. Sequential Read Benchmark Phase
        let mut reader = writer;
        reader
            .seek(SeekFrom::Start(0))
            .map_err(ProfilerError::BenchmarkIoError)?;

        let start_read = Instant::now();
        for _ in 0..chunks_count {
            let t0 = Instant::now();
            reader
                .read_exact(&mut buffer)
                .map_err(ProfilerError::BenchmarkIoError)?;
            let elapsed_ns = t0.elapsed().as_nanos() as u64;
            read_latencies.push(elapsed_ns);
        }
        let total_read_time = start_read.elapsed().as_secs_f64();
        let read_speed_mbps = ((test_size as f64) / 1_048_576.0) / total_read_time.max(0.0001);

        // 7. Calculate Latency Percentiles
        write_latencies.sort_unstable();
        read_latencies.sort_unstable();

        let p50_w = (write_latencies[write_latencies.len() / 2] as f64) / 1_000_000.0;
        let p95_w_idx =
            ((write_latencies.len() as f64 * 0.95) as usize).min(write_latencies.len() - 1);
        let p95_w = (write_latencies[p95_w_idx] as f64) / 1_000_000.0;

        let p50_r = (read_latencies[read_latencies.len() / 2] as f64) / 1_000_000.0;
        let p95_r_idx =
            ((read_latencies.len() as f64 * 0.95) as usize).min(read_latencies.len() - 1);
        let p95_r = (read_latencies[p95_r_idx] as f64) / 1_000_000.0;

        // End of function: _guard drops and automatically unlinks temp file
        Ok(BenchmarkResult {
            write_speed_mbps,
            read_speed_mbps,
            sync_latency_ms,
            p50_write_latency_ms: p50_w,
            p95_write_latency_ms: p95_w,
            p50_read_latency_ms: p50_r,
            p95_read_latency_ms: p95_r,
            test_size_bytes: test_size as u64,
        })
    }
}
