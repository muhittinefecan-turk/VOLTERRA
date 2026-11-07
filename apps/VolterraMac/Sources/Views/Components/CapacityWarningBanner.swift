//
// CapacityWarningBanner.swift
// VolterraMac
//

import SwiftUI

public struct CapacityWarningBanner: View {
    public let warning: CapacityWarning

    public init(warning: CapacityWarning) {
        self.warning = warning
    }

    public var body: some View {
        HStack(spacing: 12) {
            Image(systemName: "exclamationmark.triangle.fill")
                .font(.title2)
                .foregroundColor(.red)
            VStack(alignment: .leading, spacing: 2) {
                Text("Insufficient Destination Capacity")
                    .font(.headline)
                    .foregroundColor(.red)
                Text("Source requires \(warning.requiredFormatted), but destination only has \(warning.availableFormatted) free. Shortage: \(warning.shortageFormatted).")
                    .font(.caption)
                    .foregroundColor(.primary)
            }
            Spacer()
        }
        .padding(12)
        .background(RoundedRectangle(cornerRadius: 8).fill(Color.red.opacity(0.1)))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(Color.red.opacity(0.3), lineWidth: 1))
    }
}

public struct MetadataWarningBanner: View {
    public let warning: MetadataWarning

    public init(warning: MetadataWarning) {
        self.warning = warning
    }

    public var body: some View {
        HStack(spacing: 12) {
            Image(systemName: "info.circle.fill")
                .font(.title2)
                .foregroundColor(.orange)
            VStack(alignment: .leading, spacing: 2) {
                Text("Filesystem Metadata Notice")
                    .font(.headline)
                    .foregroundColor(.orange)
                Text(warning.message)
                    .font(.caption)
                    .foregroundColor(.primary)
            }
            Spacer()
        }
        .padding(12)
        .background(RoundedRectangle(cornerRadius: 8).fill(Color.orange.opacity(0.1)))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(Color.orange.opacity(0.3), lineWidth: 1))
    }
}
