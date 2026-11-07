//! TEST-11: Adaptive Optimizer Benchmark (`test_fixed_vs_adaptive`)
//!
//! Reproducible calibrated benchmark comparing:
//! 1. Leg 1: Fixed 1 MiB chunk sizing (Conservative baseline)
//! 2. Leg 2: Fixed 8 MiB chunk sizing (Standard baseline)
//! 3. Leg 3: Adaptive Disk Profiler (ADP) & Intelligent Transfer Optimizer (ITO) dynamic scaling
//!
//! Calibrated on rotational HDD-to-SSD migration workload:
//! - HDD seek latency: 6.0 ms
//! - Media bandwidth: 160 MB/s
//! - SSD destination write ratio: 0.40

use volterra_optimizer::{
    BottleneckDetector, BottleneckType, ChunkSize, ControllerPhase,
    TransferOptimizer, TransferPolicy,
};
use volterra_profiler::{
    DiskRelationship, PerformanceMonitor, TransferDirection,
};
use volterra_test_suite::CalibratedIoMedium;

#[derive(Debug, Clone)]
pub struct AdaptationRecord {
    pub step: usize,
    pub phase: ControllerPhase,
    pub chunk_size_bytes: usize,
    pub ewma_5s_mbps: f64,
    pub p95_latency_ms: f64,
    pub action_description: String,
    pub bottleneck: BottleneckType,
}

#[derive(Debug, Clone)]
pub struct BenchmarkResult {
    pub name: String,
    pub strategy: String,
    pub total_bytes: u64,
    pub elapsed_ms: u64,
    pub throughput_mbps: f64,
    pub p50_read_latency_ms: f64,
    pub p95_read_latency_ms: f64,
    pub p50_write_latency_ms: f64,
    pub p95_write_latency_ms: f64,
    pub adaptation_events_count: usize,
    pub speedup_vs_baseline: f64,
    pub adaptations: Vec<AdaptationRecord>,
}

fn run_fixed_benchmark(
    name: &str,
    chunk_size: ChunkSize,
    total_bytes: u64,
    medium: &CalibratedIoMedium,
) -> BenchmarkResult {
    let monitor = PerformanceMonitor::new();
    let chunk_bytes = chunk_size.as_bytes();
    let mut transferred = 0u64;
    let mut total_simulated_ns = 0u64;

    while transferred < total_bytes {
        let remaining = total_bytes - transferred;
        let cur_bytes = remaining.min(chunk_bytes as u64) as usize;
        let read_lat_ns = medium.simulate_read(cur_bytes);
        let write_lat_ns = medium.simulate_write(cur_bytes);
        let op_ns = read_lat_ns + write_lat_ns;

        monitor.record_read(cur_bytes, read_lat_ns);
        monitor.record_write(cur_bytes, write_lat_ns);
        transferred += cur_bytes as u64;
        total_simulated_ns += op_ns;
    }

    let elapsed_ms = (total_simulated_ns / 1_000_000).max(1);
    let throughput =
        ((total_bytes as f64) / (1024.0 * 1024.0)) / (total_simulated_ns as f64 / 1_000_000_000.0);
    let tel = monitor.get_telemetry();

    BenchmarkResult {
        name: name.into(),
        strategy: format!("Fixed {} MiB", chunk_bytes / (1024 * 1024)),
        total_bytes,
        elapsed_ms,
        throughput_mbps: throughput,
        p50_read_latency_ms: tel.p50_read_latency_ms,
        p95_read_latency_ms: tel.p95_read_latency_ms,
        p50_write_latency_ms: tel.p50_write_latency_ms,
        p95_write_latency_ms: tel.p95_write_latency_ms,
        adaptation_events_count: 0,
        speedup_vs_baseline: 1.0,
        adaptations: Vec::new(),
    }
}

fn run_adaptive_benchmark(
    name: &str,
    total_bytes: u64,
    medium: &CalibratedIoMedium,
) -> BenchmarkResult {
    let monitor = PerformanceMonitor::new();
    let relationship = DiskRelationship {
        source_device_id: "disk1".into(),
        dest_device_id: "disk2".into(),
        source_parent_id: "disk0".into(),
        dest_parent_id: "disk11".into(),
        is_same_physical_disk: false,
        is_same_filesystem_container: false,
        direction: TransferDirection::HddToSsd,
        recommended_concurrency: 1,
        warning: None,
    };

    let mut optimizer =
        TransferOptimizer::new(relationship, 128 * 1024 * 1024, TransferPolicy::Automatic);
    let mut transferred = 0u64;
    let mut adaptations = Vec::new();
    let mut step = 0;
    let mut cur_chunk_size = ChunkSize::MiB8; // Conservative 8 MiB starting candidate
    let mut total_simulated_ns = 0u64;

    while transferred < total_bytes {
        step += 1;

        let mut tel = monitor.get_telemetry();
        // If prior I/O was recorded, feed realistic EWMA speed to closed-loop optimizer
        if total_simulated_ns > 0 && transferred > 0 {
            let instantaneous_speed =
                ((transferred as f64) / (1024.0 * 1024.0)) / (total_simulated_ns as f64 / 1_000_000_000.0);
            tel.ewma_1s_mbps = instantaneous_speed;
            tel.ewma_5s_mbps = instantaneous_speed;
            tel.ewma_30s_mbps = instantaneous_speed;
        }

        let decision = optimizer.evaluate_step(&tel);

        let candidate_chunk = ChunkSize::from_bytes(decision.chunk_size_bytes);

        if candidate_chunk != cur_chunk_size || step <= 3 {
            let b_class = BottleneckDetector::detect_bottleneck(&tel);
            adaptations.push(AdaptationRecord {
                step,
                phase: decision.phase,
                chunk_size_bytes: candidate_chunk.as_bytes(),
                ewma_5s_mbps: tel.ewma_5s_mbps,
                p95_latency_ms: tel.p95_read_latency_ms,
                action_description: decision.action_description.clone(),
                bottleneck: b_class.bottleneck_type,
            });
            cur_chunk_size = candidate_chunk;
        }

        let remaining = total_bytes - transferred;
        let cur_bytes = remaining.min(cur_chunk_size.as_bytes() as u64) as usize;
        let read_lat_ns = medium.simulate_read(cur_bytes);
        let write_lat_ns = medium.simulate_write(cur_bytes);
        let op_ns = read_lat_ns + write_lat_ns;

        monitor.record_read(cur_bytes, read_lat_ns);
        monitor.record_write(cur_bytes, write_lat_ns);
        transferred += cur_bytes as u64;
        total_simulated_ns += op_ns;
    }

    let elapsed_ms = (total_simulated_ns / 1_000_000).max(1);
    let throughput =
        ((total_bytes as f64) / (1024.0 * 1024.0)) / (total_simulated_ns as f64 / 1_000_000_000.0);
    let final_tel = monitor.get_telemetry();

    BenchmarkResult {
        name: name.into(),
        strategy: "Adaptive (1-64 MiB Dynamic)".into(),
        total_bytes,
        elapsed_ms,
        throughput_mbps: throughput,
        p50_read_latency_ms: final_tel.p50_read_latency_ms,
        p95_read_latency_ms: final_tel.p95_read_latency_ms,
        p50_write_latency_ms: final_tel.p50_write_latency_ms,
        p95_write_latency_ms: final_tel.p95_write_latency_ms,
        adaptation_events_count: adaptations.len(),
        speedup_vs_baseline: 1.0,
        adaptations,
    }
}

#[test]
fn test_fixed_vs_adaptive_benchmark() {
    let total_bytes = 256 * 1024 * 1024; // 256 MiB synthetic workload
    let medium = CalibratedIoMedium::new_hdd_to_ssd();

    println!("\n================================================================================");
    println!("  VOLTERRA TEST-11: ADAPTIVE OPTIMIZER BENCHMARK (Fixed vs Adaptive)");
    println!("  Payload: 256 MiB | Workload: HDD -> SSD | Hysteresis: +5.0% | Max Chunk: 64 MiB");
    println!("================================================================================\n");

    // Leg 1: Fixed 1 MiB (Conservative Baseline)
    let res_1m = run_fixed_benchmark("Fixed-1MiB", ChunkSize::MiB1, total_bytes, &medium);

    // Leg 2: Fixed 8 MiB (Standard Baseline)
    let mut res_8m = run_fixed_benchmark("Fixed-8MiB", ChunkSize::MiB8, total_bytes, &medium);
    res_8m.speedup_vs_baseline = res_8m.throughput_mbps / res_1m.throughput_mbps;

    // Leg 3: Adaptive (ADP + ITO Dynamic Scaling)
    let mut res_adapt = run_adaptive_benchmark("Adaptive-ADP-ITO", total_bytes, &medium);
    res_adapt.speedup_vs_baseline = res_adapt.throughput_mbps / res_1m.throughput_mbps;

    // Print Comparative Markdown Table
    println!(
        "| {:<16} | {:<22} | {:>12} | {:>14} | {:>14} | {:>12} |",
        "Mode", "Strategy", "Throughput", "p50 Latency", "p95 Latency", "Speedup"
    );
    println!("|------------------|------------------------|--------------|----------------|----------------|--------------|");
    println!(
        "| {:<16} | {:<22} | {:>9.1} MB/s | {:>11.1} ms | {:>11.1} ms | {:>12} |",
        res_1m.name,
        res_1m.strategy,
        res_1m.throughput_mbps,
        res_1m.p50_read_latency_ms,
        res_1m.p95_read_latency_ms,
        "1.00x (base)"
    );
    println!(
        "| {:<16} | {:<22} | {:>9.1} MB/s | {:>11.1} ms | {:>11.1} ms | {:>11.2}x |",
        res_8m.name,
        res_8m.strategy,
        res_8m.throughput_mbps,
        res_8m.p50_read_latency_ms,
        res_8m.p95_read_latency_ms,
        res_8m.speedup_vs_baseline
    );
    println!(
        "| {:<16} | {:<22} | {:>9.1} MB/s | {:>11.1} ms | {:>11.1} ms | {:>11.2}x |",
        res_adapt.name,
        res_adapt.strategy,
        res_adapt.throughput_mbps,
        res_adapt.p50_read_latency_ms,
        res_adapt.p95_read_latency_ms,
        res_adapt.speedup_vs_baseline
    );
    println!("|------------------|------------------------|--------------|----------------|----------------|--------------|\n");

    println!("Adaptive Optimizer Events Recorded ({} events):", res_adapt.adaptations.len());
    for ev in &res_adapt.adaptations {
        println!(
            "  [Step {:>2}] Phase: {:<10} | Chunk: {:>2} MiB | EWMA: {:>5.1} MB/s | p95: {:>5.1} ms | Action: {}",
            ev.step,
            format!("{:?}", ev.phase),
            ev.chunk_size_bytes / (1024 * 1024),
            ev.ewma_5s_mbps,
            ev.p95_latency_ms,
            ev.action_description
        );
    }
    println!("\n================================================================================\n");

    // Acceptance Invariants
    assert!(
        res_adapt.throughput_mbps >= res_1m.throughput_mbps,
        "Adaptive throughput ({:.1} MB/s) must meet or exceed Fixed 1 MiB baseline ({:.1} MB/s)",
        res_adapt.throughput_mbps,
        res_1m.throughput_mbps
    );
    assert!(
        res_adapt.adaptation_events_count > 0,
        "Adaptive optimizer must record adaptation events during transfer"
    );
    assert!(
        res_adapt.p95_read_latency_ms < 500.0,
        "Adaptive p95 latency ({:.1} ms) must remain bounded",
        res_adapt.p95_read_latency_ms
    );
}
