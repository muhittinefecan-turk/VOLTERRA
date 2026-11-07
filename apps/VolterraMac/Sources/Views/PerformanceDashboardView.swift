//
// PerformanceDashboardView.swift
// VolterraMac
//

import SwiftUI

public struct PerformanceDashboardView: View {
    @EnvironmentObject public var performanceVM: PerformanceViewModel
    @EnvironmentObject public var transferVM: TransferViewModel
    @ObservedObject public var l10n = LocalizationManager.shared

    public init() {}

    public var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                // Header
                VStack(alignment: .leading, spacing: 4) {
                    Text(l10n.t(.perfTitle))
                        .font(.largeTitle.bold())
                    Text(l10n.t(.perfSubtitle))
                        .foregroundColor(.secondary)
                }

                // Bottleneck Diagnostic Banner
                BottleneckBanner(
                    code: performanceVM.bottleneckCode,
                    message: performanceVM.bottleneckMessage
                )

                // Side-by-Side Device Telemetry Panels
                HStack(spacing: 16) {
                    DevicePerformancePanel(
                        title: l10n.t(.sourceDeviceRead),
                        diskProfile: transferVM.sourceDisk,
                        rateMbps: performanceVM.sourceReadMbps,
                        latencyP50: performanceVM.p50ReadLatencyMs,
                        latencyP95: performanceVM.p95ReadLatencyMs,
                        isSource: true
                    )

                    DevicePerformancePanel(
                        title: l10n.t(.destDeviceWrite),
                        diskProfile: transferVM.destinationDisk,
                        rateMbps: performanceVM.destWriteMbps,
                        latencyP50: performanceVM.p50WriteLatencyMs,
                        latencyP95: performanceVM.p95WriteLatencyMs,
                        isSource: false
                    )
                }

                // Multi-Line Rolling Throughput Chart
                VStack(alignment: .leading, spacing: 10) {
                    HStack {
                        Text(l10n.t(.throughputProfiles))
                            .font(.headline)
                        Spacer()
                        HStack(spacing: 16) {
                            LegendItem(label: "1s Instantaneous", color: .cyan)
                            LegendItem(label: "5s EWMA", color: .blue)
                            LegendItem(label: "30s Smoothed", color: .indigo)
                            LegendItem(label: "Physical Verified", color: .green)
                        }
                    }

                    MultiLineThroughputChart(dataPoints: performanceVM.telemetryHistory)
                        .frame(height: 220)
                }
                .padding(16)
                .background(RoundedRectangle(cornerRadius: 12).fill(Color(nsColor: .controlBackgroundColor)))

                // Adaptive Chunk Engine Status Ladder
                VStack(alignment: .leading, spacing: 12) {
                    Text(l10n.t(.adaptiveChunkLadder))
                        .font(.headline)

                    ChunkEngineVisualizer(
                        candidateSizesMB: [1, 4, 8, 16, 32, 64],
                        activeSizeMB: performanceVM.activeChunkSizeMB
                    )

                    HStack(spacing: 24) {
                        Label("Current Concurrency: \(performanceVM.activeConcurrency) workers", systemImage: "person.2.fill")
                        Label("Buffer Pool Budget: \(transferVM.memoryBudgetMB) MiB", systemImage: "memorychip")
                        Label("Verified Integrity: BLAKE3 256-bit", systemImage: "checkmark.seal.fill")
                    }
                    .font(.caption)
                    .foregroundColor(.secondary)
                    .padding(.top, 4)
                }
                .padding(16)
                .background(RoundedRectangle(cornerRadius: 12).fill(Color(nsColor: .controlBackgroundColor)))
            }
            .padding(24)
        }
    }
}
