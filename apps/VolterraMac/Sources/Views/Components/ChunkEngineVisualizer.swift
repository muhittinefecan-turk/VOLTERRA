//
// ChunkEngineVisualizer.swift
// VolterraMac
//

import SwiftUI
import AppKit

public struct ChunkEngineVisualizer: View {
    public let candidateSizesMB: [Int]
    public let activeSizeMB: Int

    public init(candidateSizesMB: [Int] = [1, 4, 8, 16, 32, 64], activeSizeMB: Int) {
        self.candidateSizesMB = candidateSizesMB
        self.activeSizeMB = activeSizeMB
    }

    public var body: some View {
        HStack(spacing: 8) {
            ForEach(candidateSizesMB, id: \.self) { size in
                let isActive = size == activeSizeMB
                VStack(spacing: 4) {
                    Text("\(size) MiB")
                        .font(.subheadline.bold())
                        .foregroundColor(isActive ? .white : .primary)
                    if isActive {
                        Text("ACTIVE")
                            .font(.system(size: 9, weight: .black))
                            .foregroundColor(.white)
                    }
                }
                .frame(maxWidth: .infinity, minHeight: 44)
                .background(
                    RoundedRectangle(cornerRadius: 8)
                        .fill(isActive ? Color.accentColor : Color(nsColor: .separatorColor).opacity(0.3))
                )
                .overlay(
                    RoundedRectangle(cornerRadius: 8)
                        .stroke(isActive ? Color.accentColor : Color.clear, lineWidth: 2)
                )
                .animation(.easeInOut, value: activeSizeMB)
            }
        }
    }
}
