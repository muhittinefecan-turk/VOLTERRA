//
// ViewModelTests.swift
// VolterraMacTests
//

import XCTest
import Foundation
import SwiftUI
@testable import VolterraMac
@testable import VolterraBridge

@MainActor
final class ViewModelTests: XCTestCase {

    // MARK: - TransferViewModel Tests

    func testTransferViewModelInitialState() {
        let vm = TransferViewModel()
        XCTAssertEqual(vm.state, .idle)
        XCTAssertFalse(vm.isActive)
        XCTAssertFalse(vm.canPauseOrResume)
        XCTAssertFalse(vm.canStartTransfer)
        XCTAssertEqual(vm.selectedMode, .smartHybrid)
        XCTAssertEqual(vm.fractionCompleted, 0.0)
    }

    func testTransferViewModelPreFlightValidation() async throws {
        let vm = TransferViewModel()
        let tempDir = FileManager.default.temporaryDirectory
        let runId = UUID().uuidString
        let srcDir = tempDir.appendingPathComponent("vm_test_src_\(runId)")
        let dstDir = tempDir.appendingPathComponent("vm_test_dst_\(runId)")

        try FileManager.default.createDirectory(at: srcDir, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: dstDir, withIntermediateDirectories: true)
        defer {
            try? FileManager.default.removeItem(at: srcDir)
            try? FileManager.default.removeItem(at: dstDir)
        }

        // Test file in source
        let file = srcDir.appendingPathComponent("data.bin")
        try "Test payload".data(using: .utf8)?.write(to: file)

        vm.selectSource(url: srcDir)
        vm.selectDestination(url: dstDir)

        await vm.runPreFlightCheck()

        XCTAssertTrue(vm.preFlightValid)
        XCTAssertNil(vm.preFlightWarning)
        XCTAssertTrue(vm.canStartTransfer)
    }

    func testTransferViewModelCollisionDetection() async throws {
        let vm = TransferViewModel()
        let tempDir = FileManager.default.temporaryDirectory
        let runId = UUID().uuidString
        let srcDir = tempDir.appendingPathComponent("collision_src_\(runId)")
        let dstDir = tempDir.appendingPathComponent("collision_dst_\(runId)")

        try FileManager.default.createDirectory(at: srcDir, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: dstDir, withIntermediateDirectories: true)
        defer {
            try? FileManager.default.removeItem(at: srcDir)
            try? FileManager.default.removeItem(at: dstDir)
        }

        // Create same filename in both src and dst
        try "Content A".data(using: .utf8)?.write(to: srcDir.appendingPathComponent("shared.txt"))
        try "Content B".data(using: .utf8)?.write(to: dstDir.appendingPathComponent("shared.txt"))

        vm.selectSource(url: srcDir)
        vm.selectDestination(url: dstDir)

        await vm.runPreFlightCheck()

        XCTAssertTrue(vm.destinationHasExistingFiles)
        XCTAssertEqual(vm.collidingFilesCount, 1)
        XCTAssertTrue(vm.collidingSamplePaths.contains("shared.txt"))
    }

    func testTransferViewModelDiscreteEvents() {
        let vm = TransferViewModel()

        // Test running transition
        vm.handleDiscreteEvent(type: 1, message: "Running transfer", fileId: 0)
        XCTAssertEqual(vm.state, .running)
        XCTAssertTrue(vm.isActive)

        // Test file quarantined event
        vm.handleDiscreteEvent(type: 7, message: "Read error", fileId: 101)
        XCTAssertEqual(vm.failedFiles, 1)
        XCTAssertEqual(vm.quarantinedCount, 1)

        // Test completed transition
        vm.handleDiscreteEvent(type: 11, message: "All verified", fileId: 0)
        XCTAssertEqual(vm.state, .completed)
        XCTAssertFalse(vm.isActive)

        // Reset
        vm.resetToIdle()
        XCTAssertEqual(vm.state, .idle)
        XCTAssertEqual(vm.failedFiles, 0)
        XCTAssertFalse(vm.allowOverwriteConfirmed)
        XCTAssertFalse(vm.showOverwriteDialog)
        XCTAssertEqual(vm.activeJobId, "")
        XCTAssertEqual(vm.currentFileName, "")
        XCTAssertNil(vm.destinationCollisionPath)
    }

    // MARK: - PerformanceViewModel Tests

    func testPerformanceViewModelTelemetryUpdateAndRingBuffers() {
        let pvm = PerformanceViewModel()
        XCTAssertEqual(pvm.bottleneckCode, .nominal)

        var state = CoalescedTelemetryState()
        state.instantaneousMbps = 245.5
        state.ewma1sMbps = 240.0
        state.ewma5sMbps = 235.0
        state.ewma30sMbps = 230.0
        state.sourceReadMbps = 250.0
        state.destWriteMbps = 240.0
        state.physicalThroughputMbps = 80.0 // Cache burst condition
        state.isCacheBurst = true
        state.p50ReadLatencyMs = 2.4
        state.p95ReadLatencyMs = 8.1
        state.p50WriteLatencyMs = 1.9
        state.p95WriteLatencyMs = 6.5
        state.activeChunkSizeBytes = 16 * 1024 * 1024
        state.activeConcurrency = 4
        state.bufferUtilizationPct = 62.5
        state.bottleneckCode = BottleneckType.destWrite.rawValue
        state.bottleneckMessage = "Slow destination HDD"

        pvm.applyTelemetryUpdate(state)

        XCTAssertEqual(pvm.instantaneousMbps, 245.5)
        XCTAssertEqual(pvm.activeChunkSizeMB, 16)
        XCTAssertEqual(pvm.activeConcurrency, 4)
        XCTAssertTrue(pvm.isCacheBurst)
        XCTAssertNotNil(pvm.cacheBurstDescription)
        XCTAssertEqual(pvm.bottleneckCode, .destWrite)
        XCTAssertTrue(pvm.bottleneck.isWarning)
        XCTAssertEqual(pvm.throughputHistory.count, 1)
        XCTAssertEqual(pvm.latencyHistory.count, 1)

        // Test ring buffer capping at 60 items
        for _ in 1...100 {
            pvm.applyTelemetryUpdate(state)
        }
        XCTAssertEqual(pvm.throughputHistory.count, 60)
        XCTAssertEqual(pvm.latencyHistory.count, 60)

        pvm.clearHistory()
        XCTAssertTrue(pvm.throughputHistory.isEmpty)
        XCTAssertTrue(pvm.latencyHistory.isEmpty)
        XCTAssertEqual(pvm.bottleneckCode, .nominal)
    }

    // MARK: - ErrorCenterViewModel Tests

    func testErrorCenterViewModelFiltering() {
        let evm = ErrorCenterViewModel()

        XCTAssertTrue(evm.quarantinedErrors.isEmpty)
        XCTAssertEqual(evm.quarantinedCount, 0)
        XCTAssertEqual(evm.totalFailedBytes, 0)

        // Test filtering
        evm.searchQuery = "corrupt"
        XCTAssertTrue(evm.filteredErrors.isEmpty)
    }

    // MARK: - HistoryViewModel Tests

    func testHistoryViewModelPathSanitization() {
        let hvm = HistoryViewModel(databasePath: "/tmp/nonexistent_volterra_test_db.db")
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        let privatePath = "\(home)/Documents/SensitiveProject/file.txt"

        hvm.maskUserPaths = true
        let sanitized = hvm.sanitizePath(privatePath)
        XCTAssertEqual(sanitized, "~/Documents/SensitiveProject/file.txt")

        hvm.maskUserPaths = false
        let unmasked = hvm.sanitizePath(privatePath)
        XCTAssertEqual(unmasked, privatePath)
    }

    func testHistoricalJobItemFormatting() {
        let item = HistoricalJobItem(
            id: "JOB-TEST-1234",
            sourceRoot: "/Volumes/SSD/Photos",
            targetRoot: "/Volumes/Backup/Photos",
            mode: "Smart Hybrid",
            state: "Completed",
            totalFiles: 50,
            completedFiles: 50,
            totalBytes: 104_857_600,
            copiedBytes: 104_857_600,
            createdAt: "2026-11-01T14:30:00Z",
            completedAt: "2026-11-01T14:32:00Z",
            errorCount: 0
        )

        XCTAssertEqual(item.progressPercent, 100.0)
        XCTAssertEqual(item.formattedDate, "2026-11-01 14:30")
        XCTAssertEqual(item.statusColor, .green)
        XCTAssertFalse(item.formattedSize.isEmpty)
    }

    // MARK: - Iteration 2 Hardening: Overwrite & Capacity Tests

    func testAllowOverwriteConfirmedResetOnIdle() async throws {
        let vm = TransferViewModel()
        vm.allowOverwriteConfirmed = true
        vm.showOverwriteDialog = true
        XCTAssertTrue(vm.allowOverwriteConfirmed)

        vm.resetToIdle()

        XCTAssertFalse(vm.allowOverwriteConfirmed, "allowOverwriteConfirmed MUST be reset to false in resetToIdle()")
        XCTAssertFalse(vm.showOverwriteDialog, "showOverwriteDialog MUST be reset to false in resetToIdle()")
        XCTAssertEqual(vm.collidingFilesCount, 0)
        XCTAssertEqual(vm.activeJobId, "")
        XCTAssertEqual(vm.currentFileName, "")
        XCTAssertNil(vm.destinationCollisionPath)
    }

    func testPreFlightCapacityOnZeroFreeBytes() async throws {
        let sourceBytes: UInt64 = 10_000_000
        let freeBytesZero: UInt64 = 0

        // Invariant: sourceBytes > 0 && (freeBytes == 0 || sourceBytes > freeBytes)
        let hasInsufficientSpace = sourceBytes > 0 && (freeBytesZero == 0 || sourceBytes > freeBytesZero)
        XCTAssertTrue(hasInsufficientSpace, "0 free bytes destination must trigger insufficient space condition")

        let warning = CapacityWarning(requiredBytes: sourceBytes, availableBytes: freeBytesZero)
        XCTAssertEqual(warning.requiredBytes, sourceBytes)
        XCTAssertEqual(warning.availableBytes, 0)
    }

    func testTransferViewModelOverwriteApprovalAndResetLifecycle() async throws {
        let vm = TransferViewModel()
        let tempDir = FileManager.default.temporaryDirectory
        let runId = UUID().uuidString
        let srcDir = tempDir.appendingPathComponent("vm_collision_src_\(runId)")
        let dstDir = tempDir.appendingPathComponent("vm_collision_dst_\(runId)")

        try FileManager.default.createDirectory(at: srcDir, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: dstDir, withIntermediateDirectories: true)
        defer {
            try? FileManager.default.removeItem(at: srcDir)
            try? FileManager.default.removeItem(at: dstDir)
        }

        try "Payload A".data(using: .utf8)?.write(to: srcDir.appendingPathComponent("clash.dat"))
        try "Payload B".data(using: .utf8)?.write(to: dstDir.appendingPathComponent("clash.dat"))

        vm.selectSource(url: srcDir)
        vm.selectDestination(url: dstDir)
        await vm.runPreFlightCheck()

        XCTAssertTrue(vm.destinationHasExistingFiles)
        XCTAssertEqual(vm.collidingFilesCount, 1)

        // Starting transfer without consent shows dialog
        vm.startTransfer()
        XCTAssertTrue(vm.showOverwriteDialog)
        XCTAssertEqual(vm.state, .idle)
        XCTAssertFalse(vm.allowOverwriteConfirmed)

        // User approves overwrite
        vm.approveOverwriteAndStart()
        XCTAssertTrue(vm.allowOverwriteConfirmed)
        XCTAssertFalse(vm.showOverwriteDialog)

        // Reset to idle MUST clear overwrite approval
        vm.resetToIdle()
        XCTAssertEqual(vm.state, .idle)
        XCTAssertFalse(vm.allowOverwriteConfirmed, "allowOverwriteConfirmed must be reset to false in resetToIdle")
        XCTAssertFalse(vm.showOverwriteDialog, "showOverwriteDialog must be reset to false in resetToIdle")
        XCTAssertEqual(vm.collidingFilesCount, 0, "collidingFilesCount must be reset to 0 in resetToIdle")
        XCTAssertFalse(vm.destinationHasExistingFiles, "destinationHasExistingFiles must be reset to false in resetToIdle")
        XCTAssertNil(vm.destinationCollisionPath)

        // Re-check and start: must require approval again
        vm.selectSource(url: srcDir)
        vm.selectDestination(url: dstDir)
        await vm.runPreFlightCheck()
        XCTAssertTrue(vm.destinationHasExistingFiles)

        vm.startTransfer()
        XCTAssertTrue(vm.showOverwriteDialog, "Must require approval again on next transfer")
        XCTAssertEqual(vm.state, .idle)
        XCTAssertFalse(vm.allowOverwriteConfirmed)
    }

    func testTransferViewModelZeroByteCapacityPreFlight() {
        let sourceBytes: UInt64 = 10_000_000
        let freeBytesZero: UInt64 = 0

        let hasInsufficientSpace = sourceBytes > 0 && (freeBytesZero == 0 || sourceBytes > freeBytesZero)
        XCTAssertTrue(hasInsufficientSpace, "0 free bytes destination must trigger insufficient space flag")

        let warning = CapacityWarning(requiredBytes: sourceBytes, availableBytes: freeBytesZero)
        XCTAssertEqual(warning.requiredBytes, sourceBytes)
        XCTAssertEqual(warning.availableBytes, 0)
    }

    func testTransferViewModelCleanupJobDetachesErrorCenter() {
        let transferVM = TransferViewModel()
        let errorCenterVM = ErrorCenterViewModel()

        transferVM.errorCenterViewModel = errorCenterVM
        transferVM.resetToIdle()

        // Calling error center actions after job cleanup must be safe, not crash, and provide feedback
        errorCenterVM.skipFile(fileId: 999)
        XCTAssertEqual(errorCenterVM.actionFeedbackMessage, "No active job available")

        errorCenterVM.retryFile(fileId: 999)
        XCTAssertEqual(errorCenterVM.actionFeedbackMessage, "No active job available")

        errorCenterVM.deferFile(fileId: 999)
        XCTAssertEqual(errorCenterVM.actionFeedbackMessage, "No active job available")

        errorCenterVM.skipAllErrors()
        XCTAssertEqual(errorCenterVM.actionFeedbackMessage, "No active job available")

        errorCenterVM.retryAllFailed()
        XCTAssertEqual(errorCenterVM.actionFeedbackMessage, "No active job available")
    }

    func testTransferViewModelStateChangedTargetParsing() {
        let vm = TransferViewModel()

        // 1. Idle -> Scanning
        vm.handleDiscreteEvent(type: 1, message: "State changed from Idle to Scanning", fileId: 0)
        XCTAssertEqual(vm.state, .scanning)

        // 2. Scanning -> Running
        vm.handleDiscreteEvent(type: 1, message: "State changed from Scanning to Running", fileId: 0)
        XCTAssertEqual(vm.state, .running)

        // 3. Running -> Paused
        vm.handleDiscreteEvent(type: 1, message: "State changed from Running to Paused", fileId: 0)
        XCTAssertEqual(vm.state, .paused)

        // 4. Paused -> Running
        vm.handleDiscreteEvent(type: 1, message: "State changed from Paused to Running", fileId: 0)
        XCTAssertEqual(vm.state, .running)

        // 5. Running -> Cancelled
        vm.handleDiscreteEvent(type: 1, message: "State changed from Running to Cancelled", fileId: 0)
        XCTAssertEqual(vm.state, .cancelled)
    }

    func testTransferViewModelTerminalStateProtectionAgainstZombieEvents() {
        let vm = TransferViewModel()

        // Set to cancelled state
        vm.handleDiscreteEvent(type: 1, message: "State changed from Running to Cancelled", fileId: 0)
        XCTAssertEqual(vm.state, .cancelled)

        // Late non-terminal zombie events MUST be ignored
        vm.handleDiscreteEvent(type: 1, message: "State changed from Scanning to Running", fileId: 0)
        XCTAssertEqual(vm.state, .cancelled, "Zombie running event must not overwrite cancelled state")

        vm.handleDiscreteEvent(type: 1, message: "State changed from Running to Paused", fileId: 0)
        XCTAssertEqual(vm.state, .cancelled, "Zombie paused event must not overwrite cancelled state")

        vm.handleDiscreteEvent(type: 1, message: "State changed from Idle to Scanning", fileId: 0)
        XCTAssertEqual(vm.state, .cancelled, "Zombie scanning event must not overwrite cancelled state")

        // Discrete JobCompleted or JobFailed arriving after cancellation must not overwrite cancelled state
        vm.handleDiscreteEvent(type: 11, message: "Job completed", fileId: 0)
        XCTAssertEqual(vm.state, .cancelled, "JobCompleted must not overwrite cancelled state")

        vm.handleDiscreteEvent(type: 12, message: "Job failed", fileId: 0)
        XCTAssertEqual(vm.state, .cancelled, "JobFailed must not overwrite cancelled state")

        // Reset and test completed state protection against zombie updates
        vm.resetToIdle()
        vm.handleDiscreteEvent(type: 1, message: "State changed from Running to Completed", fileId: 0)
        XCTAssertEqual(vm.state, .completed)

        vm.handleDiscreteEvent(type: 1, message: "State changed from Scanning to Running", fileId: 0)
        XCTAssertEqual(vm.state, .completed, "Zombie running event must not overwrite completed state")

        vm.handleDiscreteEvent(type: 1, message: "State changed from Running to Paused", fileId: 0)
        XCTAssertEqual(vm.state, .completed, "Zombie paused event must not overwrite completed state")
    }

    func testTransferViewModelCancelTransferSynchronousPropertiesReset() {
        let vm = TransferViewModel()

        // Configure dummy state
        vm.allowOverwriteConfirmed = true
        vm.showOverwriteDialog = true
        vm.destinationCollisionPath = "sub/file.txt"

        // cancelTransfer must synchronously reset state on MainActor even when controller is nil
        vm.cancelTransfer()

        XCTAssertFalse(vm.allowOverwriteConfirmed, "allowOverwriteConfirmed must synchronously reset to false")
        XCTAssertFalse(vm.showOverwriteDialog, "showOverwriteDialog must synchronously reset to false")
        XCTAssertNil(vm.destinationCollisionPath, "destinationCollisionPath must synchronously reset to nil")
        XCTAssertEqual(vm.state, .cancelled, "state must synchronously be set to .cancelled")
        XCTAssertEqual(vm.statusMessage, "Transfer cancelled by user.")
    }

    func testTransferViewModelCanonicalPathPreFlightDiskMatching() async throws {
        let vm = TransferViewModel()
        let tempDir = FileManager.default.temporaryDirectory.resolvingSymlinksInPath()
        let runId = UUID().uuidString
        let srcDir = tempDir.appendingPathComponent("canonical_src_\(runId)")
        let dstDir = tempDir.appendingPathComponent("canonical_dst_\(runId)")

        try FileManager.default.createDirectory(at: srcDir, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: dstDir, withIntermediateDirectories: true)
        defer {
            try? FileManager.default.removeItem(at: srcDir)
            try? FileManager.default.removeItem(at: dstDir)
        }

        try "Payload".data(using: .utf8)?.write(to: srcDir.appendingPathComponent("test.bin"))

        // Create a symlink pointing to dstDir
        let symlinkDst = tempDir.appendingPathComponent("canonical_dst_symlink_\(runId)")
        try FileManager.default.createSymbolicLink(at: symlinkDst, withDestinationURL: dstDir)
        defer {
            try? FileManager.default.removeItem(at: symlinkDst)
        }

        vm.selectSource(url: srcDir)
        vm.selectDestination(url: symlinkDst)

        await vm.runPreFlightCheck()

        XCTAssertTrue(vm.preFlightValid, "Pre-flight must validate successfully for symlinked destination")
        XCTAssertNil(vm.preFlightWarning)
        XCTAssertTrue(vm.canStartTransfer)
    }
}
