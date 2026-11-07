import Foundation
import CVolterra

public enum VolterraBridgeMode: UInt32, Sendable {
    case turbo = 0
    case smartHybrid = 1
    case safeRecovery = 2
}

public enum VolterraBridgeState: UInt32, Sendable {
    case pending = 0
    case scanning = 1
    case running = 2
    case paused = 3
    case completed = 4
    case failed = 5
    case waitingForDisk = 6
    case cancelled = 7
}

public enum VolterraBridgeEventKind: UInt32, Sendable {
    case stateChanged = 1
    case scanningProgress = 2
    case scanningComplete = 3
    case fileStarted = 4
    case chunkProgress = 5
    case fileCompleted = 6
    case fileQuarantined = 7
    case telemetryUpdated = 8
    case bottleneckDetected = 9
    case diskWarning = 10
    case jobCompleted = 11
    case jobFailed = 12
}

public struct VolterraBridgeEvent: Sendable {
    public let kind: VolterraBridgeEventKind
    public let fileId: Int64
    public let progressBytes: UInt64
    public let totalBytes: UInt64
    public let message: String

    public init(
        kind: VolterraBridgeEventKind,
        fileId: Int64,
        progressBytes: UInt64,
        totalBytes: UInt64,
        message: String
    ) {
        self.kind = kind
        self.fileId = fileId
        self.progressBytes = progressBytes
        self.totalBytes = totalBytes
        self.message = message
    }
}

public struct VolterraSnapshot: Sendable {
    public let jobId: String
    public let state: VolterraBridgeState
    public let totalFiles: UInt64
    public let processedFiles: UInt64
    public let failedFiles: UInt64
    public let skippedFiles: UInt64
    public let totalBytes: UInt64
    public let processedBytes: UInt64
    public let speedBps: Double
    public let rollingSpeedBps: Double
    public let etaSeconds: UInt64
    public let errorCount: UInt64
    public let percentComplete: Double
    public let readLatencyP50Ms: Double
    public let readLatencyP95Ms: Double
    public let writeLatencyP50Ms: Double
    public let writeLatencyP95Ms: Double
    public let activeChunkSizeBytes: Int
    public let activeConcurrency: Int
    public let activeBottleneck: String?

    public init(
        jobId: String,
        state: VolterraBridgeState,
        totalFiles: UInt64,
        processedFiles: UInt64,
        failedFiles: UInt64,
        skippedFiles: UInt64,
        totalBytes: UInt64,
        processedBytes: UInt64,
        speedBps: Double,
        rollingSpeedBps: Double,
        etaSeconds: UInt64,
        errorCount: UInt64,
        percentComplete: Double,
        readLatencyP50Ms: Double,
        readLatencyP95Ms: Double,
        writeLatencyP50Ms: Double,
        writeLatencyP95Ms: Double,
        activeChunkSizeBytes: Int,
        activeConcurrency: Int,
        activeBottleneck: String?
    ) {
        self.jobId = jobId
        self.state = state
        self.totalFiles = totalFiles
        self.processedFiles = processedFiles
        self.failedFiles = failedFiles
        self.skippedFiles = skippedFiles
        self.totalBytes = totalBytes
        self.processedBytes = processedBytes
        self.speedBps = speedBps
        self.rollingSpeedBps = rollingSpeedBps
        self.etaSeconds = etaSeconds
        self.errorCount = errorCount
        self.percentComplete = percentComplete
        self.readLatencyP50Ms = readLatencyP50Ms
        self.readLatencyP95Ms = readLatencyP95Ms
        self.writeLatencyP50Ms = writeLatencyP50Ms
        self.writeLatencyP95Ms = writeLatencyP95Ms
        self.activeChunkSizeBytes = activeChunkSizeBytes
        self.activeConcurrency = activeConcurrency
        self.activeBottleneck = activeBottleneck
    }
}

public struct VolterraTelemetry: Sendable {
    public let ewma1sMbps: Double
    public let ewma5sMbps: Double
    public let ewma30sMbps: Double
    public let instantaneousMbps: Double
    public let sourceReadMbps: Double
    public let destWriteMbps: Double
    public let physicalThroughputMbps: Double
    public let p50ReadLatencyMs: Double
    public let p95ReadLatencyMs: Double
    public let p50WriteLatencyMs: Double
    public let p95WriteLatencyMs: Double
    public let bufferUtilizationPct: Double
    public let activeChunkSizeBytes: Int
    public let activeConcurrency: Int
    public let bottleneckCode: UInt32
    public let bottleneckMessage: String
    public let isCacheBurst: Bool
    public let isStalled: Bool
    public let totalBytesRead: UInt64
    public let totalBytesWritten: UInt64
    public let readErrors: UInt64
    public let writeErrors: UInt64

    public init(
        ewma1sMbps: Double,
        ewma5sMbps: Double,
        ewma30sMbps: Double,
        instantaneousMbps: Double,
        sourceReadMbps: Double,
        destWriteMbps: Double,
        physicalThroughputMbps: Double,
        p50ReadLatencyMs: Double,
        p95ReadLatencyMs: Double,
        p50WriteLatencyMs: Double,
        p95WriteLatencyMs: Double,
        bufferUtilizationPct: Double,
        activeChunkSizeBytes: Int,
        activeConcurrency: Int,
        bottleneckCode: UInt32,
        bottleneckMessage: String,
        isCacheBurst: Bool,
        isStalled: Bool,
        totalBytesRead: UInt64,
        totalBytesWritten: UInt64,
        readErrors: UInt64,
        writeErrors: UInt64
    ) {
        self.ewma1sMbps = ewma1sMbps
        self.ewma5sMbps = ewma5sMbps
        self.ewma30sMbps = ewma30sMbps
        self.instantaneousMbps = instantaneousMbps
        self.sourceReadMbps = sourceReadMbps
        self.destWriteMbps = destWriteMbps
        self.physicalThroughputMbps = physicalThroughputMbps
        self.p50ReadLatencyMs = p50ReadLatencyMs
        self.p95ReadLatencyMs = p95ReadLatencyMs
        self.p50WriteLatencyMs = p50WriteLatencyMs
        self.p95WriteLatencyMs = p95WriteLatencyMs
        self.bufferUtilizationPct = bufferUtilizationPct
        self.activeChunkSizeBytes = activeChunkSizeBytes
        self.activeConcurrency = activeConcurrency
        self.bottleneckCode = bottleneckCode
        self.bottleneckMessage = bottleneckMessage
        self.isCacheBurst = isCacheBurst
        self.isStalled = isStalled
        self.totalBytesRead = totalBytesRead
        self.totalBytesWritten = totalBytesWritten
        self.readErrors = readErrors
        self.writeErrors = writeErrors
    }
}

public struct VolterraExtentError: Sendable {
    public let offset: UInt64
    public let length: UInt64
    public let errorCode: Int32

    public init(offset: UInt64, length: UInt64, errorCode: Int32) {
        self.offset = offset
        self.length = length
        self.errorCode = errorCode
    }
}

public struct VolterraErrorItem: Sendable {
    public let fileId: Int64
    public let path: String
    public let errorKind: Int32
    public let errorMessage: String
    public let totalSizeBytes: UInt64
    public let state: UInt32
    public let attempts: UInt32
    public let extents: [VolterraExtentError]

    public init(
        fileId: Int64,
        path: String,
        errorKind: Int32,
        errorMessage: String,
        totalSizeBytes: UInt64,
        state: UInt32,
        attempts: UInt32,
        extents: [VolterraExtentError]
    ) {
        self.fileId = fileId
        self.path = path
        self.errorKind = errorKind
        self.errorMessage = errorMessage
        self.totalSizeBytes = totalSizeBytes
        self.state = state
        self.attempts = attempts
        self.extents = extents
    }
}

public struct VolterraBridgeDisk: Sendable {
    public let deviceId: String
    public let bsdName: String
    public let mountPoint: String
    public let volumeName: String
    public let fsType: String
    public let totalBytes: UInt64
    public let freeBytes: UInt64
    public let isRotational: Bool
    public let isRemovable: Bool
    public let isApfsSynthesized: Bool
    public let parentBsdName: String
    public let model: String
    public let vendor: String
    public let transport: String

    public init(
        deviceId: String,
        bsdName: String,
        mountPoint: String,
        volumeName: String,
        fsType: String,
        totalBytes: UInt64,
        freeBytes: UInt64,
        isRotational: Bool,
        isRemovable: Bool,
        isApfsSynthesized: Bool,
        parentBsdName: String,
        model: String,
        vendor: String,
        transport: String
    ) {
        self.deviceId = deviceId
        self.bsdName = bsdName
        self.mountPoint = mountPoint
        self.volumeName = volumeName
        self.fsType = fsType
        self.totalBytes = totalBytes
        self.freeBytes = freeBytes
        self.isRotational = isRotational
        self.isRemovable = isRemovable
        self.isApfsSynthesized = isApfsSynthesized
        self.parentBsdName = parentBsdName
        self.model = model
        self.vendor = vendor
        self.transport = transport
    }
}

public enum VolterraBridgeError: Error, LocalizedError, Sendable {
    case invalidArgument(String)
    case sourceNotFound(String)
    case targetNotFound(String)
    case targetExists(String)
    case insufficientSpace(String)
    case permissionDenied(String)
    case readIO(String)
    case writeIO(String)
    case integrityMismatch(String)
    case diskDisconnected(String)
    case sourceModified(String)
    case unsupportedMetadata(String)
    case invalidState(String)
    case recoveryFailed(String)
    case internalPanic(String)
    case unknown(Int32, String)

    public static func from(cError: volterra_error_t) -> VolterraBridgeError {
        let msg = cError.message != nil ? String(cString: cError.message) : "Unknown error"
        switch cError.code {
        case 1: return .invalidArgument(msg)
        case 2: return .sourceNotFound(msg)
        case 3: return .targetNotFound(msg)
        case 4: return .targetExists(msg)
        case 5: return .insufficientSpace(msg)
        case 6: return .permissionDenied(msg)
        case 7: return .readIO(msg)
        case 8: return .writeIO(msg)
        case 9: return .integrityMismatch(msg)
        case 10: return .diskDisconnected(msg)
        case 11: return .sourceModified(msg)
        case 12: return .unsupportedMetadata(msg)
        case 13: return .invalidState(msg)
        case 14: return .recoveryFailed(msg)
        case 99: return .internalPanic(msg)
        default: return .unknown(cError.code, msg)
        }
    }

    public var errorDescription: String? {
        switch self {
        case .invalidArgument(let m): return "Invalid argument: \(m)"
        case .sourceNotFound(let m): return "Source path not found: \(m)"
        case .targetNotFound(let m): return "Destination path not found: \(m)"
        case .targetExists(let m): return "Target destination already exists: \(m)"
        case .insufficientSpace(let m): return "Insufficient disk space: \(m)"
        case .permissionDenied(let m): return "Permission denied: \(m)"
        case .readIO(let m): return "Read I/O failure: \(m)"
        case .writeIO(let m): return "Write I/O failure: \(m)"
        case .integrityMismatch(let m): return "BLAKE3 integrity verification failure: \(m)"
        case .diskDisconnected(let m): return "Disk disconnected abruptly: \(m)"
        case .sourceModified(let m): return "Source files mutated during copy: \(m)"
        case .unsupportedMetadata(let m): return "Metadata not supported on target filesystem: \(m)"
        case .invalidState(let m): return "Invalid lifecycle state transition: \(m)"
        case .recoveryFailed(let m): return "Recovery operation failed: \(m)"
        case .internalPanic(let m): return "Internal engine error: \(m)"
        case .unknown(let c, let m): return "VOLTERRA error [code \(c)]: \(m)"
        }
    }
}
