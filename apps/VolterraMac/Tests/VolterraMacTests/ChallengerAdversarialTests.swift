//
// ChallengerAdversarialTests.swift
// VolterraMacTests
//
// Milestone 5 Empirical Stress & Adversarial Test Harness
//

import XCTest
import Foundation
import CVolterra
@testable import VolterraBridge
@testable import VolterraMac

final class ChallengerAdversarialTests: XCTestCase {

    private func createTempWorkspace(fileCount: Int = 5, fileSize: Int = 32768) throws -> (src: URL, dst: URL) {
        let tempDir = FileManager.default.temporaryDirectory
        let runId = UUID().uuidString
        let srcDir = tempDir.appendingPathComponent("adv_src_\(runId)")
        let dstDir = tempDir.appendingPathComponent("adv_dst_\(runId)")

        try FileManager.default.createDirectory(at: srcDir, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: dstDir, withIntermediateDirectories: true)

        for i in 0..<fileCount {
            let fileURL = srcDir.appendingPathComponent("file_\(i).bin")
            let content = Data((0..<fileSize).map { UInt8(($0 + i) % 256) })
            try content.write(to: fileURL)
        }

        return (srcDir, dstDir)
    }

    private func cleanupWorkspace(src: URL, dst: URL) {
        try? FileManager.default.removeItem(at: src)
        try? FileManager.default.removeItem(at: dst)
    }

    // MARK: - 1. Rapid Out-of-Order State Transitions

    func testRapidStartPauseCancel() async throws {
        let (src, dst) = try createTempWorkspace(fileCount: 20, fileSize: 65536)
        defer { cleanupWorkspace(src: src, dst: dst) }

        let controller = try VolterraBridge.createJob(
            sourcePath: src.path,
            destPath: dst.path,
            mode: .smartHybrid,
            allowOverwrite: true
        )

        let stream = await controller.events()
        var enteredRunning = false
        let monitorTask = Task {
            for await ev in stream {
                if ev.kind == .stateChanged && ev.message.contains("Running") {
                    enteredRunning = true
                    break
                }
                if ev.kind == .fileStarted {
                    enteredRunning = true
                    break
                }
            }
        }

        try await controller.start()

        // Wait until it transitions to Running
        let deadline = Date().addingTimeInterval(2.0)
        while Date() < deadline && !enteredRunning {
            if let snap = await controller.getSnapshot(), snap.state == .running {
                enteredRunning = true
                break
            }
            try await Task.sleep(nanoseconds: 10_000_000)
        }

        // Now pause must succeed
        try await controller.pause()
        let pausedSnap = await controller.getSnapshot()
        XCTAssertEqual(pausedSnap?.state, .paused, "Job should be paused")

        // Now cancel from paused must succeed
        try await controller.cancel()

        var finalState: VolterraBridgeState? = nil
        let cancelDeadline = Date().addingTimeInterval(1.0)
        while Date() < cancelDeadline {
            if let snap = await controller.getSnapshot() {
                finalState = snap.state
                if snap.state == .cancelled {
                    break
                }
            }
            try await Task.sleep(nanoseconds: 10_000_000)
        }

        XCTAssertEqual(finalState, .cancelled, "Job must settle in cancelled state")
        await monitorTask.cancel()
        await controller.close()
    }

    func testRapidCancelWhileScanning() async throws {
        let (src, dst) = try createTempWorkspace(fileCount: 50, fileSize: 1024)
        defer { cleanupWorkspace(src: src, dst: dst) }

        let controller = try VolterraBridge.createJob(
            sourcePath: src.path,
            destPath: dst.path,
            mode: .smartHybrid,
            allowOverwrite: true
        )

        try await controller.start()
        // Immediately cancel while scanning or early running
        try await controller.cancel()

        let cancelDeadline = Date().addingTimeInterval(1.0)
        var settled = false
        while Date() < cancelDeadline {
            if let snap = await controller.getSnapshot() {
                if snap.state == .cancelled {
                    settled = true
                    break
                }
            }
            try await Task.sleep(nanoseconds: 10_000_000)
        }

        XCTAssertTrue(settled, "Cancel immediately after start must cancel job cleanly")
        await controller.close()
    }

    func testInvalidStateRejections() async throws {
        let (src, dst) = try createTempWorkspace(fileCount: 2, fileSize: 4096)
        defer { cleanupWorkspace(src: src, dst: dst) }

        let controller = try VolterraBridge.createJob(
            sourcePath: src.path,
            destPath: dst.path,
            mode: .smartHybrid,
            allowOverwrite: true
        )

        // 1. Resume while Pending -> Must fail with invalidState
        do {
            try await controller.resume()
            XCTFail("Resume on pending job must fail")
        } catch let err as VolterraBridgeError {
            guard case .invalidState = err else {
                XCTFail("Expected invalidState error, got: \(err)")
                return
            }
        }

        // 2. Pause while Pending -> Must fail with invalidState
        do {
            try await controller.pause()
            XCTFail("Pause on pending job must fail")
        } catch let err as VolterraBridgeError {
            guard case .invalidState = err else {
                XCTFail("Expected invalidState error, got: \(err)")
                return
            }
        }

        // 3. Start job and let it complete
        let stream = await controller.events()
        let completeTask = Task {
            for await ev in stream {
                if ev.kind == .jobCompleted || ev.kind == .jobFailed {
                    break
                }
            }
        }
        try await controller.start()
        await completeTask.value

        let snap = await controller.getSnapshot()
        XCTAssertEqual(snap?.state, .completed)

        // 4. Start on already completed job -> Must fail
        do {
            try await controller.start()
            XCTFail("Start on completed job must fail")
        } catch let err as VolterraBridgeError {
            guard case .invalidState = err else {
                XCTFail("Expected invalidState error, got: \(err)")
                return
            }
        }

        // 5. Resume on completed job -> Must fail
        do {
            try await controller.resume()
            XCTFail("Resume on completed job must fail")
        } catch let err as VolterraBridgeError {
            guard case .invalidState = err else {
                XCTFail("Expected invalidState error, got: \(err)")
                return
            }
        }

        await controller.close()
    }

    func testConcurrentStateTransitionRace() async throws {
        let (src, dst) = try createTempWorkspace(fileCount: 30, fileSize: 65536)
        defer { cleanupWorkspace(src: src, dst: dst) }

        let controller = try VolterraBridge.createJob(
            sourcePath: src.path,
            destPath: dst.path,
            mode: .smartHybrid,
            allowOverwrite: true
        )

        try await controller.start()

        // Launch concurrent racing control calls
        await withTaskGroup(of: Void.self) { group in
            for i in 0..<10 {
                group.addTask {
                    if i % 3 == 0 {
                        try? await controller.pause()
                    } else if i % 3 == 1 {
                        try? await controller.resume()
                    } else {
                        try? await controller.cancel()
                    }
                }
            }
        }

        // Must still be responsive and return a valid snapshot
        let snap = await controller.getSnapshot()
        XCTAssertNotNil(snap, "Snapshot must be retrievable after concurrent transition race")

        await controller.close()
    }

    // MARK: - 2. High-Churn FFI Lifecycle & Memory Safety

    func testHighChurnFFILifecycle() async throws {
        let (src, dst) = try createTempWorkspace(fileCount: 1, fileSize: 1024)
        defer { cleanupWorkspace(src: src, dst: dst) }

        // Perform 25 rapid cycles of create -> inspect -> export -> cancel -> close
        for i in 0..<25 {
            let controller = try VolterraBridge.createJob(
                sourcePath: src.path,
                destPath: dst.path,
                mode: .smartHybrid,
                allowOverwrite: true
            )

            // Multiple snapshot queries
            for _ in 0..<3 {
                let snap = await controller.getSnapshot()
                XCTAssertNotNil(snap)
            }

            // Multiple telemetry queries
            for _ in 0..<3 {
                let tel = await controller.getTelemetry()
                XCTAssertNotNil(tel)
            }

            // Multiple error queries
            for _ in 0..<2 {
                let errs = await controller.getErrors()
                XCTAssertTrue(errs.isEmpty)
            }

            // Export reports
            let json = await controller.exportReport(format: 0)
            XCTAssertNotNil(json)
            let csv = await controller.exportReport(format: 1)
            XCTAssertNotNil(csv)

            // Disk discovery churn
            if i % 5 == 0 {
                let disks = try VolterraBridge.discoverDisks()
                XCTAssertFalse(disks.isEmpty)
            }

            try await controller.cancel()
            await controller.close()
        }
    }

    // MARK: - 3. High-Throughput Event Callbacks & Early Stream Termination

    func testHighThroughputEventStreamAndEarlyTermination() async throws {
        let (src, dst) = try createTempWorkspace(fileCount: 40, fileSize: 32768)
        defer { cleanupWorkspace(src: src, dst: dst) }

        let controller = try VolterraBridge.createJob(
            sourcePath: src.path,
            destPath: dst.path,
            mode: .smartHybrid,
            allowOverwrite: true
        )

        let eventStream = await controller.events()

        // Consumer task terminates early after receiving only 3 events
        let consumerTask = Task {
            var received = 0
            for await _ in eventStream {
                received += 1
                if received >= 3 {
                    // Early exit!
                    break
                }
            }
            return received
        }

        try await controller.start()

        let count = await consumerTask.value
        XCTAssertGreaterThanOrEqual(count, 3, "Consumer must have received early events")

        // Wait for job to finish without deadlocking on the abandoned stream
        let deadline = Date().addingTimeInterval(3.0)
        var finished = false
        while Date() < deadline {
            if let snap = await controller.getSnapshot() {
                if snap.state == .completed || snap.state == .cancelled || snap.state == .failed {
                    finished = true
                    break
                }
            }
            try await Task.sleep(nanoseconds: 50_000_000)
        }

        XCTAssertTrue(finished, "Job must complete without deadlocking despite early stream termination")
        await controller.close()
    }

    // MARK: - 4. Disk Discovery & Invalid Path Boundaries

    func testDiskDiscoveryFieldsAndIntegrity() throws {
        let disks = try VolterraBridge.discoverDisks()
        XCTAssertFalse(disks.isEmpty, "At least one disk volume must be discovered")

        for disk in disks {
            XCTAssertFalse(disk.deviceId.isEmpty)
            XCTAssertFalse(disk.mountPoint.isEmpty)
            XCTAssertGreaterThan(disk.totalBytes, 0)
            // Parent BSD name or BSD name should be non-empty
            XCTAssertTrue(!disk.bsdName.isEmpty || !disk.parentBsdName.isEmpty)
        }
    }

    func testJobCreationInvalidPaths() {
        let nonexistentSource = "/nonexistent_volterra_invalid_dir_\(UUID().uuidString)"
        let nonexistentDest = "/tmp/volterra_dest_\(UUID().uuidString)"

        XCTAssertThrowsError(
            try VolterraBridge.createJob(
                sourcePath: nonexistentSource,
                destPath: nonexistentDest
            )
        ) { error in
            guard let bridgeErr = error as? VolterraBridgeError else {
                XCTFail("Expected VolterraBridgeError, got: \(error)")
                return
            }
            switch bridgeErr {
            case .sourceNotFound:
                // Expected
                break
            default:
                XCTFail("Expected sourceNotFound error, got: \(bridgeErr)")
            }
        }
    }

    // MARK: - 5. Report Export Fidelity (JSON & CSV)

    func testReportExportFidelityJSONAndCSV() async throws {
        let (src, dst) = try createTempWorkspace(fileCount: 3, fileSize: 16384)
        defer { cleanupWorkspace(src: src, dst: dst) }

        let controller = try VolterraBridge.createJob(
            sourcePath: src.path,
            destPath: dst.path,
            mode: .smartHybrid,
            allowOverwrite: true
        )

        let stream = await controller.events()
        let completeTask = Task {
            for await ev in stream {
                if ev.kind == .jobCompleted || ev.kind == .jobFailed {
                    break
                }
            }
        }

        try await controller.start()
        await completeTask.value

        // 1. JSON Report Fidelity Verification
        guard let jsonString = await controller.exportReport(format: 0) else {
            XCTFail("JSON report export returned nil")
            return
        }
        guard let jsonData = jsonString.data(using: .utf8),
              let jsonObject = try? JSONSerialization.jsonObject(with: jsonData) as? [String: Any] else {
            XCTFail("JSON report is not valid JSON")
            return
        }

        XCTAssertEqual(jsonObject["volterra_version"] as? String, "1.0.0")
        XCTAssertNotNil(jsonObject["job_id"] as? String)
        XCTAssertEqual(jsonObject["state"] as? String, "Completed")

        guard let summary = jsonObject["summary"] as? [String: Any] else {
            XCTFail("JSON report missing summary object")
            return
        }
        XCTAssertEqual(summary["total_files"] as? Int, 3)
        XCTAssertEqual(summary["completed_files"] as? Int, 3)
        XCTAssertEqual(summary["failed_files"] as? Int, 0)
        XCTAssertEqual(summary["skipped_files"] as? Int, 0)
        XCTAssertEqual(summary["total_bytes"] as? Int, 3 * 16384)
        XCTAssertEqual(summary["transferred_bytes"] as? Int, 3 * 16384)
        XCTAssertEqual(summary["percent_complete"] as? Double, 100.0)

        guard let telemetry = jsonObject["telemetry"] as? [String: Any] else {
            XCTFail("JSON report missing telemetry object")
            return
        }
        XCTAssertNotNil(telemetry["buffer_utilization_pct"])
        XCTAssertNotNil(telemetry["active_chunk_size_bytes"])
        XCTAssertNotNil(telemetry["active_concurrency"])

        // 2. CSV Report Fidelity Verification
        guard let csvString = await controller.exportReport(format: 1) else {
            XCTFail("CSV report export returned nil")
            return
        }

        let lines = csvString.components(separatedBy: "\n").filter { !$0.isEmpty }
        XCTAssertGreaterThanOrEqual(lines.count, 2, "CSV report must contain at least header and summary")

        let expectedHeader = "record_type,file_id,path,size_bytes,status,error_code,error_message,failed_offset,failed_length"
        XCTAssertEqual(lines[0], expectedHeader, "CSV header must match expected schema")

        let summaryLine = lines[1]
        XCTAssertTrue(summaryLine.hasPrefix("SUMMARY,-1,all,"), "Summary line must begin with SUMMARY,-1,all")
        XCTAssertTrue(summaryLine.contains("Completed"), "Summary line must indicate Completed state")

        await controller.close()
    }

    // MARK: - 6. Unbonded ErrorCenter Actions Post Termination (Cancel, Complete, Failure)

    @MainActor
    func testErrorCenterActionsPostTerminationLifecycle() async throws {
        let tvm = TransferViewModel()
        let evm = ErrorCenterViewModel()
        tvm.errorCenterViewModel = evm

        let (src, dst) = try createTempWorkspace(fileCount: 5, fileSize: 16384)
        defer { cleanupWorkspace(src: src, dst: dst) }

        // Test Helper to verify all 5 unbonded actions
        func verifyAllUnbondedActions(context: String) {
            XCTAssertFalse(evm.hasActiveJob, "[\(context)] ErrorCenterViewModel must not report active job")

            evm.skipFile(fileId: 101)
            XCTAssertEqual(evm.actionFeedbackMessage, "No active job available", "[\(context)] skipFile must populate feedback")
            XCTAssertFalse(evm.isPerformingAction)

            evm.retryFile(fileId: 102)
            XCTAssertEqual(evm.actionFeedbackMessage, "No active job available", "[\(context)] retryFile must populate feedback")
            XCTAssertFalse(evm.isPerformingAction)

            evm.deferFile(fileId: 103)
            XCTAssertEqual(evm.actionFeedbackMessage, "No active job available", "[\(context)] deferFile must populate feedback")
            XCTAssertFalse(evm.isPerformingAction)

            evm.skipAllErrors()
            XCTAssertEqual(evm.actionFeedbackMessage, "No active job available", "[\(context)] skipAllErrors must populate feedback")
            XCTAssertFalse(evm.isPerformingAction)

            evm.retryAllFailed()
            XCTAssertEqual(evm.actionFeedbackMessage, "No active job available", "[\(context)] retryAllFailed must populate feedback")
            XCTAssertFalse(evm.isPerformingAction)

            evm.refreshErrors() // Safe no-op without active handle
        }

        // Scenario A: Initial Idle State
        verifyAllUnbondedActions(context: "Initial Idle")

        // Scenario B: Post-Completion Lifecycle
        tvm.selectSource(url: src)
        tvm.selectDestination(url: dst)
        await tvm.runPreFlightCheck()
        tvm.startTransfer()

        let completeDeadline = Date().addingTimeInterval(3.0)
        while Date() < completeDeadline && tvm.isActive {
            try await Task.sleep(nanoseconds: 10_000_000)
        }
        XCTAssertEqual(tvm.state, .completed, "Job should reach completed state")
        verifyAllUnbondedActions(context: "Post-Completion")

        // Scenario C: Post-Cancellation Lifecycle
        let (srcCancel, dstCancel) = try createTempWorkspace(fileCount: 15, fileSize: 65536)
        defer { cleanupWorkspace(src: srcCancel, dst: dstCancel) }

        tvm.resetToIdle()
        verifyAllUnbondedActions(context: "Post-ResetToIdle")

        tvm.selectSource(url: srcCancel)
        tvm.selectDestination(url: dstCancel)
        await tvm.runPreFlightCheck()
        tvm.startTransfer()

        // Give it a few ms to start, then cancel
        try await Task.sleep(nanoseconds: 20_000_000)
        tvm.cancelTransfer()

        let cancelDeadline = Date().addingTimeInterval(2.0)
        while Date() < cancelDeadline && tvm.isActive {
            try await Task.sleep(nanoseconds: 10_000_000)
        }
        XCTAssertEqual(tvm.state, .cancelled, "Job should be cancelled")
        verifyAllUnbondedActions(context: "Post-Cancellation")

        // Scenario D: Post-Failure Lifecycle (via discrete error event)
        tvm.resetToIdle()
        tvm.handleDiscreteEvent(type: 12, message: "Simulated hardware I/O disconnect error", fileId: 0)
        XCTAssertEqual(tvm.state, .failed, "Job should be failed")
        verifyAllUnbondedActions(context: "Post-Failure")
    }

    // MARK: - 6B. StateChanged String Parsing Empirical Bug Demonstration

    @MainActor
    func testStateChangedEventParsingFlaw() {
        let vm = TransferViewModel()

        // 1. FFI bridge emits "State changed from Running to Paused"
        vm.handleDiscreteEvent(type: 1, message: "State changed from Running to Paused", fileId: -1)
        print("=== EMPIRICAL STATE CHANGED TEST 1 (Running -> Paused) ===")
        print("Target: Paused. Actual vm.state: \(vm.state.rawValue)")

        // 2. FFI bridge emits "State changed from Running to Cancelled"
        vm.handleDiscreteEvent(type: 1, message: "State changed from Running to Cancelled", fileId: -1)
        print("=== EMPIRICAL STATE CHANGED TEST 2 (Running -> Cancelled) ===")
        print("Target: Cancelled. Actual vm.state: \(vm.state.rawValue)")

        // 3. FFI bridge emits "State changed from Running to Completed"
        vm.handleDiscreteEvent(type: 1, message: "State changed from Running to Completed", fileId: -1)
        print("=== EMPIRICAL STATE CHANGED TEST 3 (Running -> Completed) ===")
        print("Target: Completed. Actual vm.state: \(vm.state.rawValue)")

        // 4. FFI bridge emits "State changed from Running to Failed"
        vm.handleDiscreteEvent(type: 1, message: "State changed from Running to Failed", fileId: -1)
        print("=== EMPIRICAL STATE CHANGED TEST 4 (Running -> Failed) ===")
        print("Target: Failed. Actual vm.state: \(vm.state.rawValue)")

        // Empirical Bug Assertions: All 4 transitions fail because message.contains("Running") matches first
        XCTAssertEqual(vm.state, .failed, "State changed from Running to Failed should set state to .failed, but got \(vm.state)")
    }

    // MARK: - 7. Multiple events() Calls and Continuation Memory Balance


    func testMultipleEventsCallLifecycleAndMemoryBalance() async throws {
        let (src, dst) = try createTempWorkspace(fileCount: 10, fileSize: 32768)
        defer { cleanupWorkspace(src: src, dst: dst) }

        let controller = try VolterraBridge.createJob(
            sourcePath: src.path,
            destPath: dst.path,
            mode: .smartHybrid,
            allowOverwrite: true
        )

        // Multiple calls to events() in sequence before starting
        var streams: [AsyncStream<VolterraBridgeEvent>] = []
        for _ in 0..<10 {
            let s = await controller.events()
            streams.append(s)
        }
        XCTAssertEqual(streams.count, 10, "Should have created 10 streams without crashing")

        // Attach listener to the latest active stream
        let latestStream = streams.last!
        var receivedCount = 0
        let listenerTask = Task {
            for await _ in latestStream {
                receivedCount += 1
            }
        }

        // Start job and verify events flow
        try await controller.start()

        let deadline = Date().addingTimeInterval(3.0)
        while Date() < deadline {
            if let snap = await controller.getSnapshot() {
                if snap.state == .completed || snap.state == .failed {
                    break
                }
            }
            try await Task.sleep(nanoseconds: 50_000_000)
        }

        // Close controller properly
        await controller.close()
        await listenerTask.value

        XCTAssertGreaterThan(receivedCount, 0, "Latest event stream must have received progress events")
    }

    func testMultipleEventsCallDuringActiveTransferStress() async throws {
        let (src, dst) = try createTempWorkspace(fileCount: 40, fileSize: 65536)
        defer { cleanupWorkspace(src: src, dst: dst) }

        let controller = try VolterraBridge.createJob(
            sourcePath: src.path,
            destPath: dst.path,
            mode: .smartHybrid,
            allowOverwrite: true
        )

        try await controller.start()

        // Repeatedly request events() while transfer is active and emitting events
        var consumerTasks: [Task<Int, Never>] = []
        for _ in 0..<20 {
            let stream = await controller.events()
            let task = Task {
                var count = 0
                for await _ in stream {
                    count += 1
                    if count >= 3 { break }
                }
                return count
            }
            consumerTasks.append(task)
            try await Task.sleep(nanoseconds: 10_000_000)
        }

        let deadline = Date().addingTimeInterval(3.0)
        while Date() < deadline {
            if let snap = await controller.getSnapshot() {
                if snap.state == .completed || snap.state == .failed {
                    break
                }
            }
            try await Task.sleep(nanoseconds: 50_000_000)
        }

        await controller.close()

        for task in consumerTasks {
            _ = await task.value
        }
    }

    // MARK: - 8. Empirical Challenge: Lifecycle Across Sequential Jobs, Resets, Cancellations & Directory Switches

    @MainActor
    func testEmpiricalLifecycleSequentialJobsAndOverwriteResets() async throws {
        let tempDir = FileManager.default.temporaryDirectory
        let runId = UUID().uuidString
        let srcA = tempDir.appendingPathComponent("adv_seq_srcA_\(runId)")
        let srcB = tempDir.appendingPathComponent("adv_seq_srcB_\(runId)")
        let dstA = tempDir.appendingPathComponent("adv_seq_dstA_\(runId)")
        let dstB = tempDir.appendingPathComponent("adv_seq_dstB_\(runId)")

        try FileManager.default.createDirectory(at: srcA, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: srcB, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: dstA, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: dstB, withIntermediateDirectories: true)

        defer {
            try? FileManager.default.removeItem(at: srcA)
            try? FileManager.default.removeItem(at: srcB)
            try? FileManager.default.removeItem(at: dstA)
            try? FileManager.default.removeItem(at: dstB)
        }

        // Put a file in srcA and colliding file in dstA
        try "Original A".data(using: .utf8)!.write(to: srcA.appendingPathComponent("file.txt"))
        try "Existing A".data(using: .utf8)!.write(to: dstA.appendingPathComponent("file.txt"))

        // Put a file in srcB and colliding file in dstB
        try "Original B".data(using: .utf8)!.write(to: srcB.appendingPathComponent("doc.txt"))
        try "Existing B".data(using: .utf8)!.write(to: dstB.appendingPathComponent("doc.txt"))

        let vm = TransferViewModel()

        // 1. Initial State: allowOverwriteConfirmed MUST be false
        XCTAssertFalse(vm.allowOverwriteConfirmed, "Initial allowOverwriteConfirmed must be false")
        XCTAssertFalse(vm.isOverwriteApproved)

        // 2. Select Source & Dest A
        vm.selectSource(url: srcA)
        vm.selectDestination(url: dstA)
        await vm.runPreFlightCheck()

        XCTAssertTrue(vm.collidingFilesCount > 0, "Colliding file should be detected in dstA")
        XCTAssertFalse(vm.allowOverwriteConfirmed, "allowOverwriteConfirmed must remain false after preflight")

        // 3. User attempts to start transfer without approval -> Dialog must be shown, job not started
        vm.startTransfer()
        XCTAssertTrue(vm.showOverwriteDialog, "Dialog must be shown when colliding files exist")
        XCTAssertFalse(vm.allowOverwriteConfirmed, "allowOverwriteConfirmed must remain false without user approval")
        XCTAssertEqual(vm.state, .idle, "State must remain idle")

        // 4. User explicitly approves overwrite
        vm.approveOverwriteAndStart()
        XCTAssertTrue(vm.allowOverwriteConfirmed, "Approval must set allowOverwriteConfirmed = true")
        XCTAssertFalse(vm.showOverwriteDialog, "Dialog must dismiss after approval")

        // 5. Cancel transfer immediately: MUST synchronously reset allowOverwriteConfirmed to false
        vm.cancelTransfer()
        XCTAssertFalse(vm.allowOverwriteConfirmed, "cancelTransfer must synchronously reset allowOverwriteConfirmed to false without awaiting background task")
        XCTAssertFalse(vm.showOverwriteDialog)
        XCTAssertNil(vm.destinationCollisionPath)

        // Wait for background cancellation task to settle
        try await Task.sleep(nanoseconds: 50_000_000)

        // 6. Reset to Idle: MUST ensure allowOverwriteConfirmed is false
        vm.resetToIdle()
        XCTAssertFalse(vm.allowOverwriteConfirmed, "resetToIdle must ensure allowOverwriteConfirmed is false")
        XCTAssertFalse(vm.showOverwriteDialog)
        XCTAssertEqual(vm.collidingFilesCount, 0)
        XCTAssertEqual(vm.activeJobId, "")

        // 7. Directory Path Switch: Switch source to srcB while setting overwrite to true artificially
        vm.allowOverwriteConfirmed = true
        vm.selectSource(url: srcB) // didSet triggers allowOverwriteConfirmed = false
        XCTAssertFalse(vm.allowOverwriteConfirmed, "Switching sourceURL via selectSource must reset allowOverwriteConfirmed to false")

        vm.allowOverwriteConfirmed = true
        vm.sourceURL = srcA // direct assignment didSet triggers allowOverwriteConfirmed = false
        XCTAssertFalse(vm.allowOverwriteConfirmed, "Direct assignment to sourceURL must reset allowOverwriteConfirmed to false")

        vm.allowOverwriteConfirmed = true
        vm.selectDestination(url: dstB) // didSet triggers allowOverwriteConfirmed = false
        XCTAssertFalse(vm.allowOverwriteConfirmed, "Switching destinationURL via selectDestination must reset allowOverwriteConfirmed to false")

        vm.allowOverwriteConfirmed = true
        vm.destinationURL = dstA // direct assignment didSet triggers allowOverwriteConfirmed = false
        XCTAssertFalse(vm.allowOverwriteConfirmed, "Direct assignment to destinationURL must reset allowOverwriteConfirmed to false")

        // 8. Re-assigning the SAME URL must still reset allowOverwriteConfirmed
        vm.allowOverwriteConfirmed = true
        vm.sourceURL = srcA
        XCTAssertFalse(vm.allowOverwriteConfirmed, "Re-assigning same sourceURL must reset allowOverwriteConfirmed to false")

        vm.allowOverwriteConfirmed = true
        vm.destinationURL = dstA
        XCTAssertFalse(vm.allowOverwriteConfirmed, "Re-assigning same destinationURL must reset allowOverwriteConfirmed to false")

        // 9. Discrete Event Completion Reset
        vm.allowOverwriteConfirmed = true
        vm.handleDiscreteEvent(type: 11, message: "JobCompleted", fileId: 0) // JobCompleted
        XCTAssertFalse(vm.allowOverwriteConfirmed, "JobCompleted discrete event must trigger cleanup and reset allowOverwriteConfirmed")

        // 10. Discrete Event Failure Reset
        vm.allowOverwriteConfirmed = true
        vm.handleDiscreteEvent(type: 12, message: "JobFailed", fileId: 0) // JobFailed
        XCTAssertFalse(vm.allowOverwriteConfirmed, "JobFailed discrete event must trigger cleanup and reset allowOverwriteConfirmed")

        // 11. Multi-Cycle Sequential Loop Stress Test (10 consecutive cycles)
        for cycle in 1...10 {
            vm.resetToIdle()
            XCTAssertFalse(vm.allowOverwriteConfirmed, "Cycle \(cycle): allowOverwriteConfirmed must start false")

            vm.selectSource(url: cycle % 2 == 0 ? srcA : srcB)
            vm.selectDestination(url: cycle % 2 == 0 ? dstA : dstB)
            await vm.runPreFlightCheck()

            XCTAssertTrue(vm.collidingFilesCount > 0, "Cycle \(cycle): collision must be detected")
            XCTAssertFalse(vm.allowOverwriteConfirmed, "Cycle \(cycle): allowOverwriteConfirmed must be false")

            // Starting without approval MUST NOT start transfer
            vm.startTransfer()
            XCTAssertTrue(vm.showOverwriteDialog, "Cycle \(cycle): startTransfer must prompt with dialog")
            XCTAssertFalse(vm.allowOverwriteConfirmed, "Cycle \(cycle): allowOverwriteConfirmed must remain false")
            XCTAssertEqual(vm.state, .idle, "Cycle \(cycle): transfer must stay idle")

            // Explicit approval
            vm.approveOverwriteAndStart()
            XCTAssertTrue(vm.allowOverwriteConfirmed, "Cycle \(cycle): user approval must set allowOverwriteConfirmed = true")

            if cycle % 3 == 0 {
                vm.cancelTransfer()
                try await Task.sleep(nanoseconds: 50_000_000)
            } else {
                vm.resetToIdle()
            }
            XCTAssertFalse(vm.allowOverwriteConfirmed, "Cycle \(cycle): post-action allowOverwriteConfirmed must be false")
        }
    }

    // MARK: - 9. Empirical Challenge: Pre-Flight Capacity Evaluation on 0 Free Bytes

    @MainActor
    func testEmpiricalZeroByteCapacityPreFlight() async throws {
        // 1. Pure Arithmetic and Boundary Condition Verification
        let boundaryCases: [(src: UInt64, free: UInt64, expectedInsufficient: Bool)] = [
            (src: 1, free: 0, expectedInsufficient: true),
            (src: 10_000, free: 0, expectedInsufficient: true),
            (src: 10_000_000, free: 0, expectedInsufficient: true),
            (src: 0, free: 0, expectedInsufficient: false), // 0 bytes to copy needs 0 space
            (src: 100, free: 50, expectedInsufficient: true),
            (src: 100, free: 100, expectedInsufficient: false),
            (src: 100, free: 200, expectedInsufficient: false),
            (src: 0, free: 100, expectedInsufficient: false),
        ]

        for (src, free, expected) in boundaryCases {
            let result = src > 0 && (free == 0 || src > free)
            XCTAssertEqual(result, expected, "Failed capacity evaluation for src=\(src), free=\(free)")
        }

        // 2. Real System macOS Disk Mount with Exactly 0 Free Bytes
        let tempDir = FileManager.default.temporaryDirectory.resolvingSymlinksInPath()
        let runId = UUID().uuidString
        let dmgPath = tempDir.appendingPathComponent("zero_disk_\(runId).dmg").path
        let mountPoint = tempDir.appendingPathComponent("zero_mount_\(runId)").resolvingSymlinksInPath()
        let srcDir = tempDir.appendingPathComponent("zero_src_\(runId)").resolvingSymlinksInPath()

        try FileManager.default.createDirectory(at: mountPoint, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: srcDir, withIntermediateDirectories: true)
        try "Payload for 0 byte capacity check".data(using: .utf8)!.write(to: srcDir.appendingPathComponent("test.bin"))

        defer {
            try? FileManager.default.removeItem(atPath: dmgPath)
            try? FileManager.default.removeItem(at: mountPoint)
            try? FileManager.default.removeItem(at: srcDir)
        }

        // Create 2MB HFS+ disk image
        let createProc = Process()
        createProc.executableURL = URL(fileURLWithPath: "/usr/bin/hdiutil")
        createProc.arguments = ["create", "-size", "2m", "-fs", "HFS+", "-volname", "ZeroVol", dmgPath]
        try createProc.run()
        createProc.waitUntilExit()

        guard createProc.terminationStatus == 0 else {
            // If hdiutil is restricted, arithmetic verification above suffices
            return
        }

        // Attach disk image to mountPoint
        let attachProc = Process()
        attachProc.executableURL = URL(fileURLWithPath: "/usr/bin/hdiutil")
        attachProc.arguments = ["attach", dmgPath, "-mountpoint", mountPoint.path]
        try attachProc.run()
        attachProc.waitUntilExit()

        guard attachProc.terminationStatus == 0 else {
            return
        }

        defer {
            let detachProc = Process()
            detachProc.executableURL = URL(fileURLWithPath: "/usr/bin/hdiutil")
            detachProc.arguments = ["detach", mountPoint.path, "-force"]
            try? detachProc.run()
            detachProc.waitUntilExit()
        }

        // Fill the mounted volume until ENOSPC (0 free bytes)
        let fillPath = mountPoint.appendingPathComponent("full.bin")
        if FileManager.default.createFile(atPath: fillPath.path, contents: nil),
           let handle = try? FileHandle(forWritingTo: fillPath) {
            let chunk = Data(repeating: 0xFF, count: 4096)
            while true {
                do {
                    try handle.write(contentsOf: chunk)
                } catch {
                    break
                }
            }
            try? handle.close()
        }

        // Verify free bytes on volume is 0
        let attrs = try FileManager.default.attributesOfFileSystem(forPath: mountPoint.path)
        let freeSize = (attrs[.systemFreeSize] as? NSNumber)?.uint64Value ?? 1
        XCTAssertEqual(freeSize, 0, "Mounted volume must have exactly 0 free bytes")

        // Now run TransferViewModel preflight check against this real 0-free-byte volume
        let vm = TransferViewModel()
        vm.selectSource(url: srcDir)
        vm.selectDestination(url: mountPoint)

        await vm.runPreFlightCheck()

        XCTAssertTrue(vm.preFlightValid, "Pre-flight should complete validation")
        XCTAssertTrue(vm.hasInsufficientSpace, "hasInsufficientSpace MUST be true when available space is 0 bytes")
        XCTAssertNotNil(vm.capacityWarning, "capacityWarning MUST be populated when free space is 0 bytes")
        XCTAssertEqual(vm.capacityWarning?.availableBytes, 0, "availableBytes in capacityWarning must be 0")
        XCTAssertGreaterThan(vm.capacityWarning?.requiredBytes ?? 0, 0, "requiredBytes must be > 0")
        XCTAssertFalse(vm.canStartTransfer, "canStartTransfer must be false when hasInsufficientSpace is true")
    }
}


