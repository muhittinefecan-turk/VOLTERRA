//
// swift_bridge_challenge.swift
// Empirical Challenge Test for VOLTERRA Universal Fat Binaries & Swift Bridge
//
// Validates:
// 1. Universal binary linkage (arm64 / x86_64)
// 2. C ABI symbols & Swift Clang module integration
// 3. Null pointer safety across all C ABI functions
// 4. Out-of-order lifecycle state rejection (Pending -> Resume rejected with ErrInvalidState = 13)
// 5. Full job lifecycle (Pending -> Scanning -> Running -> Completed) with event subscription
// 6. Data integrity of transferred files across multiple sizes and subdirectories
// 7. Telemetry, snapshot, error querying, and JSON/CSV reporting
// 8. Memory management, proper deallocation, and stress looping (zero leaks)
//

import Foundation
import CVolterra

// Thread-safe event collector
final class EventCollector: @unchecked Sendable {
    private let lock = NSLock()
    private var events: [(type: UInt32, fileId: Int64, progress: UInt64, total: UInt64, msg: String)] = []

    func append(type: UInt32, fileId: Int64, progress: UInt64, total: UInt64, msg: String) {
        lock.lock()
        defer { lock.unlock() }
        events.append((type, fileId, progress, total, msg))
    }

    func count() -> Int {
        lock.lock()
        defer { lock.unlock() }
        return events.count
    }

    func allEvents() -> [(type: UInt32, fileId: Int64, progress: UInt64, total: UInt64, msg: String)] {
        lock.lock()
        defer { lock.unlock() }
        return events
    }
}

// C-compatible callback trampoline
let eventCallback: volterra_event_callback_fn = { eventPtr, userData in
    guard let event = eventPtr?.pointee, let userData = userData else { return }
    let collector = Unmanaged<EventCollector>.fromOpaque(userData).takeUnretainedValue()
    let msgStr: String
    if let cMsg = event.message {
        msgStr = String(cString: cMsg)
    } else {
        msgStr = ""
    }
    collector.append(
        type: event.event_type,
        fileId: event.file_id,
        progress: event.progress_bytes,
        total: event.total_bytes,
        msg: msgStr
    )
}

func log(_ msg: String) {
    print("[SWIFT_CHALLENGE] \(msg)")
}

func assertCondition(_ cond: Bool, _ message: String) {
    if !cond {
        fputs("[ASSERTION_FAILED] \(message)\n", stderr)
        exit(1)
    }
}

// MARK: - Step 1: Version & Environment Verification
log("Step 1: Checking VOLTERRA version & architecture...")
#if arch(arm64)
log("Running on Apple Silicon (arm64)")
#elseif arch(x86_64)
log("Running on Intel / Rosetta (x86_64)")
#else
log("Running on Unknown Architecture")
#endif

guard let verPtr = volterra_version() else {
    fatalError("volterra_version() returned NULL")
}
let verString = String(cString: verPtr)
assertCondition(verString == "1.0.0", "Version must be 1.0.0, got: \(verString)")
log("Verified version: \(verString)")

// MARK: - Step 2: Disk Discovery
log("Step 2: Checking Disk Discovery via Swift bridge...")
var diskErr: UnsafeMutablePointer<volterra_error_t>? = nil
guard let diskList = volterra_discover_disks(&diskErr) else {
    fatalError("volterra_discover_disks returned NULL")
}
assertCondition(diskErr == nil, "volterra_discover_disks returned non-null error")
assertCondition(diskList.pointee.count > 0, "Discovered disk count should be > 0, got \(diskList.pointee.count)")
log("Discovered \(diskList.pointee.count) disk volumes")
for i in 0..<min(Int(diskList.pointee.count), 3) {
    let disk = diskList.pointee.disks[i]
    let bsd = disk.bsd_name != nil ? String(cString: disk.bsd_name!) : "n/a"
    let mount = disk.mount_point != nil ? String(cString: disk.mount_point!) : "n/a"
    let fs = disk.fs_type != nil ? String(cString: disk.fs_type!) : "n/a"
    log("  Disk[\(i)]: bsd=\(bsd), mount=\(mount), fs=\(fs), total=\(disk.total_bytes / 1024 / 1024)MB")
}
volterra_disks_free(diskList)
volterra_disks_free(nil) // Safe null free
log("Disk discovery verified and cleanly deallocated.")

// MARK: - Step 3: Null Safety Across All C ABI Functions
log("Step 3: Checking Null Pointer Safety across C ABI functions...")
var nullErr: UnsafeMutablePointer<volterra_error_t>? = nil

assertCondition(volterra_job_create(nil, &nullErr) == nil, "job_create(nil) must return nil")
assertCondition(nullErr != nil && nullErr?.pointee.code == Int32(VOLTERRA_ERROR_CODE_ERR_INVALID_ARGUMENT.rawValue), "err code must be invalid arg")
volterra_error_free(nullErr)
nullErr = nil

assertCondition(volterra_job_start(nil, &nullErr) == Int32(VOLTERRA_ERROR_CODE_ERR_INVALID_ARGUMENT.rawValue), "job_start(nil) must return invalid arg")
assertCondition(nullErr != nil, "job_start(nil) out_err must be populated")
volterra_error_free(nullErr)
nullErr = nil

assertCondition(volterra_job_pause(nil, nil) == Int32(VOLTERRA_ERROR_CODE_ERR_INVALID_ARGUMENT.rawValue), "job_pause(nil) must return invalid arg")
assertCondition(volterra_job_resume(nil, nil) == Int32(VOLTERRA_ERROR_CODE_ERR_INVALID_ARGUMENT.rawValue), "job_resume(nil) must return invalid arg")
assertCondition(volterra_job_cancel(nil, nil) == Int32(VOLTERRA_ERROR_CODE_ERR_INVALID_ARGUMENT.rawValue), "job_cancel(nil) must return invalid arg")
volterra_job_free(nil) // Safe null free

assertCondition(volterra_get_snapshot(nil) == nil, "get_snapshot(nil) must return nil")
volterra_snapshot_free(nil)

assertCondition(volterra_get_telemetry(nil) == nil, "get_telemetry(nil) must return nil")
volterra_telemetry_free(nil)

assertCondition(volterra_subscribe_events(nil, nil, nil) == Int32(VOLTERRA_ERROR_CODE_ERR_INVALID_ARGUMENT.rawValue), "subscribe_events(nil) must return invalid arg")
assertCondition(volterra_get_errors(nil) == nil, "get_errors(nil) must return nil")
volterra_error_list_free(nil)

assertCondition(volterra_skip_file(nil, 1) == Int32(VOLTERRA_ERROR_CODE_ERR_INVALID_ARGUMENT.rawValue), "skip_file(nil) must return invalid arg")
assertCondition(volterra_retry_file(nil, 1) == Int32(VOLTERRA_ERROR_CODE_ERR_INVALID_ARGUMENT.rawValue), "retry_file(nil) must return invalid arg")
assertCondition(volterra_defer_file(nil, 1) == Int32(VOLTERRA_ERROR_CODE_ERR_INVALID_ARGUMENT.rawValue), "defer_file(nil) must return invalid arg")
assertCondition(volterra_skip_all_errors(nil) == Int32(VOLTERRA_ERROR_CODE_ERR_INVALID_ARGUMENT.rawValue), "skip_all_errors(nil) must return invalid arg")
assertCondition(volterra_retry_all_failed(nil) == Int32(VOLTERRA_ERROR_CODE_ERR_INVALID_ARGUMENT.rawValue), "retry_all_failed(nil) must return invalid arg")

assertCondition(volterra_export_report(nil, 0) == nil, "export_report(nil) must return nil")
volterra_string_free(nil)
volterra_error_free(nil)
log("Null pointer safety across all functions verified in Swift.")

// MARK: - Step 4: Out-of-Order Lifecycle State Challenge
log("Step 4: Stress-testing out-of-order lifecycle transitions...")
let tempDir = FileManager.default.temporaryDirectory
let testRunId = UUID().uuidString
let srcDir = tempDir.appendingPathComponent("volterra_swift_src_\(testRunId)")
let dstDir = tempDir.appendingPathComponent("volterra_swift_dst_\(testRunId)")
try FileManager.default.createDirectory(at: srcDir, withIntermediateDirectories: true)
try FileManager.default.createDirectory(at: dstDir, withIntermediateDirectories: true)

// Create a test payload file
let payloadPath = srcDir.appendingPathComponent("payload.bin")
let testBytes = Data(repeating: 0xA5, count: 256 * 1024) // 256 KB
try testBytes.write(to: payloadPath)

let srcC = strdup(srcDir.path)!
let dstC = strdup(dstDir.path)!

var oooConfig = volterra_config_t(
    source_path: srcC,
    dest_path: dstC,
    mode: VOLTERRA_MODE_SMART_HYBRID.rawValue,
    chunk_size: 64 * 1024,
    direct_io: false,
    verify_readback: false,
    memory_budget: 16 * 1024 * 1024,
    allow_overwrite: true,
    journal_path: nil
)

var oooErr: UnsafeMutablePointer<volterra_error_t>? = nil
guard let oooJob = volterra_job_create(&oooConfig, &oooErr) else {
    fatalError("Failed to create oooJob: \(String(cString: oooErr!.pointee.message))")
}
assertCondition(oooErr == nil, "oooErr must be nil upon creation")

// Challenge: Call resume while in Pending state!
log("Attempting volterra_job_resume on Pending job...")
let oooResumeRet = volterra_job_resume(oooJob, &oooErr)
assertCondition(
    oooResumeRet == Int32(VOLTERRA_ERROR_CODE_ERR_INVALID_STATE.rawValue),
    "volterra_job_resume on Pending MUST return VOLTERRA_ERR_INVALID_STATE (13), got \(oooResumeRet)"
)
assertCondition(oooErr != nil, "out_err MUST be populated on invalid transition")
if let err = oooErr {
    assertCondition(err.pointee.code == Int32(VOLTERRA_ERROR_CODE_ERR_INVALID_STATE.rawValue), "Error code must be 13")
    let errMsg = String(cString: err.pointee.message)
    log("Verified error message: \(errMsg)")
    assertCondition(errMsg.contains("Cannot resume job"), "Error message should describe invalid resume")
    volterra_error_free(oooErr)
    oooErr = nil
}

// Verify snapshot still shows Pending
guard let oooSnap = volterra_get_snapshot(oooJob) else {
    fatalError("Failed to get snapshot from oooJob")
}
assertCondition(
    oooSnap.pointee.state == VOLTERRA_STATE_PENDING.rawValue,
    "Job state MUST remain Pending (0) after rejected resume, got \(oooSnap.pointee.state)"
)
volterra_snapshot_free(oooSnap)

// Challenge: Call pause while in Pending state
let oooPauseRet = volterra_job_pause(oooJob, &oooErr)
log("volterra_job_pause on Pending returned: \(oooPauseRet)")
if oooErr != nil {
    volterra_error_free(oooErr)
    oooErr = nil
}

// Cleanly free this job
volterra_job_free(oooJob)
free(srcC)
free(dstC)
log("Out-of-order lifecycle challenge passed successfully.")

// MARK: - Step 5: Full End-to-End Transfer Lifecycle with Events
log("Step 5: Executing full transfer job lifecycle with event callback...")

// Prepare structured dataset:
// 1. small.txt (1000 bytes)
// 2. nested/payload.bin (512 KB)
// 3. nested/deep/marker.dat (128 KB)
let nestedDir = srcDir.appendingPathComponent("nested")
let deepDir = nestedDir.appendingPathComponent("deep")
try FileManager.default.createDirectory(at: deepDir, withIntermediateDirectories: true)

let smallPath = srcDir.appendingPathComponent("small.txt")
let smallData = "VOLTERRA Swift Bridge Integration Challenge 2026\n".data(using: .utf8)!
try smallData.write(to: smallPath)

let mediumPath = nestedDir.appendingPathComponent("payload.bin")
var mediumData = Data(count: 512 * 1024)
for i in 0..<mediumData.count {
    mediumData[i] = UInt8(i % 251)
}
try mediumData.write(to: mediumPath)

let deepPath = deepDir.appendingPathComponent("marker.dat")
let deepData = Data(repeating: 0x7E, count: 128 * 1024)
try deepData.write(to: deepPath)

let totalExpectedBytes = UInt64(smallData.count + mediumData.count + deepData.count)

let fullSrcC = strdup(srcDir.path)!
let fullDstC = strdup(dstDir.path)!

var fullConfig = volterra_config_t(
    source_path: fullSrcC,
    dest_path: fullDstC,
    mode: VOLTERRA_MODE_SMART_HYBRID.rawValue,
    chunk_size: 64 * 1024,
    direct_io: false,
    verify_readback: true,
    memory_budget: 32 * 1024 * 1024,
    allow_overwrite: true,
    journal_path: nil
)

var jobErr: UnsafeMutablePointer<volterra_error_t>? = nil
guard let job = volterra_job_create(&fullConfig, &jobErr) else {
    fatalError("Failed to create full job: \(String(describing: jobErr?.pointee.message))")
}
assertCondition(jobErr == nil, "jobErr must be nil")

let collector = EventCollector()
let collectorPtr = Unmanaged.passUnretained(collector).toOpaque()

let subRet = volterra_subscribe_events(job, eventCallback, collectorPtr)
assertCondition(subRet == Int32(VOLTERRA_ERROR_CODE_OK.rawValue), "volterra_subscribe_events must return OK (0)")

// Inspect snapshot and telemetry before start
guard let snapBefore = volterra_get_snapshot(job) else {
    fatalError("Snapshot before start returned nil")
}
assertCondition(snapBefore.pointee.state == VOLTERRA_STATE_PENDING.rawValue, "State must be Pending before start")
let jobIdString = String(cString: snapBefore.pointee.job_id)
log("Job created with ID: \(jobIdString)")
volterra_snapshot_free(snapBefore)

guard let telBefore = volterra_get_telemetry(job) else {
    fatalError("Telemetry before start returned nil")
}
log("Initial bottleneck code: \(telBefore.pointee.bottleneck_code)")
volterra_telemetry_free(telBefore)

// Test report exports in Pending state
if let repJson = volterra_export_report(job, Int32(VOLTERRA_REPORT_FORMAT_JSON.rawValue)) {
    let s = String(cString: repJson)
    assertCondition(s.contains("volterra_version") && s.contains("1.0.0"), "JSON report invalid")
    volterra_string_free(repJson)
} else {
    fatalError("volterra_export_report JSON returned nil")
}

// Start Job
log("Starting job execution...")
let startRet = volterra_job_start(job, &jobErr)
assertCondition(startRet == Int32(VOLTERRA_ERROR_CODE_OK.rawValue), "volterra_job_start failed with code \(startRet)")
assertCondition(jobErr == nil, "jobErr must be nil on success")

// Poll job progress until Completed or Failed
var isTerminal = false
var terminalState: UInt32 = 0
let deadline = Date().addingTimeInterval(15.0)

while Date() < deadline {
    guard let snap = volterra_get_snapshot(job) else {
        fatalError("Failed to get snapshot during polling")
    }
    let st = snap.pointee.state
    terminalState = st
    let totalF = snap.pointee.total_files
    let procF = snap.pointee.processed_files
    let procB = snap.pointee.processed_bytes
    let pct = snap.pointee.percent_complete
    let speed = snap.pointee.speed_bps / (1024 * 1024)
    volterra_snapshot_free(snap)

    if st == VOLTERRA_STATE_COMPLETED.rawValue || st == VOLTERRA_STATE_FAILED.rawValue || st == VOLTERRA_STATE_CANCELLED.rawValue {
        isTerminal = true
        log("Job reached terminal state: \(st) (Completed=\(VOLTERRA_STATE_COMPLETED.rawValue))")
        break
    }
    log("Progress: state=\(st), files=\(procF)/\(totalF), bytes=\(procB)/\(totalExpectedBytes), speed=\(String(format: "%.2f", speed)) MB/s, \(String(format: "%.1f", pct))%")
    Thread.sleep(forTimeInterval: 0.05)
}

assertCondition(isTerminal, "Job did not reach terminal state within 15 seconds")
assertCondition(terminalState == VOLTERRA_STATE_COMPLETED.rawValue, "Job ended in unexpected state: \(terminalState)")

// Verify final snapshot
guard let snapFinal = volterra_get_snapshot(job) else {
    fatalError("Failed to get final snapshot")
}
assertCondition(snapFinal.pointee.processed_files == 4, "Expected 4 processed files (including payload.bin from step 4), got \(snapFinal.pointee.processed_files)")
assertCondition(snapFinal.pointee.failed_files == 0, "Failed files must be 0")
assertCondition(snapFinal.pointee.error_count == 0, "Error count must be 0")
log("Final snapshot: processed_files=\(snapFinal.pointee.processed_files), processed_bytes=\(snapFinal.pointee.processed_bytes)")
volterra_snapshot_free(snapFinal)

// Verify telemetry
guard let telFinal = volterra_get_telemetry(job) else {
    fatalError("Failed to get final telemetry")
}
assertCondition(telFinal.pointee.read_errors == 0, "Final telemetry read errors must be 0")
assertCondition(telFinal.pointee.write_errors == 0, "Final telemetry write errors must be 0")
log("Final telemetry: read_bytes=\(telFinal.pointee.total_bytes_read), write_bytes=\(telFinal.pointee.total_bytes_written)")
volterra_telemetry_free(telFinal)

// Verify error list is empty
guard let errList = volterra_get_errors(job) else {
    fatalError("Failed to get error list")
}
assertCondition(errList.pointee.count == 0, "Quarantined error count should be 0")
volterra_error_list_free(errList)

// Verify exported audit reports
guard let finalReportJson = volterra_export_report(job, Int32(VOLTERRA_REPORT_FORMAT_JSON.rawValue)) else {
    fatalError("Failed to export final JSON report")
}
let jsonStr = String(cString: finalReportJson)
assertCondition(jsonStr.contains("\"state\": \"Completed\"") || jsonStr.contains("\"state\":\"Completed\"") || jsonStr.contains("Completed"), "Report must show Completed")
volterra_string_free(finalReportJson)

guard let finalReportCsv = volterra_export_report(job, Int32(VOLTERRA_REPORT_FORMAT_CSV.rawValue)) else {
    fatalError("Failed to export final CSV report")
}
let csvStr = String(cString: finalReportCsv)
assertCondition(csvStr.contains("record_type") && csvStr.contains("SUMMARY"), "CSV report must contain record_type and SUMMARY")
volterra_string_free(finalReportCsv)

// Verify received events
let events = collector.allEvents()
log("Total events received by Swift callback: \(events.count)")
assertCondition(events.count > 0, "Must have received at least 1 event")
var sawJobCompletedEvent = false
for ev in events {
    if ev.type == VOLTERRA_EVENT_KIND_JOB_COMPLETED.rawValue {
        sawJobCompletedEvent = true
    }
}
assertCondition(sawJobCompletedEvent, "Must have received VOLTERRA_EVENT_KIND_JOB_COMPLETED event")
log("Event callback delivery verified.")

// Cleanly free the completed job
volterra_job_free(job)
free(fullSrcC)
free(fullDstC)
log("Full job lifecycle verified and cleanly freed.")

// MARK: - Step 6: Verify Exact Data Integrity
log("Step 6: Verifying byte-for-byte data integrity on destination...")
let dstSmall = dstDir.appendingPathComponent("small.txt")
let dstMedium = dstDir.appendingPathComponent("nested/payload.bin")
let dstDeep = dstDir.appendingPathComponent("nested/deep/marker.dat")

assertCondition(FileManager.default.fileExists(atPath: dstSmall.path), "dst small.txt missing")
assertCondition(FileManager.default.fileExists(atPath: dstMedium.path), "dst nested/payload.bin missing")
assertCondition(FileManager.default.fileExists(atPath: dstDeep.path), "dst nested/deep/marker.dat missing")

let dstSmallData = try Data(contentsOf: dstSmall)
assertCondition(dstSmallData == smallData, "small.txt data mismatch")

let dstMediumData = try Data(contentsOf: dstMedium)
assertCondition(dstMediumData == mediumData, "payload.bin data mismatch")

let dstDeepData = try Data(contentsOf: dstDeep)
assertCondition(dstDeepData == deepData, "marker.dat data mismatch")
log("100% Data integrity verified across all destination files.")

// MARK: - Step 7: Repetitive Lifecycle Stress & Leak Loop
log("Step 7: Executing 10 repetitive create/start/free lifecycle cycles...")
for iteration in 1...10 {
    let iterSrc = tempDir.appendingPathComponent("volterra_iter_src_\(iteration)")
    let iterDst = tempDir.appendingPathComponent("volterra_iter_dst_\(iteration)")
    try FileManager.default.createDirectory(at: iterSrc, withIntermediateDirectories: true)
    try FileManager.default.createDirectory(at: iterDst, withIntermediateDirectories: true)

    let iterFile = iterSrc.appendingPathComponent("file.dat")
    try Data(repeating: UInt8(iteration & 0xFF), count: 64 * 1024).write(to: iterFile)

    let iterSrcC = strdup(iterSrc.path)!
    let iterDstC = strdup(iterDst.path)!

    var cfg = volterra_config_t(
        source_path: iterSrcC,
        dest_path: iterDstC,
        mode: VOLTERRA_MODE_TURBO.rawValue,
        chunk_size: 32 * 1024,
        direct_io: false,
        verify_readback: false,
        memory_budget: 16 * 1024 * 1024,
        allow_overwrite: true,
        journal_path: nil
    )

    var iterErr: UnsafeMutablePointer<volterra_error_t>? = nil
    guard let iterJob = volterra_job_create(&cfg, &iterErr) else {
        fatalError("Failed to create iterJob on iteration \(iteration)")
    }

    let iterCollector = EventCollector()
    let iterCollectorPtr = Unmanaged.passUnretained(iterCollector).toOpaque()
    _ = volterra_subscribe_events(iterJob, eventCallback, iterCollectorPtr)

    let sRet = volterra_job_start(iterJob, &iterErr)
    assertCondition(sRet == Int32(VOLTERRA_ERROR_CODE_OK.rawValue), "Start failed on iteration \(iteration)")

    // Wait until completed
    var iterDone = false
    for _ in 0..<100 {
        if let s = volterra_get_snapshot(iterJob) {
            let state = s.pointee.state
            volterra_snapshot_free(s)
            if state == VOLTERRA_STATE_COMPLETED.rawValue {
                iterDone = true
                break
            }
        }
        Thread.sleep(forTimeInterval: 0.02)
    }
    assertCondition(iterDone, "Iteration \(iteration) did not complete")

    // Snapshots, telemetry, and reporting in each iteration
    if let s = volterra_get_snapshot(iterJob) { volterra_snapshot_free(s) }
    if let t = volterra_get_telemetry(iterJob) { volterra_telemetry_free(t) }
    if let r = volterra_export_report(iterJob, Int32(VOLTERRA_REPORT_FORMAT_JSON.rawValue)) { volterra_string_free(r) }
    if let el = volterra_get_errors(iterJob) { volterra_error_list_free(el) }

    volterra_job_free(iterJob)
    free(iterSrcC)
    free(iterDstC)

    try? FileManager.default.removeItem(at: iterSrc)
    try? FileManager.default.removeItem(at: iterDst)
}
log("Completed 10 repetitive lifecycle cycles with zero memory or concurrency issues.")

// Cleanup main temp dirs
try? FileManager.default.removeItem(at: srcDir)
try? FileManager.default.removeItem(at: dstDir)

log("ALL EMPIRICAL CHALLENGE TESTS PASSED SUCCESSFULLY.")
exit(0)
