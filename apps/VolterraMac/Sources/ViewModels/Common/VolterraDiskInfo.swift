//
// VolterraDiskInfo.swift
// VolterraMac
//

import Foundation

/// Physical storage device metadata discovered via DiskArbitration / IOKit.
public struct VolterraDiskInfo: Identifiable, Hashable, Sendable {
    public var id: String { deviceId }
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
        isApfsSynthesized: Bool = false,
        parentBsdName: String = "",
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

    public var mediaTypeDescription: String {
        if isRotational {
            return "Mechanical HDD"
        } else {
            return "Solid State (SSD)"
        }
    }

    public var capacityFormatted: String {
        let total = ByteCountFormatter.string(fromByteCount: Int64(totalBytes), countStyle: .file)
        let free = ByteCountFormatter.string(fromByteCount: Int64(freeBytes), countStyle: .file)
        return "\(free) available of \(total)"
    }
}

public typealias DiskDeviceProfile = VolterraDiskInfo
