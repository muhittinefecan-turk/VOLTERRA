//
// LiveTransferView.swift
// VolterraMac
//

import SwiftUI
import AppKit

public struct LiveTransferView: View {
    @EnvironmentObject public var transferVM: TransferViewModel
    @EnvironmentObject public var performanceVM: PerformanceViewModel
    @ObservedObject public var l10n = LocalizationManager.shared
    @State private var showingCancelAlert = false

    public init() {}

    public var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                // Header with Job ID and Status Pill
                HStack {
                    VStack(alignment: .leading, spacing: 4) {
                        HStack(spacing: 8) {
                            Text(l10n.t(.activeJob))
                                .font(.headline)
                                .foregroundColor(.secondary)
                            Text(transferVM.activeJobId.isEmpty ? l10n.t(.noActiveTransfer) : transferVM.activeJobId)
                                .font(.headline.monospaced())
                            if !transferVM.activeJobId.isEmpty {
                                Button(action: {
                                    NSPasteboard.general.clearContents()
                                    NSPasteboard.general.setString(transferVM.activeJobId, forType: .string)
                                }) {
                                    Image(systemName: "doc.on.doc")
                                        .font(.caption)
                                }
                                .buttonStyle(.plain)
                                .help("Copy Job ID to clipboard")
                            }
                        }

                        Text("Mode: \(transferVM.selectedMode.displayName) • \(transferVM.statusMessage)")
                            .font(.caption)
                            .foregroundColor(.secondary)
                    }

                    Spacer()

                    // Job State Pill
                    HStack(spacing: 6) {
                        Circle()
                            .fill(transferVM.statusColor)
                            .frame(width: 10, height: 10)
                        Text(transferVM.state.displayName)
                            .font(.subheadline.bold())
                    }
                    .padding(.horizontal, 12)
                    .padding(.vertical, 6)
                    .background(Capsule().fill(transferVM.statusColor.opacity(0.15)))
                }

                // Hero Progress Bar
                VStack(alignment: .leading, spacing: 8) {
                    HStack {
                        Text("\(String(format: "%.1f", transferVM.percentComplete))%")
                            .font(.system(size: 36, weight: .bold, design: .rounded))
                        Spacer()
                        VStack(alignment: .trailing, spacing: 2) {
                            Text("\(transferVM.formattedProcessedBytes) / \(transferVM.formattedTotalBytes)")
                                .font(.subheadline.monospacedDigit())
                            Text("\(transferVM.processedFiles) / \(transferVM.totalFiles)")
                                .font(.caption)
                                .foregroundColor(.secondary)
                        }
                    }

                    ProgressView(value: transferVM.percentComplete, total: 100.0)
                        .progressViewStyle(.linear)

                    if !transferVM.currentFileName.isEmpty {
                        Text(transferVM.currentFileName)
                            .font(.caption2.monospaced())
                            .foregroundColor(.secondary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                    }
                }
                .padding(20)
                .background(RoundedRectangle(cornerRadius: 12).fill(Color(nsColor: .controlBackgroundColor)))

                // Telemetry 6-Card Grid
                LazyVGrid(columns: [GridItem(.flexible()), GridItem(.flexible()), GridItem(.flexible())], spacing: 14) {
                    StatCardView(
                        title: l10n.t(.currentSpeed),
                        primaryValue: "\(String(format: "%.1f", performanceVM.instantaneousMbps)) MB/s",
                        subtitle: "EWMA (5s): \(String(format: "%.1f", performanceVM.ewma5sMbps)) MB/s",
                        icon: "speedometer",
                        accentColor: .blue
                    )

                    StatCardView(
                        title: l10n.t(.estimatedTime),
                        primaryValue: transferVM.formattedETA,
                        subtitle: transferVM.isETAStable ? "Pace Stable" : "Calibrating...",
                        icon: "clock",
                        accentColor: .purple
                    )

                    StatCardView(
                        title: l10n.t(.activeChunkEngine),
                        primaryValue: "\(performanceVM.activeChunkSizeMB) MiB",
                        subtitle: "Concurrency: \(performanceVM.activeConcurrency) workers",
                        icon: "square.stack.3d.down.right.fill",
                        accentColor: .green
                    )

                    StatCardView(
                        title: l10n.t(.readWriteLatency),
                        primaryValue: "R: \(String(format: "%.1f", performanceVM.p50ReadLatencyMs))ms",
                        subtitle: "W: \(String(format: "%.1f", performanceVM.p50WriteLatencyMs))ms (p95: \(String(format: "%.1f", performanceVM.p95WriteLatencyMs))ms)",
                        icon: "waveform.path.ecg",
                        accentColor: .orange
                    )

                    StatCardView(
                        title: l10n.t(.bufferUtilization),
                        primaryValue: "\(String(format: "%.1f", performanceVM.bufferUtilizationPct))%",
                        subtitle: "Backpressure: \(performanceVM.bufferUtilizationPct > 85 ? "High" : "Optimal")",
                        icon: "memorychip",
                        accentColor: .teal
                    )

                    StatCardView(
                        title: l10n.t(.quarantinedFiles),
                        primaryValue: "\(transferVM.quarantinedCount)",
                        subtitle: transferVM.quarantinedCount == 0 ? "0" : l10n.t(.tabErrorCenter),
                        icon: "exclamationmark.triangle.fill",
                        accentColor: transferVM.quarantinedCount == 0 ? .gray : .red
                    )
                }

                // Rolling Sparkline Throughput Chart
                VStack(alignment: .leading, spacing: 8) {
                    HStack {
                        Text(l10n.t(.throughputLast60s))
                            .font(.headline)
                        Spacer()
                        if performanceVM.isCacheBurst {
                            BadgeView(text: "⚡️ OS Write Cache Burst Detected", color: .orange)
                        }
                    }

                    ThroughputChart(dataPoints: performanceVM.telemetryHistory)
                        .frame(height: 180)
                }
                .padding(16)
                .background(RoundedRectangle(cornerRadius: 12).fill(Color(nsColor: .controlBackgroundColor)))

                // Interactive Control Toolbar
                HStack(spacing: 16) {
                    Button(action: { transferVM.togglePauseResume() }) {
                        HStack {
                            Image(systemName: transferVM.isPaused ? "play.fill" : "pause.fill")
                            Text(transferVM.isPaused ? l10n.t(.resumeTransfer) : l10n.t(.pauseTransfer))
                        }
                        .font(.headline)
                        .padding(.horizontal, 16)
                        .padding(.vertical, 8)
                    }
                    .buttonStyle(.bordered)
                    .disabled(!transferVM.canPauseOrResume)

                    Button(action: { showingCancelAlert = true }) {
                        HStack {
                            Image(systemName: "xmark.circle")
                            Text(l10n.t(.cancelTransfer))
                        }
                        .font(.headline)
                        .foregroundColor(.red)
                        .padding(.horizontal, 16)
                        .padding(.vertical, 8)
                    }
                    .buttonStyle(.bordered)
                    .disabled(!transferVM.isActive)

                    Spacer()

                    if transferVM.quarantinedCount > 0 {
                        NavigationLink(value: NavigationTab.errorCenter) {
                            HStack {
                                Image(systemName: "exclamationmark.triangle.fill")
                                Text("\(l10n.t(.tabErrorCenter)) (\(transferVM.quarantinedCount))")
                            }
                            .foregroundColor(.white)
                            .padding(.horizontal, 14)
                            .padding(.vertical, 8)
                            .background(Capsule().fill(Color.red))
                        }
                        .buttonStyle(.plain)
                    }
                }
            }
            .padding(24)
        }
        .alert("Cancel Active Transfer?", isPresented: $showingCancelAlert) {
            Button("Keep Running", role: .cancel) {}
            Button("Cancel Transfer", role: .destructive) {
                transferVM.cancelTransfer()
            }
        } message: {
            Text("Are you sure you want to cancel? Any verified files will remain safe in partial destination storage.")
        }
    }
}
