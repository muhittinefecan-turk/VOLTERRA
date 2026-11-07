//
// DisplayThrottlerTests.swift
// VolterraMacTests
//

import XCTest
import Foundation
@testable import VolterraMac

final class DisplayThrottlerTests: XCTestCase {

    func testDrainIfDirty() {
        let throttler = DisplayThrottler { _ in }

        // Initially not dirty
        XCTAssertNil(throttler.drainIfDirty())

        // Update scanning progress makes it dirty
        throttler.updateScanningProgress(files: 100, bytes: 50_000_000)
        let state1 = throttler.drainIfDirty()
        XCTAssertNotNil(state1)
        XCTAssertEqual(state1?.totalFiles, 100)
        XCTAssertEqual(state1?.totalBytes, 50_000_000)

        // Second drain without changes is nil
        XCTAssertNil(throttler.drainIfDirty())

        // Update chunk progress makes it dirty again
        throttler.updateChunkProgress(fileId: 42, bytesCopied: 25_000_000, totalBytes: 50_000_000)
        let state2 = throttler.drainIfDirty()
        XCTAssertNotNil(state2)
        XCTAssertEqual(state2?.processedBytes, 25_000_000)
        XCTAssertEqual(state2?.percentComplete, 50.0)
    }

    func testHighFrequencyTelemetryCoalescing() async throws {
        var flushCount = 0
        var lastState: CoalescedTelemetryState?

        let throttler = DisplayThrottler { state in
            flushCount += 1
            lastState = state
        }

        throttler.start()

        // Rapidly push 2,000 telemetry updates simulating high-frequency I/O chunk events
        for i in 1...2000 {
            throttler.updateTelemetry(
                ewma1s: Double(i) * 0.1,
                ewma5s: Double(i) * 0.08,
                ewma30s: Double(i) * 0.05,
                instantMbps: 150.0,
                sourceReadMbps: 160.0,
                destWriteMbps: 140.0,
                physicalMbps: 130.0,
                p50Read: 1.5,
                p95Read: 4.2,
                p50Write: 2.1,
                p95Write: 5.8,
                chunkSize: 8 * 1024 * 1024,
                concurrency: 2,
                bufferUtil: 45.0,
                bottleneckCode: 0,
                bottleneckMsg: "Nominal",
                isCacheBurst: false,
                isStalled: false,
                processedBytes: UInt64(i * 1024 * 1024),
                totalBytes: 2000 * 1024 * 1024,
                completedFiles: UInt64(i / 10),
                totalFiles: 200,
                failedFiles: 0,
                etaSeconds: 60
            )
        }

        // Wait ~100ms for several 60Hz ticks to occur (~6 ticks)
        try await Task.sleep(nanoseconds: 100_000_000)

        throttler.stop()

        // Wait another 50ms to ensure ticker is done
        try await Task.sleep(nanoseconds: 50_000_000)

        // Verify that flushes occurred and were bounded (substantially fewer than 2,000 flushes)
        XCTAssertGreaterThan(flushCount, 0, "Expected at least one 60Hz flush pass")
        XCTAssertLessThan(flushCount, 100, "60Hz throttler should bound flush count, got \(flushCount)")

        // Verify that the final state contains the accumulated data
        if let last = lastState {
            XCTAssertEqual(last.processedBytes, 2000 * 1024 * 1024)
            XCTAssertEqual(last.percentComplete, 100.0)
            XCTAssertEqual(last.activeConcurrency, 2)
        }
    }
}
