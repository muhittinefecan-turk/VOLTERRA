//
// ExtentInspectorSheet.swift
// VolterraMac
//

import SwiftUI
import AppKit

public struct ExtentInspectorSheet: View {
    public let file: QuarantinedFileItem
    public let onSkip: () -> Void
    public let onRetry: () -> Void
    public let onDefer: () -> Void
    public let onClose: () -> Void

    public init(
        file: QuarantinedFileItem,
        onSkip: @escaping () -> Void,
        onRetry: @escaping () -> Void,
        onDefer: @escaping () -> Void,
        onClose: @escaping () -> Void
    ) {
        self.file = file
        self.onSkip = onSkip
        self.onRetry = onRetry
        self.onDefer = onDefer
        self.onClose = onClose
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            // Header
            HStack {
                VStack(alignment: .leading, spacing: 4) {
                    Text("Logical Extent Inspector")
                        .font(.title2.bold())
                    Text(file.relativePath)
                        .font(.caption.monospaced())
                        .foregroundColor(.secondary)
                }
                Spacer()
                Button("Done", action: onClose)
                    .keyboardShortcut(.defaultAction)
            }

            // Summary Stats
            HStack(spacing: 20) {
                Label("Size: \(file.formattedSize)", systemImage: "doc")
                Label("Attempts: \(file.attempts)", systemImage: "arrow.clockwise")
                Label("Failed Extents: \(file.extents.count)", systemImage: "exclamationmark.triangle")
            }
            .font(.subheadline)
            .padding(10)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(RoundedRectangle(cornerRadius: 8).fill(Color(nsColor: .controlBackgroundColor)))

            // Graphical Byte-Range Extent Map
            VStack(alignment: .leading, spacing: 6) {
                Text("File Extent Map Visualization (0B to \(file.formattedSize))")
                    .font(.caption.bold())
                ExtentMapView(totalBytes: file.totalSizeBytes, extents: file.extents)
                    .frame(height: 24)
                HStack(spacing: 16) {
                    LegendIndicator(color: .green.opacity(0.7), text: "Readable Content")
                    LegendIndicator(color: .red, text: "Unreadable Sector (EIO)")
                }
                .font(.caption2)
            }

            // Failed Extents Table
            Text("Failed Extent Byte Offsets")
                .font(.headline)

            Table(file.extents) {
                TableColumn("Offset (Bytes)") { ext in
                    Text("\(ext.offset)")
                        .font(.caption.monospacedDigit())
                }
                TableColumn("Length (Bytes)") { ext in
                    Text("\(ext.length)")
                        .font(.caption.monospacedDigit())
                }
                TableColumn("Offset (MiB)") { ext in
                    Text(String(format: "%.2f MiB", Double(ext.offset) / 1048576.0))
                }
                TableColumn("Error") { ext in
                    Text(ext.errorName)
                        .font(.caption.bold())
                        .foregroundColor(.red)
                }
            }
            .frame(height: 180)

            // Interactive Per-File Action Bar
            HStack(spacing: 12) {
                Button("Skip File (Continue Others)", action: onSkip)
                    .buttonStyle(.bordered)

                Button("Defer to Second Pass", action: onDefer)
                    .buttonStyle(.bordered)

                Spacer()

                Button("Retry Now", action: onRetry)
                    .buttonStyle(.borderedProminent)
            }
        }
        .padding(24)
        .frame(width: 640, height: 540)
    }
}
