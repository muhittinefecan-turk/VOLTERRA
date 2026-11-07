//
// DiskCapacityBar.swift
// VolterraMac
//

import SwiftUI

public struct DiskCapacityBar: View {
    public let totalBytes: UInt64
    public let freeBytes: UInt64
    public let isSource: Bool

    public init(totalBytes: UInt64, freeBytes: UInt64, isSource: Bool = false) {
        self.totalBytes = totalBytes
        self.freeBytes = freeBytes
        self.isSource = isSource
    }

    public var usedBytes: UInt64 {
        totalBytes > freeBytes ? totalBytes - freeBytes : 0
    }

    public var usedPercentage: Double {
        guard totalBytes > 0 else { return 0 }
        return Double(usedBytes) / Double(totalBytes)
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            GeometryReader { geo in
                ZStack(alignment: .leading) {
                    Capsule()
                        .fill(Color.secondary.opacity(0.2))
                    Capsule()
                        .fill(usedPercentage > 0.9 ? Color.red : Color.blue)
                        .frame(width: geo.size.width * CGFloat(min(max(usedPercentage, 0.0), 1.0)))
                }
            }
            .frame(height: 8)

            HStack {
                Text("\(ByteCountFormatter.string(fromByteCount: Int64(usedBytes), countStyle: .file)) used")
                Spacer()
                Text("\(ByteCountFormatter.string(fromByteCount: Int64(freeBytes), countStyle: .file)) free")
            }
            .font(.caption2)
            .foregroundColor(.secondary)
        }
    }
}
