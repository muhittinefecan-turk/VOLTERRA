//
// DevicePerformancePanel.swift
// VolterraMac
//

import SwiftUI
import AppKit

public struct DevicePerformancePanel: View {
    public let title: String
    public let diskProfile: VolterraDiskInfo?
    public let rateMbps: Double
    public let latencyP50: Double
    public let latencyP95: Double
    public let isSource: Bool

    public init(
        title: String,
        diskProfile: VolterraDiskInfo?,
        rateMbps: Double,
        latencyP50: Double,
        latencyP95: Double,
        isSource: Bool
    ) {
        self.title = title
        self.diskProfile = diskProfile
        self.rateMbps = rateMbps
        self.latencyP50 = latencyP50
        self.latencyP95 = latencyP95
        self.isSource = isSource
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Label(title, systemImage: isSource ? "arrow.down.circle" : "arrow.up.circle")
                    .font(.headline)
                Spacer()
                if let disk = diskProfile {
                    BadgeView(text: disk.isRotational ? "HDD" : "SSD", color: disk.isRotational ? .orange : .green)
                }
            }

            if let disk = diskProfile {
                VStack(alignment: .leading, spacing: 4) {
                    Text(disk.volumeName.isEmpty ? disk.bsdName : disk.volumeName)
                        .font(.subheadline.bold())
                    Text("FS: \(disk.fsType) • Transport: \(disk.transport.isEmpty ? "Direct" : disk.transport)")
                        .font(.caption)
                        .foregroundColor(.secondary)
                }
            } else {
                Text("Device not selected or profiling...")
                    .font(.caption)
                    .foregroundColor(.secondary)
            }

            Divider()

            HStack(spacing: 20) {
                VStack(alignment: .leading, spacing: 2) {
                    Text("Rate")
                        .font(.caption2)
                        .foregroundColor(.secondary)
                    Text(String(format: "%.1f MB/s", rateMbps))
                        .font(.title3.bold().monospacedDigit())
                        .foregroundColor(isSource ? .blue : .purple)
                }

                VStack(alignment: .leading, spacing: 2) {
                    Text("p50 Latency")
                        .font(.caption2)
                        .foregroundColor(.secondary)
                    Text(String(format: "%.1f ms", latencyP50))
                        .font(.subheadline.bold().monospacedDigit())
                }

                VStack(alignment: .leading, spacing: 2) {
                    Text("p95 Latency")
                        .font(.caption2)
                        .foregroundColor(.secondary)
                    Text(String(format: "%.1f ms", latencyP95))
                        .font(.subheadline.bold().monospacedDigit())
                        .foregroundColor(latencyP95 > 100 ? .red : .primary)
                }
            }
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 12).fill(Color(nsColor: .controlBackgroundColor)))
        .overlay(
            RoundedRectangle(cornerRadius: 12)
                .stroke(Color.primary.opacity(0.1), lineWidth: 1)
        )
    }
}
