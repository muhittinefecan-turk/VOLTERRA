use volterra_optimizer::{
    AdaptiveChunkEngine, BottleneckDetector, BottleneckType, ChunkSize, ControllerPhase,
    IoScheduler, TransferOptimizer, TransferPolicy,
};
use volterra_profiler::{DiskRelationship, TransferDirection, TransferTelemetry};

fn make_dummy_relationship(
    direction: TransferDirection,
    warning: Option<String>,
) -> DiskRelationship {
    DiskRelationship {
        source_device_id: "disk1".into(),
        dest_device_id: "disk2".into(),
        source_parent_id: "disk0".into(),
        dest_parent_id: if direction == TransferDirection::SameDiskTransfer {
            "disk0".into()
        } else {
            "disk11".into()
        },
        is_same_physical_disk: direction == TransferDirection::SameDiskTransfer,
        is_same_filesystem_container: false,
        direction,
        recommended_concurrency: 1,
        warning,
    }
}

#[test]
fn test_adaptive_chunk_candidates() {
    let candidates = ChunkSize::all_candidates();
    assert_eq!(candidates.len(), 6);
    assert_eq!(ChunkSize::MiB1.as_bytes(), 1024 * 1024);
    assert_eq!(ChunkSize::MiB64.as_bytes(), 64 * 1024 * 1024);

    let mut c = ChunkSize::MiB1;
    c = c.step_up();
    assert_eq!(c, ChunkSize::MiB4);
    c = c.step_up();
    assert_eq!(c, ChunkSize::MiB8);
    c = c.step_up();
    assert_eq!(c, ChunkSize::MiB16);
    c = c.step_up();
    assert_eq!(c, ChunkSize::MiB32);
    c = c.step_up();
    assert_eq!(c, ChunkSize::MiB64);
    c = c.step_up();
    assert_eq!(c, ChunkSize::MiB64); // Cap at 64 MiB

    c = c.step_down();
    assert_eq!(c, ChunkSize::MiB32);
    c = c.step_down();
    assert_eq!(c, ChunkSize::MiB16);
    c = c.step_down();
    assert_eq!(c, ChunkSize::MiB8);
    c = c.step_down();
    assert_eq!(c, ChunkSize::MiB4);
    c = c.step_down();
    assert_eq!(c, ChunkSize::MiB1);
    c = c.step_down();
    assert_eq!(c, ChunkSize::MiB1); // Floor at 1 MiB
}

#[test]
fn test_chunk_engine_file_slicing() {
    let file_id = 42;
    let file_size = 25 * 1024 * 1024; // 25 MiB
    let chunks = AdaptiveChunkEngine::slice_file(file_id, file_size, ChunkSize::MiB8);

    assert_eq!(chunks.len(), 4); // 8, 8, 8, 1
    assert_eq!(chunks[0].offset_bytes, 0);
    assert_eq!(chunks[0].length_bytes, 8 * 1024 * 1024);
    assert_eq!(chunks[1].offset_bytes, 8 * 1024 * 1024);
    assert_eq!(chunks[1].length_bytes, 8 * 1024 * 1024);
    assert_eq!(chunks[2].offset_bytes, 16 * 1024 * 1024);
    assert_eq!(chunks[2].length_bytes, 8 * 1024 * 1024);
    assert_eq!(chunks[3].offset_bytes, 24 * 1024 * 1024);
    assert_eq!(chunks[3].length_bytes, 1024 * 1024);

    let total: u64 = chunks.iter().map(|c| c.length_bytes as u64).sum();
    assert_eq!(total, file_size);
}

#[test]
fn test_chunk_engine_missing_extents() {
    let file_id = 99;
    let missing_extents = vec![(0, 4 * 1024 * 1024), (16 * 1024 * 1024, 2 * 1024 * 1024)];
    let chunks =
        AdaptiveChunkEngine::slice_missing_extents(file_id, &missing_extents, ChunkSize::MiB4);

    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].offset_bytes, 0);
    assert_eq!(chunks[0].length_bytes, 4 * 1024 * 1024);
    assert_eq!(chunks[1].offset_bytes, 16 * 1024 * 1024);
    assert_eq!(chunks[1].length_bytes, 2 * 1024 * 1024);
}

#[test]
fn test_chunk_engine_error_clamping() {
    let tel = TransferTelemetry {
        read_errors: 1,
        ..Default::default()
    };
    let chunk = AdaptiveChunkEngine::determine_chunk_size(
        &tel,
        100 * 1024 * 1024,
        128 * 1024 * 1024,
        2,
        true,
        TransferDirection::SsdToSsd,
    );
    assert_eq!(
        chunk,
        ChunkSize::MiB1,
        "Error state must unconditionally clamp to 1 MiB"
    );
}

#[test]
fn test_scheduler_directional_policies() {
    // HDD -> SSD: 1 reader
    let hdd_ssd = make_dummy_relationship(TransferDirection::HddToSsd, None);
    let prof1 = IoScheduler::create_profile(&hdd_ssd, 128 * 1024 * 1024);
    assert_eq!(prof1.reader_concurrency, 1);
    assert_eq!(prof1.read_ahead_mb, 32);
    assert!(!prof1.serialized_batching);

    // SSD -> HDD: 1 writer
    let ssd_hdd = make_dummy_relationship(TransferDirection::SsdToHdd, None);
    let prof2 = IoScheduler::create_profile(&ssd_hdd, 128 * 1024 * 1024);
    assert_eq!(prof2.writer_concurrency, 1);
    assert_eq!(prof2.read_ahead_mb, 8);

    // Same Disk Rotational: Serialized batching!
    let same_hdd = make_dummy_relationship(
        TransferDirection::SameDiskTransfer,
        Some("rotational head thrashing warning".into()),
    );
    let prof3 = IoScheduler::create_profile(&same_hdd, 128 * 1024 * 1024);
    assert!(prof3.serialized_batching);
    assert_eq!(prof3.read_ahead_mb, 0);
    assert!(prof3.batch_buffer_bytes >= 16 * 1024 * 1024);
}

#[test]
fn test_bottleneck_detection_matrix() {
    // 1. Error Degradation
    let tel_err = TransferTelemetry {
        read_errors: 2,
        ..Default::default()
    };
    let b1 = BottleneckDetector::detect_bottleneck(&tel_err);
    assert_eq!(b1.bottleneck_type, BottleneckType::ErrorDegradation);

    // 2. Destination Write Limited
    let tel_write = TransferTelemetry {
        total_bytes_read: 50 * 1024 * 1024,
        ewma_5s_mbps: 50.0,
        buffer_utilization_pct: 85.0,
        p95_read_latency_ms: 10.0,
        p95_write_latency_ms: 60.0,
        ..Default::default()
    };
    let b2 = BottleneckDetector::detect_bottleneck(&tel_write);
    assert_eq!(b2.bottleneck_type, BottleneckType::DestinationWriteLimited);

    // 3. Source Read Limited
    let tel_read = TransferTelemetry {
        total_bytes_read: 50 * 1024 * 1024,
        ewma_5s_mbps: 40.0,
        buffer_utilization_pct: 15.0,
        p95_read_latency_ms: 70.0,
        p95_write_latency_ms: 15.0,
        ..Default::default()
    };
    let b3 = BottleneckDetector::detect_bottleneck(&tel_read);
    assert_eq!(b3.bottleneck_type, BottleneckType::SourceReadLimited);

    // 4. Interface Constrained (USB 2.0 clamp)
    let tel_usb = TransferTelemetry {
        total_bytes_read: 50 * 1024 * 1024,
        ewma_5s_mbps: 38.5, // in [32..43]
        p95_read_latency_ms: 12.0,
        p95_write_latency_ms: 14.0,
        ..Default::default()
    };
    let b4 = BottleneckDetector::detect_bottleneck(&tel_usb);
    assert_eq!(b4.bottleneck_type, BottleneckType::InterfaceConstrained);
}

#[test]
fn test_closed_loop_controller_hysteresis_and_flip_flop() {
    let rel = make_dummy_relationship(TransferDirection::SsdToSsd, None);
    let mut opt = TransferOptimizer::new(rel, 128 * 1024 * 1024, TransferPolicy::Automatic);

    let mut tel = TransferTelemetry {
        total_bytes_read: 20 * 1024 * 1024,
        ewma_5s_mbps: 100.0,
        p95_read_latency_ms: 15.0,
        p95_write_latency_ms: 20.0,
        ..Default::default()
    };

    // Run warmup (4 steps)
    for _ in 0..4 {
        let dec = opt.evaluate_step(&tel);
        assert!(dec.phase == ControllerPhase::Warmup || dec.phase == ControllerPhase::Settled);
    }

    // Settled step: will pick a probe candidate (ChunkSize increase 16 -> 32 MiB)
    let dec_probe = opt.evaluate_step(&tel);
    assert_eq!(dec_probe.phase, ControllerPhase::Probing);

    // Probing steps (eval_steps_remaining = 3)
    opt.evaluate_step(&tel);
    opt.evaluate_step(&tel);
    opt.evaluate_step(&tel);

    // Evaluating step with only +2% speed improvement (< 5% threshold)
    tel.ewma_5s_mbps = 102.0;
    let dec_eval = opt.evaluate_step(&tel);
    assert_eq!(dec_eval.phase, ControllerPhase::Evaluating);
    assert!(
        dec_eval.action_description.contains("Reverted"),
        "Marginal +2% gain must be reverted under 5% hysteresis"
    );
}
