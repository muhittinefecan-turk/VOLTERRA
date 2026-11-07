//
// PerformanceViewModel.swift
// VolterraMac
//

import Foundation
import SwiftUI
import Combine

@MainActor
public final class PerformanceViewModel: ObservableObject {
    // MARK: - Rolling Throughput (MB/s)
    @Published public private(set) var instantaneousMbps: Double = 0.0
    @Published public private(set) var ewma1sMbps: Double = 0.0
    @Published public private(set) var ewma5sMbps: Double = 0.0
    @Published public private(set) var ewma30sMbps: Double = 0.0
    @Published public private(set) var sourceReadMbps: Double = 0.0
    @Published public private(set) var destWriteMbps: Double = 0.0

    // MARK: - Physical vs Cache Discriminator
    @Published public private(set) var physicalThroughputMbps: Double = 0.0
    @Published public private(set) var isCacheBurst: Bool = false
    @Published public private(set) var isStalled: Bool = false

    // MARK: - Latencies (ms)
    @Published public private(set) var p50ReadLatencyMs: Double = 0.0
    @Published public private(set) var p95ReadLatencyMs: Double = 0.0
    @Published public private(set) var p50WriteLatencyMs: Double = 0.0
    @Published public private(set) var p95WriteLatencyMs: Double = 0.0

    // MARK: - Optimizer & Scheduler State
    @Published public private(set) var activeChunkSizeBytes: Int = 8 * 1024 * 1024
    @Published public private(set) var activeConcurrency: Int = 1
    @Published public private(set) var queueDepth: Int = 1
    @Published public private(set) var bufferUtilizationPct: Double = 0.0

    // MARK: - Bottleneck Diagnosis
    @Published public private(set) var bottleneck: BottleneckInfo = .nominal

    // MARK: - Chart Ring Buffers (Capped at 60 points)
    @Published public private(set) var throughputHistory: [ThroughputPoint] = []
    @Published public private(set) var latencyHistory: [LatencyPoint] = []

    private let maxHistorySamples = 60

    public init() {}

    public var telemetryHistory: [ThroughputPoint] { throughputHistory }

    public var bottleneckCode: BottleneckCode { bottleneck.type }
    public var bottleneckMessage: String { bottleneck.userMessage }

    public var activeChunkSizeMB: Int {
        max(1, activeChunkSizeBytes / (1024 * 1024))
    }

    public var activeChunkSizeFormatted: String {
        let mib = activeChunkSizeBytes / (1024 * 1024)
        return "\(mib) MiB"
    }

    public var cacheBurstDescription: String? {
        guard isCacheBurst else { return nil }
        let physical = String(format: "%.1f MB/s", physicalThroughputMbps)
        let app = String(format: "%.1f MB/s", instantaneousMbps)
        return "OS Page Cache Burst: App rate \(app) exceeds physical disk capability (\(physical))."
    }

    /// Invoked at 60Hz from the coalesced display throttler.
    public func applyTelemetryUpdate(_ update: CoalescedTelemetryState) {
        self.instantaneousMbps = update.instantaneousMbps
        self.ewma1sMbps = update.ewma1sMbps
        self.ewma5sMbps = update.ewma5sMbps
        self.ewma30sMbps = update.ewma30sMbps
        self.sourceReadMbps = update.sourceReadMbps
        self.destWriteMbps = update.destWriteMbps
        self.physicalThroughputMbps = update.physicalThroughputMbps
        self.isCacheBurst = update.isCacheBurst
        self.isStalled = update.isStalled

        self.p50ReadLatencyMs = update.p50ReadLatencyMs
        self.p95ReadLatencyMs = update.p95ReadLatencyMs
        self.p50WriteLatencyMs = update.p50WriteLatencyMs
        self.p95WriteLatencyMs = update.p95WriteLatencyMs

        self.activeChunkSizeBytes = update.activeChunkSizeBytes
        self.activeConcurrency = update.activeConcurrency
        self.bufferUtilizationPct = update.bufferUtilizationPct

        // Bottleneck resolution
        let bType = BottleneckType(rawValue: update.bottleneckCode) ?? .nominal
        self.bottleneck = BottleneckInfo(
            type: bType,
            title: bType.title,
            userMessage: update.bottleneckMessage.isEmpty ? bType.title : update.bottleneckMessage,
            isWarning: bType != .nominal
        )

        // Append to ring buffers
        let now = Date()
        let tPoint = ThroughputPoint(
            timestamp: now,
            instantaneousMbps: update.instantaneousMbps,
            ewma1sMbps: update.ewma1sMbps,
            ewma5sMbps: update.ewma5sMbps,
            ewma30sMbps: update.ewma30sMbps,
            physicalThroughputMbps: update.physicalThroughputMbps
        )
        throughputHistory.append(tPoint)
        if throughputHistory.count > maxHistorySamples {
            throughputHistory.removeFirst(throughputHistory.count - maxHistorySamples)
        }

        let lPoint = LatencyPoint(
            timestamp: now,
            p50ReadMs: update.p50ReadLatencyMs,
            p95ReadMs: update.p95ReadLatencyMs,
            p50WriteMs: update.p50WriteLatencyMs,
            p95WriteMs: update.p95WriteLatencyMs
        )
        latencyHistory.append(lPoint)
        if latencyHistory.count > maxHistorySamples {
            latencyHistory.removeFirst(latencyHistory.count - maxHistorySamples)
        }
    }

    public func clearHistory() {
        throughputHistory.removeAll()
        latencyHistory.removeAll()
        bottleneck = .nominal
        isCacheBurst = false
        isStalled = false
    }
}
