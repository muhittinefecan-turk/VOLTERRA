//
// StatCardView.swift
// VolterraMac
//

import SwiftUI
import AppKit

public struct StatCardView: View {
    public let title: String
    public let primaryValue: String
    public let subtitle: String
    public let icon: String
    public let accentColor: Color

    public init(
        title: String,
        primaryValue: String,
        subtitle: String,
        icon: String,
        accentColor: Color = .blue
    ) {
        self.title = title
        self.primaryValue = primaryValue
        self.subtitle = subtitle
        self.icon = icon
        self.accentColor = accentColor
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text(title)
                    .font(.caption)
                    .foregroundColor(.secondary)
                Spacer()
                Image(systemName: icon)
                    .font(.caption)
                    .foregroundColor(accentColor)
            }

            Text(primaryValue)
                .font(.title2.bold())
                .foregroundColor(.primary)
                .lineLimit(1)
                .minimumScaleFactor(0.8)

            Text(subtitle)
                .font(.caption2)
                .foregroundColor(.secondary)
                .lineLimit(1)
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 10).fill(Color(nsColor: .controlBackgroundColor)))
        .overlay(
            RoundedRectangle(cornerRadius: 10)
                .stroke(Color.primary.opacity(0.08), lineWidth: 1)
        )
    }
}
