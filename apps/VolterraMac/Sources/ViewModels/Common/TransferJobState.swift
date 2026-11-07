//
// TransferJobState.swift
// VolterraMac
//

import Foundation

/// Lifecycle execution state of a transfer job for UI presentation.
public enum TransferJobState: String, CaseIterable, Equatable, Sendable {
    case idle = "Idle"
    case scanning = "Scanning"
    case running = "Running"
    case paused = "Paused"
    case completed = "Completed"
    case failed = "Failed"
    case cancelled = "Cancelled"

    public var isTerminal: Bool {
        self == .completed || self == .failed || self == .cancelled
    }

    public var canPause: Bool {
        self == .running
    }

    public var canResume: Bool {
        self == .paused
    }

    public var canCancel: Bool {
        self == .scanning || self == .running || self == .paused
    }

    public var displayName: String {
        rawValue
    }
}
