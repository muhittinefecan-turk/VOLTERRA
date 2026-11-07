//
// ModeSelectorSection.swift
// VolterraMac
//

import SwiftUI
import AppKit

public struct ModeSelectorSection: View {
    @Binding public var selectedMode: TransferMode

    public init(selectedMode: Binding<TransferMode>) {
        self._selectedMode = selectedMode
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Transfer Mode")
                .font(.headline)

            HStack(spacing: 12) {
                ModeCard(
                    mode: .smartHybrid,
                    currentMode: selectedMode,
                    title: "Smart Hybrid",
                    badge: "Recommended",
                    description: "Adaptive HDD/SSD scheduler. Minimizes rotational head seek, tracks EWMA latency, scales chunks 1-64 MiB.",
                    icon: "arrow.triangle.branch"
                ) {
                    selectedMode = .smartHybrid
                }

                ModeCard(
                    mode: .turbo,
                    currentMode: selectedMode,
                    title: "Turbo Mode",
                    badge: "High Speed",
                    description: "Direct I/O (F_NOCACHE), large chunk buffers, concurrent worker pipelines for SSD-to-SSD.",
                    icon: "bolt.fill"
                ) {
                    selectedMode = .turbo
                }

                ModeCard(
                    mode: .safeRecovery,
                    currentMode: selectedMode,
                    title: "Safe Recovery",
                    badge: "Fault Tolerant",
                    description: "Sector/extent fault isolation. Conservative chunks, single sequential thread, zero aggressive retries.",
                    icon: "shield.checkerboard"
                ) {
                    selectedMode = .safeRecovery
                }
            }
        }
    }
}

public struct ModeCard: View {
    public let mode: TransferMode
    public let currentMode: TransferMode
    public let title: String
    public let badge: String
    public let description: String
    public let icon: String
    public let onSelect: () -> Void

    public init(
        mode: TransferMode,
        currentMode: TransferMode,
        title: String,
        badge: String,
        description: String,
        icon: String,
        onSelect: @escaping () -> Void
    ) {
        self.mode = mode
        self.currentMode = currentMode
        self.title = title
        self.badge = badge
        self.description = description
        self.icon = icon
        self.onSelect = onSelect
    }

    public var isSelected: Bool { mode == currentMode }

    public var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Image(systemName: icon)
                    .font(.title2)
                    .foregroundColor(isSelected ? .accentColor : .secondary)
                Spacer()
                Text(badge)
                    .font(.caption2.bold())
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2)
                    .background(Capsule().fill(isSelected ? Color.accentColor.opacity(0.2) : Color.secondary.opacity(0.2)))
                    .foregroundColor(isSelected ? .accentColor : .secondary)
            }

            Text(title)
                .font(.headline)

            Text(description)
                .font(.caption)
                .foregroundColor(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(14)
        .frame(maxWidth: .infinity, minHeight: 120, alignment: .topLeading)
        .background(
            RoundedRectangle(cornerRadius: 10)
                .fill(isSelected ? Color.accentColor.opacity(0.08) : Color(nsColor: .controlBackgroundColor))
        )
        .overlay(
            RoundedRectangle(cornerRadius: 10)
                .stroke(isSelected ? Color.accentColor : Color.primary.opacity(0.1), lineWidth: isSelected ? 2 : 1)
        )
        .contentShape(Rectangle())
        .onTapGesture {
            onSelect()
        }
    }
}
