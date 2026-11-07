//
// VolterraBridgeTests.swift
// VolterraMacTests
//

import XCTest
import Foundation
import CVolterra
@testable import VolterraBridge

final class VolterraBridgeTests: XCTestCase {

    func testVersion() {
        let version = VolterraBridge.version()
        XCTAssertEqual(version, "1.0.0", "Expected VOLTERRA version 1.0.0")
    }

    func testDiskDiscovery() throws {
        let disks = try VolterraBridge.discoverDisks()
        XCTAssertFalse(disks.isEmpty, "Expected at least 1 discovered disk on macOS")

        for disk in disks {
            XCTAssertFalse(disk.deviceId.isEmpty, "Device ID must not be empty")
            XCTAssertFalse(disk.bsdName.isEmpty, "BSD Name must not be empty")
            XCTAssertFalse(disk.fsType.isEmpty, "FS Type must not be empty")
            XCTAssertGreaterThan(disk.totalBytes, 0, "Total bytes must be > 0")
        }
    }

    func testJobLifecycleAndAsyncStream() async throws {
        let tempDir = FileManager.default.temporaryDirectory
        let runId = UUID().uuidString
        let srcDir = tempDir.appendingPathComponent("volterra_test_src_\(runId)")
        let dstDir = tempDir.appendingPathComponent("volterra_test_dst_\(runId)")

        try FileManager.default.createDirectory(at: srcDir, withIntermediateDirectories: true)
        defer {
            try? FileManager.default.removeItem(at: srcDir)
            try? FileManager.default.removeItem(at: dstDir)
        }

        // Create a test file of 64KB with known pattern
        let testFile = srcDir.appendingPathComponent("payload.bin")
        let data = Data((0..<65536).map { UInt8($0 % 256) })
        try data.write(to: testFile)

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

        let eventStream = await controller.events()

        var receivedCompleted = false
        var eventKinds: [VolterraBridgeEventKind] = []

        // Start event listener task
        let consumerTask = Task {
            for await event in eventStream {
                eventKinds.append(event.kind)
                if event.kind == .jobCompleted {
                    receivedCompleted = true
                    break
                }
                if event.kind == .jobFailed {
                    break
                }
            }
        }

        // Start job
        try await controller.start()

        // Wait for completion (up to 5 seconds)
        let deadline = Date().addingTimeInterval(5.0)
        while Date() < deadline && !receivedCompleted {
            try await Task.sleep(nanoseconds: 50_000_000)
        }

        await consumerTask.value

        XCTAssertTrue(receivedCompleted, "Expected jobCompleted event")

        // Verify snapshot
        let snap = await controller.getSnapshot()
        XCTAssertNotNil(snap)
        if let s = snap {
            XCTAssertEqual(s.state, .completed)
            XCTAssertGreaterThanOrEqual(s.processedBytes, 65536)
        }

        // Verify telemetry
        let tel = await controller.getTelemetry()
        XCTAssertNotNil(tel)

        // Verify error list is empty
        let errors = await controller.getErrors()
        XCTAssertTrue(errors.isEmpty, "Expected zero errors for clean transfer")

        // Verify reports export
        let jsonReport = await controller.exportReport(format: 0)
        XCTAssertNotNil(jsonReport)
        XCTAssertTrue(jsonReport!.contains("completed") || jsonReport!.contains("job_id"))

        let csvReport = await controller.exportReport(format: 1)
        XCTAssertNotNil(csvReport)
        XCTAssertTrue(csvReport!.contains("record_type") || csvReport!.contains("SUMMARY"))

        // Verify destination file byte integrity
        let copiedFile = dstDir.appendingPathComponent("payload.bin")
        XCTAssertTrue(FileManager.default.fileExists(atPath: copiedFile.path))
        let copiedData = try Data(contentsOf: copiedFile)
        XCTAssertEqual(copiedData, data, "Copied data must match source exactly")

        await controller.close()
    }

    func testOutOfOrderStateRejection() async throws {
        let tempDir = FileManager.default.temporaryDirectory
        let runId = UUID().uuidString
        let srcDir = tempDir.appendingPathComponent("volterra_bad_src_\(runId)")
        let dstDir = tempDir.appendingPathComponent("volterra_bad_dst_\(runId)")

        try FileManager.default.createDirectory(at: srcDir, withIntermediateDirectories: true)
        defer {
            try? FileManager.default.removeItem(at: srcDir)
            try? FileManager.default.removeItem(at: dstDir)
        }

        let controller = try VolterraBridge.createJob(
            sourcePath: srcDir.path,
            destPath: dstDir.path,
            mode: .smartHybrid
        )

        // Resume on a pending job must fail with invalidState
        do {
            try await controller.resume()
            XCTFail("Resuming a pending job should have thrown an error")
        } catch let err as VolterraBridgeError {
            switch err {
            case .invalidState:
                // Expected
                break
            default:
                XCTFail("Expected invalidState error, got: \(err)")
            }
        }

        await controller.close()
    }
}
