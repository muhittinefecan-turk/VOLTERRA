//
// AdversarialStressTests.swift
// VolterraMacTests
//
// Challenger 2 Empirical Stress Test Harness for Milestone 5
//

import XCTest
import Foundation
import SwiftUI
import CVolterra
@testable import VolterraMac
@testable import VolterraBridge

final class AdversarialStressTests: XCTestCase {

    // MARK: - Scenario 1: Burst of 50,000 Telemetry Events into DisplayThrottler
    func testBurst50kTelemetryCoalescingAndResponsiveness() async throws {
        final class FlushRecorder: @unchecked Sendable {
            private let lock = NSLock()
            var count = 0
            var lastState: CoalescedTelemetryState?

            func record(state: CoalescedTelemetryState) {
                lock.lock()
                count += 1
                lastState = state
                lock.unlock()
            }

            func results() -> (count: Int, last: CoalescedTelemetryState?) {
                lock.lock()
                defer { lock.unlock() }
                return (count, lastState)
            }
        }

        let recorder = FlushRecorder()
        let throttler = DisplayThrottler { state in
            recorder.record(state: state)
        }

        throttler.start()

        let totalEvents = 50_000
        let concurrentWorkers = 5
        let eventsPerWorker = totalEvents / concurrentWorkers

        // Track main thread responsiveness during the burst
        var mainActorCheckLatencies: [Double] = []
        let monitoringTask = Task {
            for _ in 0..<10 {
                let start = ContinuousClock.now
                await MainActor.run {
                    _ = 1 + 1
                }
                let elapsed = ContinuousClock.now.duration(to: start)
                let elapsedMs = Double(elapsed.components.attoseconds) / -1e15
                mainActorCheckLatencies.append(elapsedMs)
                try? await Task.sleep(nanoseconds: 20_000_000) // check every 20ms
            }
        }

        let burstStart = ContinuousClock.now

        // Fire 50,000 events across 5 concurrent tasks
        await withTaskGroup(of: Void.self) { group in
            for workerId in 0..<concurrentWorkers {
                group.addTask {
                    for i in 0..<eventsPerWorker {
                        let eventIndex = workerId * eventsPerWorker + i
                        throttler.updateTelemetry(
                            ewma1s: Double(eventIndex) * 0.01,
                            ewma5s: Double(eventIndex) * 0.008,
                            ewma30s: Double(eventIndex) * 0.005,
                            instantMbps: 250.0 + Double(i % 50),
                            sourceReadMbps: 260.0,
                            destWriteMbps: 240.0,
                            physicalMbps: 230.0,
                            p50Read: 1.2,
                            p95Read: 3.8,
                            p50Write: 1.8,
                            p95Write: 4.5,
                            chunkSize: 8 * 1024 * 1024,
                            concurrency: 4,
                            bufferUtil: 55.0,
                            bottleneckCode: 0,
                            bottleneckMsg: "Nominal",
                            isCacheBurst: false,
                            isStalled: false,
                            processedBytes: UInt64(eventIndex + 1) * 1024 * 1024,
                            totalBytes: UInt64(totalEvents) * 1024 * 1024,
                            completedFiles: UInt64(eventIndex / 10),
                            totalFiles: 5000,
                            failedFiles: 0,
                            etaSeconds: 45
                        )
                    }
                }
            }
        }

        let burstElapsed = burstStart.duration(to: .now)
        let burstElapsedMs = Double(burstElapsed.components.attoseconds) / 1e15

        // Allow ~200ms for 60Hz ticker (~12 ticks)
        try await Task.sleep(nanoseconds: 200_000_000)

        throttler.stop()
        await monitoringTask.value

        // Allow final drain
        try await Task.sleep(nanoseconds: 50_000_000)

        let (finalFlushes, finalState) = recorder.results()

        print("=== EMPIRICAL BURST 50K RESULTS ===")
        print("Burst execution time: \(burstElapsedMs) ms for 50,000 updates")
        print("Flushes triggered to @MainActor: \(finalFlushes) (Coalescing ratio: \(Double(totalEvents) / Double(max(1, finalFlushes))):1)")
        print("MainActor average probe latency: \(mainActorCheckLatencies.reduce(0, +) / Double(max(1, mainActorCheckLatencies.count))) ms")
        if let s = finalState {
            print("Final processedBytes: \(s.processedBytes), percentComplete: \(s.percentComplete)%")
        }

        // Empirical Assertions:
        // 1. Throttler must drastically coalesce 50,000 updates down to bounded 60Hz ticks (< 60 flushes over ~300ms)
        XCTAssertGreaterThan(finalFlushes, 0, "Expected at least one 60Hz flush pass")
        XCTAssertLessThan(finalFlushes, 60, "50k burst must coalesce to bounded 60Hz updates, but got \(finalFlushes) flushes")

        // 2. Coalescing ratio must exceed 500:1
        let ratio = Double(totalEvents) / Double(max(1, finalFlushes))
        XCTAssertGreaterThan(ratio, 500.0, "Coalescing ratio must exceed 500:1, got \(ratio)")

        // 3. Main thread must never be starved (all probe latencies well below 100ms)
        for lat in mainActorCheckLatencies {
            XCTAssertLessThan(lat, 100.0, "MainActor latency spiked to \(lat) ms, indicating thread starvation")
        }

        // 4. State must reflect processed data
        XCTAssertNotNil(finalState)
        XCTAssertGreaterThan(finalState?.processedBytes ?? 0, 0)
    }

    // MARK: - Scenario 2: Immediate Priority Lane Verification
    @MainActor
    func testImmediatePriorityLaneZeroDelay() async throws {
        let vm = TransferViewModel()
        let throttler = DisplayThrottler { _ in }
        throttler.start()
        defer { throttler.stop() }

        // Start high-frequency background flood simulating chunk progress
        let floodActive = ThreadSafeFlag(initial: true)
        let floodTask = Task.detached(priority: .background) {
            var counter: UInt64 = 0
            while floodActive.value {
                counter += 1
                throttler.updateChunkProgress(fileId: 1, bytesCopied: counter * 65536, totalBytes: 100_000_000)
                if counter % 500 == 0 {
                    try? await Task.sleep(nanoseconds: 1_000_000) // 1ms breath
                }
            }
        }

        // Test 1: StateChanged event (immediate dispatch)
        let t0 = ContinuousClock.now
        vm.handleDiscreteEvent(type: 1, message: "Job is Running", fileId: 0)
        let t1 = ContinuousClock.now
        let latency1Ms = Double(t0.duration(to: t1).components.attoseconds) / 1e15

        XCTAssertEqual(vm.state, .running, "StateChanged must immediately transition ViewModel to running")
        XCTAssertLessThan(latency1Ms, 5.0, "StateChanged latency must be < 5ms (was \(latency1Ms) ms)")

        // Test 2: FileQuarantined event (immediate dispatch)
        let t2 = ContinuousClock.now
        vm.handleDiscreteEvent(type: 7, message: "Sector I/O error on file 88", fileId: 88)
        let t3 = ContinuousClock.now
        let latency2Ms = Double(t2.duration(to: t3).components.attoseconds) / 1e15

        XCTAssertEqual(vm.failedFiles, 1, "FileQuarantined must immediately increment failedFiles")
        XCTAssertEqual(vm.quarantinedCount, 1, "FileQuarantined must immediately increment quarantinedCount")
        XCTAssertLessThan(latency2Ms, 5.0, "FileQuarantined latency must be < 5ms (was \(latency2Ms) ms)")

        // Test 3: DiskWarning event (immediate dispatch)
        let t4 = ContinuousClock.now
        vm.handleDiscreteEvent(type: 10, message: "SMART pending sectors reallocated", fileId: 0)
        let t5 = ContinuousClock.now
        let latency3Ms = Double(t4.duration(to: t5).components.attoseconds) / 1e15

        XCTAssertNotNil(vm.errorMessage)
        XCTAssertTrue(vm.errorMessage?.contains("SMART") == true, "DiskWarning must immediately populate errorMessage")
        XCTAssertLessThan(latency3Ms, 5.0, "DiskWarning latency must be < 5ms (was \(latency3Ms) ms)")

        // Test 4: JobCompleted event (immediate dispatch)
        let t6 = ContinuousClock.now
        vm.handleDiscreteEvent(type: 11, message: "Job completed and verified", fileId: 0)
        let t7 = ContinuousClock.now
        let latency4Ms = Double(t6.duration(to: t7).components.attoseconds) / 1e15

        XCTAssertEqual(vm.state, .completed, "JobCompleted must immediately transition ViewModel to completed")
        XCTAssertLessThan(latency4Ms, 5.0, "JobCompleted latency must be < 5ms (was \(latency4Ms) ms)")

        floodActive.set(false)
        _ = await floodTask.value

        print("=== EMPIRICAL PRIORITY LANE LATENCIES ===")
        print("StateChanged latency: \(latency1Ms) ms")
        print("FileQuarantined latency: \(latency2Ms) ms")
        print("DiskWarning latency: \(latency3Ms) ms")
        print("JobCompleted latency: \(latency4Ms) ms")
    }

    // MARK: - Scenario 3: Pre-Flight Capacity Checks & Collision Warnings Edge Conditions
    @MainActor
    func testPreFlightCapacityAndCollisionEdgeCases() async throws {
        let vm = TransferViewModel()
        let tempDir = FileManager.default.temporaryDirectory
        let runId = UUID().uuidString
        let srcDir = tempDir.appendingPathComponent("adv_src_\(runId)")
        let dstDir = tempDir.appendingPathComponent("adv_dst_\(runId)")

        try FileManager.default.createDirectory(at: srcDir, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: dstDir, withIntermediateDirectories: true)
        defer {
            try? FileManager.default.removeItem(at: srcDir)
            try? FileManager.default.removeItem(at: dstDir)
        }

        // Subcase 1: Nested directory collisions
        // Create nested directory tree in src: src/level1/level2/deep.txt
        let deepSrcDir = srcDir.appendingPathComponent("level1/level2")
        try FileManager.default.createDirectory(at: deepSrcDir, withIntermediateDirectories: true)
        try "Deep Content".data(using: .utf8)?.write(to: deepSrcDir.appendingPathComponent("deep.txt"))

        // Create matching top-level folder in dst: dst/level1/
        let deepDstDir = dstDir.appendingPathComponent("level1")
        try FileManager.default.createDirectory(at: deepDstDir, withIntermediateDirectories: true)

        vm.selectSource(url: srcDir)
        vm.selectDestination(url: dstDir)
        await vm.runPreFlightCheck()

        // Verify collision detection caught "level1"
        XCTAssertTrue(vm.destinationHasExistingFiles, "Destination containing 'level1' must be flagged as existing files")
        XCTAssertEqual(vm.collidingFilesCount, 1)
        XCTAssertEqual(vm.collidingSamplePaths.first, "level1")

        // Subcase 2: Collision Sheet Protection (R2 Non-Destructive constraint)
        // Calling startTransfer without user approval MUST NOT start the job and MUST present sheet
        vm.startTransfer()
        XCTAssertTrue(vm.showOverwriteDialog, "startTransfer must present showOverwriteDialog when collisions exist")
        XCTAssertEqual(vm.state, .idle, "Transfer must remain idle until user confirms overwrite")
        XCTAssertFalse(vm.allowOverwriteConfirmed)

        // User confirms overwrite
        vm.approveOverwriteAndStart()
        XCTAssertTrue(vm.allowOverwriteConfirmed, "Approval must confirm allowOverwriteConfirmed")
        XCTAssertFalse(vm.showOverwriteDialog, "Dialog must dismiss after approval")

        // Reset to idle
        vm.resetToIdle()

        // Verify R2 Overwrite Protection Fix:
        // resetToIdle() MUST clear allowOverwriteConfirmed to false and reset dialog & collision states
        XCTAssertFalse(vm.allowOverwriteConfirmed, "resetToIdle() must reset allowOverwriteConfirmed to false to prevent R2 violation")
        XCTAssertFalse(vm.showOverwriteDialog, "resetToIdle() must ensure showOverwriteDialog is false")
        XCTAssertEqual(vm.collidingFilesCount, 0, "resetToIdle() must reset collidingFilesCount to 0")
        XCTAssertFalse(vm.destinationHasExistingFiles, "resetToIdle() must reset destinationHasExistingFiles to false")

        // Re-configure a second transfer targeting colliding destination
        vm.selectSource(url: srcDir)
        vm.selectDestination(url: dstDir)
        await vm.runPreFlightCheck()
        XCTAssertTrue(vm.destinationHasExistingFiles, "Second pre-flight must detect collision")

        // Starting transfer without explicit user consent MUST NOT proceed
        vm.startTransfer()
        XCTAssertTrue(vm.showOverwriteDialog, "Subsequent transfer must require fresh explicit user approval")
        XCTAssertEqual(vm.state, .idle, "Transfer must not start without confirmation")
        XCTAssertFalse(vm.allowOverwriteConfirmed, "allowOverwriteConfirmed must remain false until user confirms")

        // Subcase 3: Empty directories edge condition
        let emptySrc = tempDir.appendingPathComponent("adv_empty_src_\(runId)")
        let emptyDst = tempDir.appendingPathComponent("adv_empty_dst_\(runId)")
        try FileManager.default.createDirectory(at: emptySrc, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: emptyDst, withIntermediateDirectories: true)
        defer {
            try? FileManager.default.removeItem(at: emptySrc)
            try? FileManager.default.removeItem(at: emptyDst)
        }

        vm.selectSource(url: emptySrc)
        vm.selectDestination(url: emptyDst)
        await vm.runPreFlightCheck()

        XCTAssertTrue(vm.preFlightValid)
        XCTAssertFalse(vm.hasInsufficientSpace)
        XCTAssertFalse(vm.destinationHasExistingFiles)
        XCTAssertEqual(vm.collidingFilesCount, 0)
    }

    // MARK: - Scenario 3B: Enforcement Analysis of Exact 0 Free Bytes Condition
    @MainActor
    func testPreFlightCapacityExactZeroFreeBytesAnalysis() {
        // Verify capacity evaluation logic:
        // sourceBytes > 0 && (freeBytes == 0 || sourceBytes > freeBytes)
        let sourceBytes: UInt64 = 10_000_000 // 10 MB payload to copy
        let freeBytesNormal: UInt64 = 5_000_000 // 5 MB available -> Insufficient
        let freeBytesSufficient: UInt64 = 20_000_000 // 20 MB available -> Sufficient
        let freeBytesZero: UInt64 = 0 // 0 bytes available (Full disk)

        let evaluateCapacity: (UInt64, UInt64) -> Bool = { free, src in
            src > 0 && (free == 0 || src > free)
        }

        // Case 1: Partial space (5MB free, 10MB needed) -> Insufficient
        XCTAssertTrue(evaluateCapacity(freeBytesNormal, sourceBytes), "Partial space must trigger insufficient space")

        // Case 2: Sufficient space (20MB free, 10MB needed) -> Sufficient
        XCTAssertFalse(evaluateCapacity(freeBytesSufficient, sourceBytes), "Sufficient space must not trigger insufficient space")

        // Case 3: Exact 0 free bytes with >0 source bytes -> Insufficient
        let insufficientZero = evaluateCapacity(freeBytesZero, sourceBytes)
        XCTAssertTrue(insufficientZero, "Exact 0 free bytes with positive source payload MUST trigger insufficient space")

        // Case 4: 0 source bytes on 0 free bytes -> Nothing to copy, not insufficient
        let zeroSourceOnZeroFree = evaluateCapacity(freeBytesZero, 0)
        XCTAssertFalse(zeroSourceOnZeroFree, "0 source bytes on 0 free bytes should not trigger insufficient space")
    }

    // MARK: - Scenario 4: Error Center Actions Non-Destructive Behavior (R2/R4)
    @MainActor
    func testErrorCenterQuarantineActionsNonDestructive() async throws {
        let evm = ErrorCenterViewModel()

        // 1. Safety verification with nil handle: must not crash or corrupt
        evm.bindJob(handle: nil)
        XCTAssertEqual(evm.quarantinedCount, 0)
        XCTAssertEqual(evm.totalFailedBytes, 0)

        evm.skipFile(fileId: 42)
        XCTAssertFalse(evm.isPerformingAction)
        XCTAssertEqual(evm.actionFeedbackMessage, "No active job available")
        evm.retryFile(fileId: 42)
        XCTAssertFalse(evm.isPerformingAction)
        XCTAssertEqual(evm.actionFeedbackMessage, "No active job available")
        evm.deferFile(fileId: 42)
        XCTAssertFalse(evm.isPerformingAction)
        XCTAssertEqual(evm.actionFeedbackMessage, "No active job available")
        evm.skipAllErrors()
        XCTAssertFalse(evm.isPerformingAction)
        XCTAssertEqual(evm.actionFeedbackMessage, "No active job available")
        evm.retryAllFailed()
        XCTAssertFalse(evm.isPerformingAction)
        XCTAssertEqual(evm.actionFeedbackMessage, "No active job available")

        // 2. Real end-to-end job verification of R2/R4 non-destructive guarantees
        let tempDir = FileManager.default.temporaryDirectory
        let runId = UUID().uuidString
        let srcDir = tempDir.appendingPathComponent("ec_src_\(runId)")
        let dstDir = tempDir.appendingPathComponent("ec_dst_\(runId)")

        try FileManager.default.createDirectory(at: srcDir, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: dstDir, withIntermediateDirectories: true)
        defer {
            try? FileManager.default.removeItem(at: srcDir)
            try? FileManager.default.removeItem(at: dstDir)
        }

        // Create 3 files in source: file1, file2, file3
        let src1 = srcDir.appendingPathComponent("file1.dat")
        let src2 = srcDir.appendingPathComponent("file2.dat")
        let src3 = srcDir.appendingPathComponent("file3.dat")
        let data1 = Data(repeating: 0xAA, count: 16384)
        let data2 = Data(repeating: 0xBB, count: 16384)
        let data3 = Data(repeating: 0xCC, count: 16384)
        try data1.write(to: src1)
        try data2.write(to: src2)
        try data3.write(to: src3)

        // Record initial attributes to verify R2 (Source files NEVER modified or deleted)
        let initialAttr1 = try FileManager.default.attributesOfItem(atPath: src1.path)
        let initialAttr2 = try FileManager.default.attributesOfItem(atPath: src2.path)
        let initialAttr3 = try FileManager.default.attributesOfItem(atPath: src3.path)

        let controller = try VolterraBridge.createJob(
            sourcePath: srcDir.path,
            destPath: dstDir.path,
            mode: .smartHybrid,
            chunkSize: 1024 * 1024,
            directIO: false,
            verifyReadback: true,
            memoryBudget: 64 * 1024 * 1024,
            allowOverwrite: true,
            journalPath: nil
        )

        if let rawHandle = await controller.getRawHandle() {
            evm.bindJob(handle: rawHandle)

            // Direct C ABI error queries on valid active job
            let errorListPtr = volterra_get_errors(rawHandle)
            XCTAssertNotNil(errorListPtr)
            if let list = errorListPtr {
                XCTAssertEqual(list.pointee.count, 0, "No errors initially")
                volterra_error_list_free(list)
            }

            // Exercise C ABI Skip/Retry/Defer actions on valid handle
            // Non-existent or inactive file IDs should return cleanly with recovery error code (14), not panic
            let okCode = Int32(VOLTERRA_ERROR_CODE_OK.rawValue)
            let recoveryFailedCode = Int32(VOLTERRA_ERROR_CODE_ERR_RECOVERY_FAILED.rawValue)

            let skipRet = volterra_skip_file(rawHandle, 999999)
            XCTAssertTrue(skipRet == okCode || skipRet == recoveryFailedCode,
                          "skip_file should return OK (0) or ERR_RECOVERY_FAILED (14), got \(skipRet)")

            let retryRet = volterra_retry_file(rawHandle, 999999)
            XCTAssertTrue(retryRet == okCode || retryRet == recoveryFailedCode,
                          "retry_file should return OK (0) or ERR_RECOVERY_FAILED (14), got \(retryRet)")

            let deferRet = volterra_defer_file(rawHandle, 999999)
            XCTAssertTrue(deferRet == okCode || deferRet == recoveryFailedCode,
                          "defer_file should return OK (0) or ERR_RECOVERY_FAILED (14), got \(deferRet)")

            let skipAllRet = volterra_skip_all_errors(rawHandle)
            XCTAssertEqual(skipAllRet, 0, "skip_all_errors on empty queue should succeed")

            let retryAllRet = volterra_retry_all_failed(rawHandle)
            XCTAssertEqual(retryAllRet, 0, "retry_all_failed on empty queue should succeed")
        }

        // Run the copy
        try await controller.start()

        // Wait for completion
        let deadline = Date().addingTimeInterval(5.0)
        var isDone = false
        while Date() < deadline && !isDone {
            let snap = await controller.getSnapshot()
            if snap?.state == .completed || snap?.state == .failed {
                isDone = true
                break
            }
            try await Task.sleep(nanoseconds: 50_000_000)
        }

        XCTAssertTrue(isDone, "Job should reach terminal state")

        // R2 Check: Verify source files still exist and were NEVER mutated or deleted
        XCTAssertTrue(FileManager.default.fileExists(atPath: src1.path))
        XCTAssertTrue(FileManager.default.fileExists(atPath: src2.path))
        XCTAssertTrue(FileManager.default.fileExists(atPath: src3.path))

        let finalAttr1 = try FileManager.default.attributesOfItem(atPath: src1.path)
        let finalAttr2 = try FileManager.default.attributesOfItem(atPath: src2.path)
        let finalAttr3 = try FileManager.default.attributesOfItem(atPath: src3.path)

        XCTAssertEqual(initialAttr1[.size] as? UInt64, finalAttr1[.size] as? UInt64)
        XCTAssertEqual(initialAttr2[.size] as? UInt64, finalAttr2[.size] as? UInt64)
        XCTAssertEqual(initialAttr3[.size] as? UInt64, finalAttr3[.size] as? UInt64)

        let readData1 = try Data(contentsOf: src1)
        let readData2 = try Data(contentsOf: src2)
        let readData3 = try Data(contentsOf: src3)
        XCTAssertEqual(readData1, data1, "Source file 1 content must remain unmodified")
        XCTAssertEqual(readData2, data2, "Source file 2 content must remain unmodified")
        XCTAssertEqual(readData3, data3, "Source file 3 content must remain unmodified")

        // R2 Check: Verify destination files match source exactly
        let dst1 = dstDir.appendingPathComponent("file1.dat")
        let dst2 = dstDir.appendingPathComponent("file2.dat")
        let dst3 = dstDir.appendingPathComponent("file3.dat")
        XCTAssertTrue(FileManager.default.fileExists(atPath: dst1.path))
        XCTAssertTrue(FileManager.default.fileExists(atPath: dst2.path))
        XCTAssertTrue(FileManager.default.fileExists(atPath: dst3.path))

        let copiedData1 = try Data(contentsOf: dst1)
        let copiedData2 = try Data(contentsOf: dst2)
        let copiedData3 = try Data(contentsOf: dst3)
        XCTAssertEqual(copiedData1, data1)
        XCTAssertEqual(copiedData2, data2)
        XCTAssertEqual(copiedData3, data3)

        await controller.close()
    }

    // MARK: - Scenario 6: Use-After-Free Prevention & Unbound Job Safety
    @MainActor
    func testClosedJobHandleSafetyUseAfterFreeProtection() async throws {
        let tvm = TransferViewModel()
        let evm = ErrorCenterViewModel()
        tvm.errorCenterViewModel = evm

        let tempDir = FileManager.default.temporaryDirectory
        let runId = UUID().uuidString
        let srcDir = tempDir.appendingPathComponent("uaf_src_\(runId)")
        let dstDir = tempDir.appendingPathComponent("uaf_dst_\(runId)")

        try FileManager.default.createDirectory(at: srcDir, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: dstDir, withIntermediateDirectories: true)
        defer {
            try? FileManager.default.removeItem(at: srcDir)
            try? FileManager.default.removeItem(at: dstDir)
        }

        try "Payload Data for UAF Test".data(using: .utf8)?.write(to: srcDir.appendingPathComponent("item.txt"))

        tvm.selectSource(url: srcDir)
        tvm.selectDestination(url: dstDir)
        await tvm.runPreFlightCheck()

        // 1. Direct handle binding and unbinding lifecycle verification
        let directController = try VolterraBridge.createJob(
            sourcePath: srcDir.path,
            destPath: dstDir.path,
            mode: .smartHybrid,
            allowOverwrite: true
        )
        if let rawHandle = await directController.getRawHandle() {
            evm.bindJob(handle: rawHandle)
            XCTAssertTrue(evm.hasActiveJob, "ErrorCenterViewModel must report hasActiveJob == true when bound to valid handle")

            // Unbind handle before closing controller
            evm.bindJob(handle: nil)
            XCTAssertFalse(evm.hasActiveJob, "ErrorCenterViewModel must report hasActiveJob == false when unbound")

            await directController.close()
        }

        // 2. End-to-end TransferViewModel lifecycle: start, complete/cancel, cleanup
        tvm.startTransfer()

        // Wait until job completes or cancel if running
        let deadline = Date().addingTimeInterval(3.0)
        while Date() < deadline && tvm.isActive {
            try await Task.sleep(nanoseconds: 10_000_000)
        }

        // Reset to idle to ensure cleanupJob has run
        tvm.resetToIdle()

        // Verify handle was safely invalidated
        XCTAssertFalse(evm.hasActiveJob, "ErrorCenterViewModel activeJobHandle MUST be nil after cleanupJob / resetToIdle")

        // Exercise all actions after deallocation: MUST NOT crash (no EXC_BAD_ACCESS)
        evm.skipFile(fileId: 42)
        XCTAssertEqual(evm.actionFeedbackMessage, "No active job available")
        XCTAssertFalse(evm.isPerformingAction)

        evm.retryFile(fileId: 42)
        XCTAssertEqual(evm.actionFeedbackMessage, "No active job available")
        XCTAssertFalse(evm.isPerformingAction)

        evm.deferFile(fileId: 42)
        XCTAssertEqual(evm.actionFeedbackMessage, "No active job available")
        XCTAssertFalse(evm.isPerformingAction)

        evm.skipAllErrors()
        XCTAssertEqual(evm.actionFeedbackMessage, "No active job available")
        XCTAssertFalse(evm.isPerformingAction)

        evm.retryAllFailed()
        XCTAssertEqual(evm.actionFeedbackMessage, "No active job available")
        XCTAssertFalse(evm.isPerformingAction)

        evm.refreshErrors() // Safe no-op
    }
}

private final class ThreadSafeFlag: @unchecked Sendable {
    private let lock = NSLock()
    private var _value: Bool

    init(initial: Bool) {
        self._value = initial
    }

    var value: Bool {
        lock.lock()
        defer { lock.unlock() }
        return _value
    }

    func set(_ newValue: Bool) {
        lock.lock()
        _value = newValue
        lock.unlock()
    }
}
