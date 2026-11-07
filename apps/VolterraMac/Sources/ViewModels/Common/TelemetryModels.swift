//
// TelemetryModels.swift
// VolterraMac
//

import Foundation

/// Bottleneck diagnosis classification code.
public enum BottleneckType: Int, CaseIterable, Sendable {
    case nominal = 0
    case sourceRead = 1
    case destWrite = 2
    case interface = 3
    case metadata = 4
    case smallFile = 5
    case cpuIntegrity = 6
    case contention = 7
    case errorDegradation = 8
    case unknown = 9

    public var title: String {
        switch self {
        case .nominal: return "Nominal"
        case .sourceRead: return "Source Read Limited"
        case .destWrite: return "Destination Write Limited"
        case .interface: return "Interface / Bus Constrained"
        case .metadata: return "Filesystem / Metadata Constrained"
        case .smallFile: return "Small File Overhead"
        case .cpuIntegrity: return "CPU / Hashing Constrained"
        case .contention: return "I/O Contention"
        case .errorDegradation: return "Sector Error Degradation"
        case .unknown: return "Analyzing Performance"
        }
    }

    public var displayName: String {
        title
    }

    public var statusTag: String {
        switch self {
        case .nominal: return "Optimal"
        case .sourceRead: return "Read Bottleneck"
        case .destWrite: return "Write Bottleneck"
        case .interface: return "Bus Limit"
        case .metadata: return "FS Overhead"
        case .smallFile: return "Small Files"
        case .cpuIntegrity: return "Hashing"
        case .contention: return "Contention"
        case .errorDegradation: return "Error Degradation"
        case .unknown: return "Profiling"
        }
    }
}

public typealias BottleneckCode = BottleneckType

public struct BottleneckInfo: Equatable, Sendable {
    public let type: BottleneckType
    public let title: String
    public let userMessage: String
    public let isWarning: Bool

    public init(type: BottleneckType, title: String, userMessage: String, isWarning: Bool) {
        self.type = type
        self.title = title
        self.userMessage = userMessage
        self.isWarning = isWarning
    }

    public static let nominal = BottleneckInfo(
        type: .nominal,
        title: "Nominal",
        userMessage: "All I/O queues balanced. High throughput sustained.",
        isWarning: false
    )
}

/// Rolling throughput chart sample point.
public struct ThroughputPoint: Identifiable, Sendable {
    public let id: UUID
    public let timestamp: Date
    public let instantaneousMbps: Double
    public let ewma1sMbps: Double
    public let ewma5sMbps: Double
    public let ewma30sMbps: Double
    public let physicalThroughputMbps: Double

    public init(
        id: UUID = UUID(),
        timestamp: Date = Date(),
        instantaneousMbps: Double,
        ewma1sMbps: Double,
        ewma5sMbps: Double,
        ewma30sMbps: Double,
        physicalThroughputMbps: Double
    ) {
        self.id = id
        self.timestamp = timestamp
        self.instantaneousMbps = instantaneousMbps
        self.ewma1sMbps = ewma1sMbps
        self.ewma5sMbps = ewma5sMbps
        self.ewma30sMbps = ewma30sMbps
        self.physicalThroughputMbps = physicalThroughputMbps
    }
}

/// Latency trend sample point.
public struct LatencyPoint: Identifiable, Sendable {
    public let id: UUID
    public let timestamp: Date
    public let p50ReadMs: Double
    public let p95ReadMs: Double
    public let p50WriteMs: Double
    public let p95WriteMs: Double

    public init(
        id: UUID = UUID(),
        timestamp: Date = Date(),
        p50ReadMs: Double,
        p95ReadMs: Double,
        p50WriteMs: Double,
        p95WriteMs: Double
    ) {
        self.id = id
        self.timestamp = timestamp
        self.p50ReadMs = p50ReadMs
        self.p95ReadMs = p95ReadMs
        self.p50WriteMs = p50WriteMs
        self.p95WriteMs = p95WriteMs
    }
}

/// Discrete failed logical byte range.
public struct LogicalExtentItem: Identifiable, Hashable, Sendable {
    public let id: UUID
    public let offset: UInt64
    public let length: UInt64
    public let errorCode: Int32

    public init(id: UUID = UUID(), offset: UInt64, length: UInt64, errorCode: Int32) {
        self.id = id
        self.offset = offset
        self.length = length
        self.errorCode = errorCode
    }

    public var endOffset: UInt64 { offset + length }

    public var formattedRange: String {
        let start = ByteCountFormatter.string(fromByteCount: Int64(offset), countStyle: .file)
        let end = ByteCountFormatter.string(fromByteCount: Int64(endOffset), countStyle: .file)
        return "[\(start) .. \(end)] (\(length) bytes)"
    }

    public var errorName: String {
        switch errorCode {
        case 5: return "EIO (Input/Output Error)"
        case 28: return "ENOSPC (No Space Left)"
        case 13: return "EACCES (Permission Denied)"
        case 19: return "ENODEV (No Such Device)"
        default: return "Error code \(errorCode)"
        }
    }
}

/// Quarantined failing file entry in the Error Center.
public struct QuarantinedFileItem: Identifiable, Hashable, Sendable {
    public let id: Int64
    public let relativePath: String
    public let errorCategory: String
    public let errorMessage: String
    public let totalSizeBytes: UInt64
    public let attempts: UInt32
    public let state: String
    public let extents: [LogicalExtentItem]

    public init(
        id: Int64,
        relativePath: String,
        errorCategory: String,
        errorMessage: String,
        totalSizeBytes: UInt64,
        attempts: UInt32,
        state: String,
        extents: [LogicalExtentItem]
    ) {
        self.id = id
        self.relativePath = relativePath
        self.errorCategory = errorCategory
        self.errorMessage = errorMessage
        self.totalSizeBytes = totalSizeBytes
        self.attempts = attempts
        self.state = state
        self.extents = extents
    }

    public var fileId: Int64 { id }
    public var errorKind: String { errorCategory }

    public var fileName: String {
        URL(fileURLWithPath: relativePath).lastPathComponent
    }

    public var directoryPath: String {
        URL(fileURLWithPath: relativePath).deletingLastPathComponent().path
    }

    public var formattedSize: String {
        ByteCountFormatter.string(fromByteCount: Int64(totalSizeBytes), countStyle: .file)
    }
}

public struct CapacityWarning: Sendable {
    public let requiredBytes: UInt64
    public let availableBytes: UInt64

    public init(requiredBytes: UInt64, availableBytes: UInt64) {
        self.requiredBytes = requiredBytes
        self.availableBytes = availableBytes
    }

    public var shortageBytes: UInt64 {
        requiredBytes > availableBytes ? requiredBytes - availableBytes : 0
    }

    public var requiredFormatted: String {
        ByteCountFormatter.string(fromByteCount: Int64(requiredBytes), countStyle: .file)
    }

    public var availableFormatted: String {
        ByteCountFormatter.string(fromByteCount: Int64(availableBytes), countStyle: .file)
    }

    public var shortageFormatted: String {
        ByteCountFormatter.string(fromByteCount: Int64(shortageBytes), countStyle: .file)
    }
}

public struct MetadataWarning: Sendable {
    public let message: String
    public let sourceFs: String
    public let targetFs: String

    public init(message: String, sourceFs: String = "APFS", targetFs: String = "exFAT") {
        self.message = message
        self.sourceFs = sourceFs
        self.targetFs = targetFs
    }
}
