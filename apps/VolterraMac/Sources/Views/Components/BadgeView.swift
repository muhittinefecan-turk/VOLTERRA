//
// BadgeView.swift
// VolterraMac
//

import SwiftUI

public struct BadgeView: View {
    public let text: String
    public let color: Color

    public init(text: String, color: Color) {
        self.text = text
        self.color = color
    }

    public var body: some View {
        Text(text)
            .font(.caption2.bold())
            .padding(.horizontal, 8)
            .padding(.vertical, 3)
            .background(Capsule().fill(color.opacity(0.18)))
            .foregroundColor(color)
            .overlay(Capsule().stroke(color.opacity(0.3), lineWidth: 0.5))
    }
}

public struct LegendItem: View {
    public let label: String
    public let color: Color

    public init(label: String, color: Color) {
        self.label = label
        self.color = color
    }

    public var body: some View {
        HStack(spacing: 5) {
            Circle()
                .fill(color)
                .frame(width: 8, height: 8)
            Text(label)
                .font(.caption2)
                .foregroundColor(.secondary)
        }
    }
}

public struct LegendIndicator: View {
    public let color: Color
    public let text: String

    public init(color: Color, text: String) {
        self.color = color
        self.text = text
    }

    public var body: some View {
        HStack(spacing: 4) {
            RoundedRectangle(cornerRadius: 2)
                .fill(color)
                .frame(width: 10, height: 10)
            Text(text)
                .foregroundColor(.secondary)
        }
    }
}
