//
// DeviceSelectionCard.swift
// VolterraMac
//

import SwiftUI
import AppKit

public struct DeviceSelectionCard: View {
    public let title: String
    public let icon: String
    public let selectedURL: URL?
    public let diskProfile: VolterraDiskInfo?
    public let isSource: Bool
    public let onSelect: () -> Void

    public init(
        title: String,
        icon: String,
        selectedURL: URL?,
        diskProfile: VolterraDiskInfo?,
        isSource: Bool,
        onSelect: @escaping () -> Void
    ) {
        self.title = title
        self.icon = icon
        self.selectedURL = selectedURL
        self.diskProfile = diskProfile
        self.isSource = isSource
        self.onSelect = onSelect
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Label(title, systemImage: icon)
                    .font(.headline)
                Spacer()
                Button(selectedURL == nil ? "Choose..." : "Change...") {
                    onSelect()
                }
                .buttonStyle(.bordered)
                .controlSize(.small)
            }

            if let url = selectedURL {
                VStack(alignment: .leading, spacing: 6) {
                    Text(url.path)
                        .font(.subheadline.bold())
                        .lineLimit(1)
                        .truncationMode(.middle)

                    if let disk = diskProfile {
                        HStack(spacing: 8) {
                            BadgeView(text: disk.fsType, color: .blue)
                            BadgeView(text: disk.isRotational ? "HDD" : "SSD", color: disk.isRotational ? .orange : .green)
                            BadgeView(text: disk.transport.isEmpty ? "Direct" : disk.transport, color: .purple)
                            if disk.isRemovable {
                                BadgeView(text: "External", color: .gray)
                            }
                        }

                        Text("Device: \(disk.bsdName)\(disk.model.isEmpty ? "" : " • Model: " + disk.model)")
                            .font(.caption)
                            .foregroundColor(.secondary)

                        DiskCapacityBar(
                            totalBytes: disk.totalBytes,
                            freeBytes: disk.freeBytes,
                            isSource: isSource
                        )
                    } else {
                        Text("Detecting device metadata...")
                            .font(.caption)
                            .foregroundColor(.secondary)
                    }
                }
            } else {
                VStack(spacing: 8) {
                    Image(systemName: "plus.rectangle.on.folder")
                        .font(.largeTitle)
                        .foregroundColor(.secondary)
                    Text("Select a folder or volume to \(isSource ? "copy from" : "copy to")")
                        .font(.caption)
                        .foregroundColor(.secondary)
                }
                .frame(maxWidth: .infinity, minHeight: 100)
            }
        }
        .padding(16)
        .frame(maxWidth: .infinity)
        .background(RoundedRectangle(cornerRadius: 12).fill(Color(nsColor: .controlBackgroundColor)))
        .overlay(
            RoundedRectangle(cornerRadius: 12)
                .stroke(Color.primary.opacity(0.1), lineWidth: 1)
        )
    }
}
