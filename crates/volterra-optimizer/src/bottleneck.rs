use serde::{Deserialize, Serialize};
use volterra_profiler::TransferTelemetry;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BottleneckType {
    SourceReadLimited,
    DestinationWriteLimited,
    InterfaceConstrained,
    MetadataConstrained,
    SmallFileOverhead,
    CpuIntegrityConstrained,
    Contention,
    ErrorDegradation,
    Unknown,
    Nominal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BottleneckSeverity {
    Nominal,
    Mild,
    Severe,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BottleneckEvidence {
    pub ewma_5s_mbps: f64,
    pub p95_read_latency_ms: f64,
    pub p95_write_latency_ms: f64,
    pub buffer_utilization_pct: f64,
    pub read_errors: u64,
    pub write_errors: u64,
}

impl From<&TransferTelemetry> for BottleneckEvidence {
    fn from(t: &TransferTelemetry) -> Self {
        Self {
            ewma_5s_mbps: t.ewma_5s_mbps,
            p95_read_latency_ms: t.p95_read_latency_ms,
            p95_write_latency_ms: t.p95_write_latency_ms,
            buffer_utilization_pct: t.buffer_utilization_pct,
            read_errors: t.read_errors,
            write_errors: t.write_errors,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BottleneckClassification {
    pub bottleneck_type: BottleneckType,
    pub confidence: f32,
    pub severity: BottleneckSeverity,
    pub user_message: String,
    pub technical_reason: String,
    pub recommended_action: String,
    pub evidence: BottleneckEvidence,
}

pub struct BottleneckDetector;

impl BottleneckDetector {
    pub fn detect_bottleneck(telemetry: &TransferTelemetry) -> BottleneckClassification {
        let evidence = BottleneckEvidence::from(telemetry);

        // 1. Error Degradation (Highest Priority Guard)
        if telemetry.read_errors > 0 || telemetry.write_errors > 0 {
            return BottleneckClassification {
                bottleneck_type: BottleneckType::ErrorDegradation,
                confidence: 1.0,
                severity: BottleneckSeverity::Critical,
                user_message: "Drive I/O errors detected. System has fallen back to safe recovery mode.".into(),
                technical_reason: format!(
                    "Unreadable sectors or write failures: read_errors={}, write_errors={}",
                    telemetry.read_errors, telemetry.write_errors
                ),
                recommended_action: "Inspect the Error Center; avoid stress testing or heavy retries on failing disk.".into(),
                evidence,
            };
        }

        // 2. Insufficient Data / Warmup
        if telemetry.total_bytes_read < 4 * 1024 * 1024 && telemetry.ewma_5s_mbps < 0.1 {
            return BottleneckClassification {
                bottleneck_type: BottleneckType::Unknown,
                confidence: 0.5,
                severity: BottleneckSeverity::Nominal,
                user_message: "Gathering baseline transfer telemetry...".into(),
                technical_reason: "Insufficient data points during initial pipeline warmup.".into(),
                recommended_action: "Allow copy to run for a few seconds to gather telemetry."
                    .into(),
                evidence,
            };
        }

        // 3. Destination Write Bottleneck (Buffer Backpressure)
        if telemetry.buffer_utilization_pct > 75.0
            && telemetry.p95_write_latency_ms > 2.0 * telemetry.p95_read_latency_ms
            && telemetry.p95_write_latency_ms > 40.0
        {
            let conf = ((0.70 + ((telemetry.buffer_utilization_pct - 75.0) / 100.0) * 1.2) as f32)
                .min(0.99);
            return BottleneckClassification {
                bottleneck_type: BottleneckType::DestinationWriteLimited,
                confidence: conf,
                severity: if telemetry.p95_write_latency_ms > 120.0 {
                    BottleneckSeverity::Severe
                } else {
                    BottleneckSeverity::Mild
                },
                user_message: "Destination drive write speed is the limiting factor. Memory backpressure is active.".into(),
                technical_reason: format!(
                    "RAM buffer is {:.0}% full; destination write p95 ({:.1}ms) exceeds read p95 ({:.1}ms).",
                    telemetry.buffer_utilization_pct,
                    telemetry.p95_write_latency_ms,
                    telemetry.p95_read_latency_ms
                ),
                recommended_action: "Ensure target drive is not thermal throttling or performing background indexing.".into(),
                evidence,
            };
        }

        // 4. Source Read Bottleneck (Buffer Starvation)
        if telemetry.buffer_utilization_pct < 25.0
            && telemetry.p95_read_latency_ms > 2.0 * telemetry.p95_write_latency_ms
            && telemetry.p95_read_latency_ms > 40.0
        {
            let conf = ((0.70 + ((25.0 - telemetry.buffer_utilization_pct) / 100.0) * 1.2) as f32)
                .min(0.99);
            return BottleneckClassification {
                bottleneck_type: BottleneckType::SourceReadLimited,
                confidence: conf,
                severity: if telemetry.p95_read_latency_ms > 120.0 {
                    BottleneckSeverity::Severe
                } else {
                    BottleneckSeverity::Mild
                },
                user_message: "Source drive read speed is the limiting factor. Destination is waiting for data.".into(),
                technical_reason: format!(
                    "RAM buffer is starved ({:.0}% used); read p95 ({:.1}ms) exceeds write p95 ({:.1}ms).",
                    telemetry.buffer_utilization_pct,
                    telemetry.p95_read_latency_ms,
                    telemetry.p95_write_latency_ms
                ),
                recommended_action: "Ensure source HDD is running sequentially with read-ahead enabled.".into(),
                evidence,
            };
        }

        // 5. Interface Clamping (e.g. USB 2.0 480 Mbps ceiling ~ 35-43 MB/s)
        if telemetry.p95_read_latency_ms < 30.0
            && telemetry.p95_write_latency_ms < 30.0
            && telemetry.ewma_5s_mbps >= 32.0
            && telemetry.ewma_5s_mbps <= 43.0
        {
            return BottleneckClassification {
                bottleneck_type: BottleneckType::InterfaceConstrained,
                confidence: 0.88,
                severity: BottleneckSeverity::Mild,
                user_message:
                    "Throughput is clamped near the theoretical ceiling of USB 2.0 (~40 MB/s)."
                        .into(),
                technical_reason: format!(
                    "Throughput locked at {:.1} MB/s with low disk latencies (<30ms).",
                    telemetry.ewma_5s_mbps
                ),
                recommended_action: "Connect drive to a high-speed USB 3.0 or Thunderbolt port."
                    .into(),
                evidence,
            };
        }

        // 6. Contention / High Latency Variance
        if (telemetry.p95_read_latency_ms > 3.5 * telemetry.p50_read_latency_ms
            && telemetry.p95_read_latency_ms > 80.0)
            || (telemetry.p95_write_latency_ms > 3.5 * telemetry.p50_write_latency_ms
                && telemetry.p95_write_latency_ms > 80.0)
        {
            return BottleneckClassification {
                bottleneck_type: BottleneckType::Contention,
                confidence: 0.78,
                severity: BottleneckSeverity::Mild,
                user_message: "Intermittent latency spikes detected. Another application may be accessing the disk.".into(),
                technical_reason: "High p95/p50 latency variance ratio (>3.5x) indicating external head contention.".into(),
                recommended_action: "Close other disk-heavy background apps (Time Machine, Spotlight indexing).".into(),
                evidence,
            };
        }

        // 7. Nominal / Balanced Flow
        BottleneckClassification {
            bottleneck_type: BottleneckType::Nominal,
            confidence: 0.90,
            severity: BottleneckSeverity::Nominal,
            user_message: "Transfer pipeline is operating nominally at optimal throughput.".into(),
            technical_reason: format!(
                "Balanced throughput {:.1} MB/s with stable latencies.",
                telemetry.ewma_5s_mbps
            ),
            recommended_action: "No action required.".into(),
            evidence,
        }
    }
}
