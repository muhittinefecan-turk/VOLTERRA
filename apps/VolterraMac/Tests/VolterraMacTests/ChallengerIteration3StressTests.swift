//
// ChallengerIteration3StressTests.swift
// VolterraMacTests
//
// Empirical Stress Test Harness by Challenger 1 for Milestone 5 Iteration 3
//

import XCTest
import Foundation
import CVolterra
@testable import VolterraBridge
@testable import VolterraMac

@MainActor
final class ChallengerIteration3StressTests: XCTestCase {

    private func createTempWorkspace(fileCount: Int = 5, fileSize: Int = 32768) throws -> (src: URL, dst: URL) {
        let tempDir = FileManager.default.temporaryDirectory.resolvingSymlinksInPath()
        let runId = UUID().uuidString
        let srcDir = tempDir.appendingPathComponent("ch3_src_\(runId)")
        let dstDir = tempDir.appendingPathComponent("ch3_dst_\(runId)")

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

    // MARK: - 1. Rapid State Transitions & Never Trap in Running

    func testRapidDiscreteEventTransitionsNeverTrapInRunning() {
        let transitions: [(message: String, expectedState: TransferJobState)] = [
            ("State changed from Running to Paused", .paused),
            ("State changed from Running to Cancelled", .cancelled),
            ("State changed from Running to Completed", .completed),
            ("State changed from Running to Failed", .failed),
            ("State changed from Paused to Running", .running),
            ("State changed from Scanning to Running", .running),
            ("State changed from Idle to Scanning", .scanning),
            ("State changed from RUNNING to PAUSED", .paused),
            ("State changed from RUNNING to CANCELLED", .cancelled),
            ("State changed from RUNNING to COMPLETED", .completed),
            ("State changed from RUNNING to FAILED", .failed),
        ]

        for (idx, t) in transitions.enumerated() {
            let vm = TransferViewModel()
            // Set initial state to running
            vm.handleDiscreteEvent(type: 1, message: "State changed from Scanning to Running", fileId: 0)
            XCTAssertEqual(vm.state, .running, "Iteration \(idx): Failed setup to .running")

            // Deliver target transition
            vm.handleDiscreteEvent(type: 1, message: t.message, fileId: 0)
            XCTAssertEqual(
                vm.state,
                t.expectedState,
                "Iteration \(idx): Message '\(t.message)' expected \(t.expectedState.rawValue) but got \(vm.state.rawValue)"
            )
            if t.expectedState != .running {
                XCTAssertNotEqual(
                    vm.state,
                    .running,
                    "Iteration \(idx): ViewModel TRAPPED in .running for message '\(t.message)'"
                )
            }
        }
    }

    func testHighChurnRandomizedStateTransitionsStress() {
        let targetStates: [(name: String, state: TransferJobState)] = [
            ("Paused", .paused),
            ("Cancelled", .cancelled),
            ("Completed", .completed),
            ("Failed", .failed),
            ("Running", .running)
        ]

        for round in 1...200 {
            let vm = TransferViewModel()
            // Put in running
            vm.handleDiscreteEvent(type: 1, message: "State changed from Scanning to Running", fileId: 0)
            XCTAssertEqual(vm.state, .running)

            let pick = targetStates[round % targetStates.count]
            let msg = "State changed from Running to \(pick.name)"
            vm.handleDiscreteEvent(type: 1, message: msg, fileId: 0)

            XCTAssertEqual(
                vm.state,
                pick.state,
                "Round \(round): Transition to \(pick.name) produced unexpected state \(vm.state)"
            )
        }
    }

    // MARK: - 2. Cancellation Lifecycle & Immediate allowOverwriteConfirmed Invariant

    func testCancellationImmediatelyClearsAllowOverwriteConfirmed() {
        let vm = TransferViewModel()

        // 1. In Idle state
        vm.allowOverwriteConfirmed = true
        vm.showOverwriteDialog = true
        vm.destinationCollisionPath = "/Volumes/Backup/existing.bin"
        vm.cancelTransfer()

        XCTAssertFalse(vm.allowOverwriteConfirmed, "Must be false immediately after cancelTransfer() in Idle")
        XCTAssertFalse(vm.showOverwriteDialog, "Dialog must be dismissed immediately")
        XCTAssertNil(vm.destinationCollisionPath, "Collision path must be nil immediately")
        XCTAssertEqual(vm.state, .cancelled)

        // 2. In Scanning state
        vm.resetToIdle()
        vm.handleDiscreteEvent(type: 1, message: "State changed from Idle to Scanning", fileId: 0)
        vm.allowOverwriteConfirmed = true
        vm.cancelTransfer()
        XCTAssertFalse(vm.allowOverwriteConfirmed, "Must be false immediately after cancelTransfer() in Scanning")
        XCTAssertEqual(vm.state, .cancelled)

        // 3. In Running state
        vm.resetToIdle()
        vm.handleDiscreteEvent(type: 1, message: "State changed from Scanning to Running", fileId: 0)
        vm.allowOverwriteConfirmed = true
        vm.cancelTransfer()
        XCTAssertFalse(vm.allowOverwriteConfirmed, "Must be false immediately after cancelTransfer() in Running")
        XCTAssertEqual(vm.state, .cancelled)

        // 4. In Paused state
        vm.resetToIdle()
        vm.handleDiscreteEvent(type: 1, message: "State changed from Scanning to Running", fileId: 0)
        vm.handleDiscreteEvent(type: 1, message: "State changed from Running to Paused", fileId: 0)
        vm.allowOverwriteConfirmed = true
        vm.cancelTransfer()
        XCTAssertFalse(vm.allowOverwriteConfirmed, "Must be false immediately after cancelTransfer() in Paused")
        XCTAssertEqual(vm.state, .cancelled)
    }

    func testCancellationDuringActiveJobControllerImmediatelyClearsApproval() async throws {
        let (src, dst) = try createTempWorkspace(fileCount: 30, fileSize: 65536)
        defer { cleanupWorkspace(src: src, dst: dst) }

        let vm = TransferViewModel()
        vm.selectSource(url: src)
        vm.selectDestination(url: dst)
        await vm.runPreFlightCheck()

        vm.allowOverwriteConfirmed = true
        vm.startTransfer()

        // Give the background task a few milliseconds to create and start the job
        let startDeadline = Date().addingTimeInterval(2.0)
        while Date() < startDeadline && vm.state != .running && vm.state != .scanning {
            try await Task.sleep(nanoseconds: 10_000_000)
        }

        // Assert overwrite is still confirmed while running
        XCTAssertTrue(vm.allowOverwriteConfirmed)

        // Call cancelTransfer() synchronously
        vm.cancelTransfer()

        // ZERO DELAY check - immediately on MainActor
        XCTAssertFalse(
            vm.allowOverwriteConfirmed,
            "CRITICAL INVARIANT: allowOverwriteConfirmed MUST be false immediately upon return from cancelTransfer(), zero delay"
        )
        XCTAssertEqual(vm.state, .cancelled)

        // Await 300ms for background controller.cancel() to drain
        try await Task.sleep(nanoseconds: 300_000_000)

        // Verify it did not resurrect
        XCTAssertFalse(vm.allowOverwriteConfirmed)
        XCTAssertEqual(vm.state, .cancelled)
        XCTAssertFalse(vm.isActive)
    }

    // MARK: - 3. Zero-Byte Capacity Check Against Full Destination Volumes

    func testEmpiricalZeroByteCapacityPreFlightValidationMatrix() async throws {
        // Test boundary conditions with synthetic files
        let tempDir = FileManager.default.temporaryDirectory.resolvingSymlinksInPath()
        let runId = UUID().uuidString
        let srcDir = tempDir.appendingPathComponent("zero_src_\(runId)")
        let dstDir = tempDir.appendingPathComponent("zero_dst_\(runId)")

        try FileManager.default.createDirectory(at: srcDir, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: dstDir, withIntermediateDirectories: true)
        defer { cleanupWorkspace(src: srcDir, dst: dstDir) }

        let vm = TransferViewModel()
        vm.selectSource(url: srcDir)
        vm.selectDestination(url: dstDir)

        // Case A: 0 byte source files -> 0 space needed -> no insufficient space
        await vm.runPreFlightCheck()
        XCTAssertTrue(vm.preFlightValid)
        XCTAssertFalse(vm.hasInsufficientSpace)
        XCTAssertNil(vm.capacityWarning)
        XCTAssertTrue(vm.canStartTransfer)

        // Case B: Add 1 byte file to source
        let byteFile = srcDir.appendingPathComponent("one_byte.bin")
        try Data([0x42]).write(to: byteFile)
        await vm.runPreFlightCheck()
        XCTAssertTrue(vm.preFlightValid)
        // DstDir has plenty of free space on standard system volume, so hasInsufficientSpace is false
        XCTAssertFalse(vm.hasInsufficientSpace)
        XCTAssertTrue(vm.canStartTransfer)
    }

    func testEmpiricalMountedFullVolumeZeroFreeBytesPreFlight() async throws {
        let tempDir = FileManager.default.temporaryDirectory.resolvingSymlinksInPath()
        let runId = UUID().uuidString
        let dmgPath = tempDir.appendingPathComponent("ch3_zero_\(runId).dmg").path
        let mountPoint = tempDir.appendingPathComponent("ch3_mount_\(runId)").resolvingSymlinksInPath()
        let srcDir = tempDir.appendingPathComponent("ch3_src_\(runId)").resolvingSymlinksInPath()

        try FileManager.default.createDirectory(at: mountPoint, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: srcDir, withIntermediateDirectories: true)
        try "Volterra M5 Iteration 3 Zero-Byte Stress Test".data(using: .utf8)!.write(
            to: srcDir.appendingPathComponent("sample.txt")
        )

        defer {
            try? FileManager.default.removeItem(atPath: dmgPath)
            try? FileManager.default.removeItem(at: mountPoint)
            try? FileManager.default.removeItem(at: srcDir)
        }

        // Create 2MB HFS+ disk image
        let createProc = Process()
        createProc.executableURL = URL(fileURLWithPath: "/usr/bin/hdiutil")
        createProc.arguments = ["create", "-size", "2m", "-fs", "HFS+", "-volname", "Ch3Zero", dmgPath]
        try createProc.run()
        createProc.waitUntilExit()

        guard createProc.terminationStatus == 0 else {
            return
        }

        // Attach disk image
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

        // Fill volume to ENOSPC (0 free bytes)
        let fillPath = mountPoint.appendingPathComponent("fill_target.bin")
        if FileManager.default.createFile(atPath: fillPath.path, contents: nil),
           let handle = try? FileHandle(forWritingTo: fillPath) {
            let block = Data(repeating: 0xAA, count: 4096)
            while true {
                do {
                    try handle.write(contentsOf: block)
                } catch {
                    break
                }
            }
            try? handle.close()
        }

        // Confirm system attributes report 0 free bytes
        let attrs = try FileManager.default.attributesOfFileSystem(forPath: mountPoint.path)
        let freeSize = (attrs[.systemFreeSize] as? NSNumber)?.uint64Value ?? 1
        XCTAssertEqual(freeSize, 0, "Mounted volume freeSize must be exactly 0")

        // 1. Direct volume mount path test
        let vm = TransferViewModel()
        vm.selectSource(url: srcDir)
        vm.selectDestination(url: mountPoint)
        await vm.runPreFlightCheck()

        XCTAssertTrue(vm.preFlightValid, "Pre-flight should be valid")
        XCTAssertTrue(vm.hasInsufficientSpace, "hasInsufficientSpace MUST be true on 0-byte volume")
        XCTAssertNotNil(vm.capacityWarning, "capacityWarning must not be nil")
        XCTAssertEqual(vm.capacityWarning?.availableBytes, 0, "availableBytes must be 0")
        XCTAssertGreaterThan(vm.capacityWarning?.requiredBytes ?? 0, 0, "requiredBytes must be > 0")
        XCTAssertFalse(vm.canStartTransfer, "canStartTransfer must be false when volume has 0 free bytes")

        // 2. Symlinked destination pointing to 0-byte volume
        let symlinkDst = tempDir.appendingPathComponent("ch3_symlink_\(runId)")
        try FileManager.default.createSymbolicLink(at: symlinkDst, withDestinationURL: mountPoint)
        defer { try? FileManager.default.removeItem(at: symlinkDst) }

        let vmSymlink = TransferViewModel()
        vmSymlink.selectSource(url: srcDir)
        vmSymlink.selectDestination(url: symlinkDst)
        await vmSymlink.runPreFlightCheck()

        XCTAssertTrue(vmSymlink.preFlightValid, "Symlinked destination preflight must be valid")
        XCTAssertTrue(vmSymlink.hasInsufficientSpace, "hasInsufficientSpace MUST be true via symlink to 0-byte volume")
        XCTAssertEqual(vmSymlink.capacityWarning?.availableBytes, 0)
        XCTAssertFalse(vmSymlink.canStartTransfer, "canStartTransfer must be false via symlink to 0-byte volume")
    }
}
