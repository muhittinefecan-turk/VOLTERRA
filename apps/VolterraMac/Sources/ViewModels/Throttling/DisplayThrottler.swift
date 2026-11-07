//
// DisplayThrottler.swift
// VolterraMac
//

import Foundation

/// Raw coalesced state accumulated from high-frequency event stream.
public struct CoalescedTelemetryState: Sendable {
    public var totalFiles: UInt64 = 0
    public var processedFiles: UInt64 = 0
    public var failedFiles: UInt64 = 0
    public var skippedFiles: UInt64 = 0
    public var totalBytes: UInt64 = 0
    public var processedBytes: UInt64 = 0
    public var currentFileName: String = ""
    public var currentSpeedBps: Double = 0.0
    public var rollingSpeedBps: Double = 0.0
    public var etaSeconds: UInt64 = 0
    public var percentComplete: Double = 0.0
    public var ewma1sMbps: Double = 0.0
    public var ewma5sMbps: Double = 0.0
    public var ewma30sMbps: Double = 0.0
    public var instantaneousMbps: Double = 0.0
    public var sourceReadMbps: Double = 0.0
    public var destWriteMbps: Double = 0.0
    public var physicalThroughputMbps: Double = 0.0
    public var p50ReadLatencyMs: Double = 0.0
    public var p95ReadLatencyMs: Double = 0.0
    public var p50WriteLatencyMs: Double = 0.0
    public var p95WriteLatencyMs: Double = 0.0
    public var activeChunkSizeBytes: Int = 8 * 1024 * 1024
    public var activeConcurrency: Int = 1
    public var queueDepth: Int = 1
    public var bufferUtilizationPct: Double = 0.0
    public var bottleneckCode: Int = 0
    public var bottleneckMessage: String = ""
    public var isCacheBurst: Bool = false
    public var isStalled: Bool = false

    public init() {}
}

/// High-performance 60Hz display throttling engine.
/// Coalesces thousands of raw telemetry events into smooth 60fps `@MainActor` updates.
public final class DisplayThrottler: @unchecked Sendable {
    private let lock = NSLock()
    private var state = CoalescedTelemetryState()
    private var isDirty = false
    private var tickerTask: Task<Void, Never>?
    private let flushHandler: @MainActor (CoalescedTelemetryState) -> Void

    public init(flushHandler: @escaping @MainActor (CoalescedTelemetryState) -> Void) {
        self.flushHandler = flushHandler
    }

    deinit {
        stop()
    }

    /// Starts the 60Hz ticker loop (~16.6ms intervals).
    public func start() {
        stop()
        tickerTask = Task.detached(priority: .high) { [weak self] in
            let clock = ContinuousClock()
            let interval = Duration.nanoseconds(16_666_667) // 60 Hz = 16.666 ms

            while !Task.isCancelled {
                try? await clock.sleep(for: interval)

                guard let self = self else { break }
                if let update = self.drainIfDirty() {
                    await MainActor.run {
                        self.flushHandler(update)
                    }
                }
            }
        }
    }

    /// Stops the 60Hz ticker loop.
    public func stop() {
        tickerTask?.cancel()
        tickerTask = nil
    }

    /// Thread-safe update of scanning progress.
    public func updateScanningProgress(files: UInt64, bytes: UInt64) {
        lock.lock()
        state.totalFiles = files
        state.totalBytes = bytes
        isDirty = true
        lock.unlock()
    }

    /// Thread-safe update of chunk progress.
    public func updateChunkProgress(fileId: Int64, bytesCopied: UInt64, totalBytes: UInt64) {
        lock.lock()
        state.processedBytes = bytesCopied
        if totalBytes > 0 {
            state.totalBytes = totalBytes
            state.percentComplete = min(100.0, (Double(bytesCopied) / Double(totalBytes)) * 100.0)
        }
        isDirty = true
        lock.unlock()
    }

    /// Thread-safe update of current active file.
    public func updateActiveFile(path: String) {
        lock.lock()
        state.currentFileName = path
        isDirty = true
        lock.unlock()
    }

    /// Thread-safe update of comprehensive snapshot & telemetry.
    public func updateTelemetry(
        ewma1s: Double,
        ewma5s: Double,
        ewma30s: Double,
        instantMbps: Double,
        sourceReadMbps: Double,
        destWriteMbps: Double,
        physicalMbps: Double,
        p50Read: Double,
        p95Read: Double,
        p50Write: Double,
        p95Write: Double,
        chunkSize: Int,
        concurrency: Int,
        bufferUtil: Double,
        bottleneckCode: Int,
        bottleneckMsg: String,
        isCacheBurst: Bool,
        isStalled: Bool,
        processedBytes: UInt64,
        totalBytes: UInt64,
        completedFiles: UInt64,
        totalFiles: UInt64,
        failedFiles: UInt64,
        etaSeconds: UInt64
    ) {
        lock.lock()
        state.ewma1sMbps = ewma1s
        state.ewma5sMbps = ewma5s
        state.ewma30sMbps = ewma30s
        state.instantaneousMbps = instantMbps
        state.sourceReadMbps = sourceReadMbps
        state.destWriteMbps = destWriteMbps
        state.physicalThroughputMbps = physicalMbps
        state.currentSpeedBps = instantMbps * 1_048_576.0
        state.rollingSpeedBps = ewma5s * 1_048_576.0
        state.p50ReadLatencyMs = p50Read
        state.p95ReadLatencyMs = p95Read
        state.p50WriteLatencyMs = p50Write
        state.p95WriteLatencyMs = p95Write
        state.activeChunkSizeBytes = chunkSize
        state.activeConcurrency = concurrency
        state.bufferUtilizationPct = bufferUtil
        state.bottleneckCode = bottleneckCode
        state.bottleneckMessage = bottleneckMsg
        state.isCacheBurst = isCacheBurst
        state.isStalled = isStalled
        state.processedBytes = processedBytes
        state.totalBytes = totalBytes
        state.processedFiles = completedFiles
        state.totalFiles = totalFiles
        state.failedFiles = failedFiles
        state.etaSeconds = etaSeconds
        if totalBytes > 0 {
            state.percentComplete = min(100.0, (Double(processedBytes) / Double(totalBytes)) * 100.0)
        }
        isDirty = true
        lock.unlock()
    }

    /// Atomically returns the current state if modified since last drain.
    public func drainIfDirty() -> CoalescedTelemetryState? {
        lock.lock()
        defer { lock.unlock() }
        guard isDirty else { return nil }
        isDirty = false
        return state
    }
}
