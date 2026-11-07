//
// ExtentMapView.swift
// VolterraMac
//

import SwiftUI

public struct ExtentMapView: View {
    public let totalBytes: UInt64
    public let extents: [LogicalExtentItem]

    public init(totalBytes: UInt64, extents: [LogicalExtentItem]) {
        self.totalBytes = totalBytes
        self.extents = extents
    }

    public var body: some View {
        GeometryReader { geo in
            ZStack(alignment: .leading) {
                // Background healthy readable fill
                RoundedRectangle(cornerRadius: 6)
                    .fill(Color.green.opacity(0.6))

                // Overlaid failed extent slices
                if totalBytes > 0 {
                    ForEach(extents) { ext in
                        let startFraction = CGFloat(ext.offset) / CGFloat(totalBytes)
                        let lengthFraction = max(0.01, CGFloat(ext.length) / CGFloat(totalBytes))
                        let xPos = geo.size.width * startFraction
                        let width = max(4.0, geo.size.width * lengthFraction)

                        RoundedRectangle(cornerRadius: 3)
                            .fill(Color.red)
                            .frame(width: min(width, geo.size.width - xPos), height: geo.size.height)
                            .offset(x: xPos)
                    }
                }
            }
        }
        .frame(height: 24)
        .overlay(
            RoundedRectangle(cornerRadius: 6)
                .stroke(Color.primary.opacity(0.15), lineWidth: 1)
        )
    }
}
