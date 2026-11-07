//
// BottleneckBanner.swift
// VolterraMac
//

import SwiftUI

public struct BottleneckBanner: View {
    public let code: BottleneckCode
    public let message: String

    public init(code: BottleneckCode, message: String) {
        self.code = code
        self.message = message
    }

    public var color: Color {
        switch code {
        case .nominal: return .green
        case .sourceRead, .destWrite: return .orange
        case .interface, .metadata, .smallFile: return .yellow
        case .contention, .errorDegradation: return .red
        case .unknown, .cpuIntegrity: return .blue
        }
    }

    public var iconName: String {
        switch code {
        case .nominal: return "checkmark.circle.fill"
        case .sourceRead: return "arrow.down.doc.fill"
        case .destWrite: return "arrow.up.doc.fill"
        case .interface: return "cable.connector.horizontal"
        case .metadata: return "doc.badge.gearshape.fill"
        case .smallFile: return "tray.2.fill"
        case .contention: return "exclamationmark.arrow.triangle.2.circlepath"
        case .errorDegradation: return "shield.slash.fill"
        case .cpuIntegrity, .unknown: return "info.circle.fill"
        }
    }

    public var body: some View {
        HStack(alignment: .top, spacing: 14) {
            Image(systemName: iconName)
                .font(.system(size: 28))
                .foregroundColor(color)
            VStack(alignment: .leading, spacing: 4) {
                HStack {
                    Text("Bottleneck Analysis: \(code.displayName)")
                        .font(.headline)
                        .foregroundColor(color)
                    Spacer()
                    BadgeView(text: code.statusTag, color: color)
                }
                Text(message.isEmpty ? "System operating nominally." : message)
                    .font(.subheadline)
                    .foregroundColor(.primary.opacity(0.85))
            }
        }
        .padding(16)
        .background(RoundedRectangle(cornerRadius: 12).fill(color.opacity(0.1)))
        .overlay(RoundedRectangle(cornerRadius: 12).stroke(color.opacity(0.3), lineWidth: 1))
    }
}
