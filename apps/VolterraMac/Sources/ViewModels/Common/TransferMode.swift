//
// TransferMode.swift
// VolterraMac
//

import Foundation

/// Transfer operation mode selector.
public enum TransferMode: Int, CaseIterable, Identifiable, Sendable {
    case turbo = 0
    case smartHybrid = 1
    case safeRecovery = 2

    public var id: Int { rawValue }

    public var title: String {
        switch self {
        case .turbo: return "Turbo Mode"
        case .smartHybrid: return "Smart Hybrid"
        case .safeRecovery: return "Safe Recovery"
        }
    }

    public var displayName: String {
        title
    }

    public var subtitle: String {
        switch self {
        case .turbo:
            return "Direct I/O, large chunks, concurrent worker pipelines for SSD-to-SSD."
        case .smartHybrid:
            return "Adaptive HDD/SSD scheduler. Minimizes head seek, tracks EWMA latency, scales chunks 1-64 MiB."
        case .safeRecovery:
            return "Defensive single-worker mode, small chunks, sector fault isolation."
        }
    }

    public var description: String {
        subtitle
    }

    public var badge: String {
        switch self {
        case .turbo: return "High Speed"
        case .smartHybrid: return "Recommended"
        case .safeRecovery: return "Fault Tolerant"
        }
    }

    public var systemImage: String {
        switch self {
        case .turbo: return "bolt.fill"
        case .smartHybrid: return "arrow.triangle.branch"
        case .safeRecovery: return "shield.checkerboard"
        }
    }
}
