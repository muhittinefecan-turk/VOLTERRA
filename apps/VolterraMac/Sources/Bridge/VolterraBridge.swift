import Foundation
import CVolterra

/// Internal thread-safe continuation box preventing lock-order inversion
private final class EventStreamBox: @unchecked Sendable {
    private let lock = NSLock()
    private var continuation: AsyncStream<VolterraBridgeEvent>.Continuation?

    init(continuation: AsyncStream<VolterraBridgeEvent>.Continuation) {
        self.continuation = continuation
    }

    func yield(_ event: VolterraBridgeEvent) {
        lock.lock()
        let cont = continuation
        lock.unlock()
        cont?.yield(event)
    }

    func finish() {
        lock.lock()
        let cont = continuation
        continuation = nil
        lock.unlock()
        cont?.finish()
    }
}

/// Trampoline C callback for volterra_subscribe_events
private func volterraEventCallback(
    eventPtr: UnsafePointer<volterra_event_t>?,
    userData: UnsafeMutableRawPointer?
) {
    guard let eventPtr = eventPtr, let userData = userData else { return }
    let cEvent = eventPtr.pointee
    let kind = VolterraBridgeEventKind(rawValue: cEvent.event_type) ?? .stateChanged
    let msg = cEvent.message != nil ? String(cString: cEvent.message) : ""
    let event = VolterraBridgeEvent(
        kind: kind,
        fileId: cEvent.file_id,
        progressBytes: cEvent.progress_bytes,
        totalBytes: cEvent.total_bytes,
        message: msg
    )
    let box = Unmanaged<EventStreamBox>.fromOpaque(userData).takeUnretainedValue()
    box.yield(event)
}

/// Actor encapsulating an active VOLTERRA transfer job controller instance.
public actor VolterraJobController {
    private var rawJob: OpaquePointer?
    private var streamBox: EventStreamBox?
    private var streamOpaque: UnsafeMutableRawPointer?

    public init(job: OpaquePointer) {
        self.rawJob = job
    }

    public func getRawHandle() -> OpaquePointer? {
        return rawJob
    }

    deinit {
        if let job = rawJob {
            volterra_job_free(job)
        }
        if let opaque = streamOpaque {
            Unmanaged<EventStreamBox>.fromOpaque(opaque).release()
        }
    }

    public func start() throws {
        guard let job = rawJob else { throw VolterraBridgeError.invalidState("Job already closed") }
        var errPtr: UnsafeMutablePointer<volterra_error_t>? = nil
        let res = volterra_job_start(job, &errPtr)
        if res != 0 {
            if let err = errPtr {
                let bridgeErr = VolterraBridgeError.from(cError: err.pointee)
                volterra_error_free(err)
                throw bridgeErr
            }
            throw VolterraBridgeError.unknown(Int32(res), "Start failed with code \(res)")
        }
    }

    public func pause() throws {
        guard let job = rawJob else { throw VolterraBridgeError.invalidState("Job already closed") }
        var errPtr: UnsafeMutablePointer<volterra_error_t>? = nil
        let res = volterra_job_pause(job, &errPtr)
        if res != 0 {
            if let err = errPtr {
                let bridgeErr = VolterraBridgeError.from(cError: err.pointee)
                volterra_error_free(err)
                throw bridgeErr
            }
            throw VolterraBridgeError.unknown(Int32(res), "Pause failed with code \(res)")
        }
    }

    public func resume() throws {
        guard let job = rawJob else { throw VolterraBridgeError.invalidState("Job already closed") }
        var errPtr: UnsafeMutablePointer<volterra_error_t>? = nil
        let res = volterra_job_resume(job, &errPtr)
        if res != 0 {
            if let err = errPtr {
                let bridgeErr = VolterraBridgeError.from(cError: err.pointee)
                volterra_error_free(err)
                throw bridgeErr
            }
            throw VolterraBridgeError.unknown(Int32(res), "Resume failed with code \(res)")
        }
    }

    public func cancel() throws {
        guard let job = rawJob else { throw VolterraBridgeError.invalidState("Job already closed") }
        var errPtr: UnsafeMutablePointer<volterra_error_t>? = nil
        let res = volterra_job_cancel(job, &errPtr)
        if res != 0 {
            if let err = errPtr {
                let bridgeErr = VolterraBridgeError.from(cError: err.pointee)
                volterra_error_free(err)
                throw bridgeErr
            }
            throw VolterraBridgeError.unknown(Int32(res), "Cancel failed with code \(res)")
        }
    }

    public func getSnapshot() -> VolterraSnapshot? {
        guard let job = rawJob else { return nil }
        guard let snap = volterra_get_snapshot(job) else { return nil }
        defer { volterra_snapshot_free(snap) }

        let s = snap.pointee
        let jobId = s.job_id != nil ? String(cString: s.job_id) : ""
        let bottleneck = s.active_bottleneck != nil ? String(cString: s.active_bottleneck) : nil
        let state = VolterraBridgeState(rawValue: s.state) ?? .pending

        return VolterraSnapshot(
            jobId: jobId,
            state: state,
            totalFiles: s.total_files,
            processedFiles: s.processed_files,
            failedFiles: s.failed_files,
            skippedFiles: s.skipped_files,
            totalBytes: s.total_bytes,
            processedBytes: s.processed_bytes,
            speedBps: s.speed_bps,
            rollingSpeedBps: s.rolling_speed_bps,
            etaSeconds: s.eta_seconds,
            errorCount: s.error_count,
            percentComplete: s.percent_complete,
            readLatencyP50Ms: s.read_latency_p50_ms,
            readLatencyP95Ms: s.read_latency_p95_ms,
            writeLatencyP50Ms: s.write_latency_p50_ms,
            writeLatencyP95Ms: s.write_latency_p95_ms,
            activeChunkSizeBytes: s.active_chunk_size_bytes,
            activeConcurrency: s.active_concurrency,
            activeBottleneck: bottleneck
        )
    }

    public func getTelemetry() -> VolterraTelemetry? {
        guard let job = rawJob else { return nil }
        guard let tel = volterra_get_telemetry(job) else { return nil }
        defer { volterra_telemetry_free(tel) }

        let t = tel.pointee
        let bMsg = t.bottleneck_message != nil ? String(cString: t.bottleneck_message) : ""

        return VolterraTelemetry(
            ewma1sMbps: t.ewma_1s_mbps,
            ewma5sMbps: t.ewma_5s_mbps,
            ewma30sMbps: t.ewma_30s_mbps,
            instantaneousMbps: t.instantaneous_mbps,
            sourceReadMbps: t.source_read_mbps,
            destWriteMbps: t.dest_write_mbps,
            physicalThroughputMbps: t.physical_throughput_mbps,
            p50ReadLatencyMs: t.p50_read_latency_ms,
            p95ReadLatencyMs: t.p95_read_latency_ms,
            p50WriteLatencyMs: t.p50_write_latency_ms,
            p95WriteLatencyMs: t.p95_write_latency_ms,
            bufferUtilizationPct: t.buffer_utilization_pct,
            activeChunkSizeBytes: t.active_chunk_size_bytes,
            activeConcurrency: t.active_concurrency,
            bottleneckCode: t.bottleneck_code,
            bottleneckMessage: bMsg,
            isCacheBurst: t.is_cache_burst,
            isStalled: t.is_stalled,
            totalBytesRead: t.total_bytes_read,
            totalBytesWritten: t.total_bytes_written,
            readErrors: t.read_errors,
            writeErrors: t.write_errors
        )
    }

    public func getErrors() -> [VolterraErrorItem] {
        guard let job = rawJob else { return [] }
        guard let list = volterra_get_errors(job) else { return [] }
        defer { volterra_error_list_free(list) }

        var result: [VolterraErrorItem] = []
        result.reserveCapacity(list.pointee.count)

        for i in 0..<list.pointee.count {
            let item = list.pointee.items[i]
            let path = item.path != nil ? String(cString: item.path) : ""
            let errMsg = item.error_message != nil ? String(cString: item.error_message) : ""

            var extents: [VolterraExtentError] = []
            if item.failed_extents_count > 0 && item.extents != nil {
                extents.reserveCapacity(item.failed_extents_count)
                for j in 0..<item.failed_extents_count {
                    let ext = item.extents[j]
                    extents.append(VolterraExtentError(
                        offset: ext.offset,
                        length: ext.length,
                        errorCode: ext.error_code
                    ))
                }
            }

            result.append(VolterraErrorItem(
                fileId: item.file_id,
                path: path,
                errorKind: item.error_kind,
                errorMessage: errMsg,
                totalSizeBytes: item.total_size_bytes,
                state: item.state,
                attempts: item.attempts,
                extents: extents
            ))
        }

        return result
    }

    public func skipFile(fileId: Int64) -> Bool {
        guard let job = rawJob else { return false }
        return volterra_skip_file(job, fileId) == 0
    }

    public func retryFile(fileId: Int64) -> Bool {
        guard let job = rawJob else { return false }
        return volterra_retry_file(job, fileId) == 0
    }

    public func deferFile(fileId: Int64) -> Bool {
        guard let job = rawJob else { return false }
        return volterra_defer_file(job, fileId) == 0
    }

    public func skipAllErrors() -> Bool {
        guard let job = rawJob else { return false }
        return volterra_skip_all_errors(job) == 0
    }

    public func retryAllFailed() -> Bool {
        guard let job = rawJob else { return false }
        return volterra_retry_all_failed(job) == 0
    }

    public func exportReport(format: Int32) -> String? {
        guard let job = rawJob else { return nil }
        guard let ptr = volterra_export_report(job, format) else { return nil }
        defer { volterra_string_free(ptr) }
        return String(cString: ptr)
    }

    public func events() -> AsyncStream<VolterraBridgeEvent> {
        AsyncStream { continuation in
            if let oldOpaque = self.streamOpaque {
                Unmanaged<EventStreamBox>.fromOpaque(oldOpaque).release()
                self.streamOpaque = nil
            }
            if let oldBox = self.streamBox {
                oldBox.finish()
                self.streamBox = nil
            }

            let box = EventStreamBox(continuation: continuation)
            self.streamBox = box
            let opaque = Unmanaged.passRetained(box).toOpaque()
            self.streamOpaque = opaque

            continuation.onTermination = { @Sendable _ in
                box.finish()
            }

            if let job = self.rawJob {
                _ = volterra_subscribe_events(
                    job,
                    volterraEventCallback,
                    opaque
                )
            }
        }
    }

    public func close() {
        if let job = rawJob {
            volterra_job_free(job)
            rawJob = nil
        }
        if let box = streamBox {
            box.finish()
            streamBox = nil
        }
        if let opaque = streamOpaque {
            Unmanaged<EventStreamBox>.fromOpaque(opaque).release()
            streamOpaque = nil
        }
    }
}

/// Unified entry point for the VOLTERRA framework bridge.
public enum VolterraBridge {
    public static func version() -> String {
        guard let ptr = volterra_version() else { return "unknown" }
        return String(cString: ptr)
    }

    public static func discoverDisks() throws -> [VolterraBridgeDisk] {
        var errPtr: UnsafeMutablePointer<volterra_error_t>? = nil
        guard let diskList = volterra_discover_disks(&errPtr) else {
            if let err = errPtr {
                let bridgeErr = VolterraBridgeError.from(cError: err.pointee)
                volterra_error_free(err)
                throw bridgeErr
            }
            throw VolterraBridgeError.unknown(1, "Failed to discover disks")
        }
        defer { volterra_disks_free(diskList) }

        var result: [VolterraBridgeDisk] = []
        result.reserveCapacity(diskList.pointee.count)

        for i in 0..<diskList.pointee.count {
            let d = diskList.pointee.disks[i]
            let devId = d.device_id != nil ? String(cString: d.device_id) : ""
            let bsd = d.bsd_name != nil ? String(cString: d.bsd_name) : ""
            let mount = d.mount_point != nil ? String(cString: d.mount_point) : ""
            let vol = d.volume_name != nil ? String(cString: d.volume_name) : ""
            let fs = d.fs_type != nil ? String(cString: d.fs_type) : ""
            let parent = d.parent_bsd_name != nil ? String(cString: d.parent_bsd_name) : ""
            let model = d.model != nil ? String(cString: d.model) : ""
            let vendor = d.vendor != nil ? String(cString: d.vendor) : ""
            let transport = d.transport != nil ? String(cString: d.transport) : ""

            result.append(VolterraBridgeDisk(
                deviceId: devId,
                bsdName: bsd,
                mountPoint: mount,
                volumeName: vol,
                fsType: fs,
                totalBytes: d.total_bytes,
                freeBytes: d.free_bytes,
                isRotational: d.is_rotational,
                isRemovable: d.is_removable,
                isApfsSynthesized: d.is_apfs_synthesized,
                parentBsdName: parent,
                model: model,
                vendor: vendor,
                transport: transport
            ))
        }

        return result
    }

    public static func createJob(
        sourcePath: String,
        destPath: String,
        mode: VolterraBridgeMode = .smartHybrid,
        chunkSize: Int = 8 * 1024 * 1024,
        directIO: Bool = false,
        verifyReadback: Bool = true,
        memoryBudget: Int = 128 * 1024 * 1024,
        allowOverwrite: Bool = false,
        journalPath: String? = nil
    ) throws -> VolterraJobController {
        return try sourcePath.withCString { srcPtr in
            try destPath.withCString { dstPtr in
                let makeJob: (UnsafePointer<CChar>?) throws -> VolterraJobController = { jrnPtr in
                    var config = volterra_config_t(
                        source_path: srcPtr,
                        dest_path: dstPtr,
                        mode: mode.rawValue,
                        chunk_size: chunkSize,
                        direct_io: directIO,
                        verify_readback: verifyReadback,
                        memory_budget: memoryBudget,
                        allow_overwrite: allowOverwrite,
                        journal_path: jrnPtr
                    )
                    var errPtr: UnsafeMutablePointer<volterra_error_t>? = nil
                    guard let job = volterra_job_create(&config, &errPtr) else {
                        if let err = errPtr {
                            let bridgeErr = VolterraBridgeError.from(cError: err.pointee)
                            volterra_error_free(err)
                            throw bridgeErr
                        }
                        throw VolterraBridgeError.unknown(1, "Failed to create job")
                    }
                    return VolterraJobController(job: job)
                }

                if let journalPath = journalPath {
                    return try journalPath.withCString { jrnPtr in
                        try makeJob(jrnPtr)
                    }
                } else {
                    return try makeJob(nil)
                }
            }
        }
    }
}
