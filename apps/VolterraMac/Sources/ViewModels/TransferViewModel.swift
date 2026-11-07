//
// TransferViewModel.swift
// VolterraMac
//

import Foundation
import SwiftUI
import Combine
import AppKit
import CVolterra
import VolterraBridge

@MainActor
public final class TransferViewModel: ObservableObject {
    // MARK: - Published Job State Machine
    @Published public private(set) var state: TransferJobState = .idle
    @Published public var activeJobId: String = ""

    // MARK: - Paths & Mode Configuration
    @Published public var sourceURL: URL? = nil {
        didSet {
            allowOverwriteConfirmed = false
            Task { await runPreFlightCheck() }
        }
    }
    @Published public var destinationURL: URL? = nil {
        didSet {
            allowOverwriteConfirmed = false
            Task { await runPreFlightCheck() }
        }
    }
    @Published public var sourceDisk: VolterraDiskInfo? = nil
    @Published public var destinationDisk: VolterraDiskInfo? = nil
    @Published public var selectedMode: TransferMode = .smartHybrid

    // MARK: - Advanced Options
    @Published public var directIO: Bool = false
    @Published public var verifyReadback: Bool = true
    @Published public var memoryBudgetMB: Int = 256

    // MARK: - Pre-Flight Capacity & Filesystem Warnings
    @Published public private(set) var isPreFlightChecking: Bool = false
    @Published public private(set) var preFlightValid: Bool = false
    @Published public private(set) var hasInsufficientSpace: Bool = false
    @Published public private(set) var hasCrossFilesystemWarning: Bool = false
    @Published public private(set) var capacityWarning: CapacityWarning? = nil
    @Published public private(set) var metadataWarning: MetadataWarning? = nil
    @Published public private(set) var preFlightWarning: String? = nil
    @Published public private(set) var crossFilesystemDetails: String? = nil

    // MARK: - Overwrite Approval Dialog State
    @Published public var showOverwriteDialog: Bool = false
    @Published public private(set) var collidingFilesCount: Int = 0
    @Published public private(set) var collidingSamplePaths: [String] = []
    @Published public var allowOverwriteConfirmed: Bool = false
    @Published public var destinationCollisionPath: String? = nil

    // MARK: - Live Throttled Transfer Progress (60Hz)
    @Published public private(set) var totalFiles: UInt64 = 0
    @Published public private(set) var processedFiles: UInt64 = 0
    @Published public private(set) var failedFiles: UInt64 = 0
    @Published public private(set) var skippedFiles: UInt64 = 0
    @Published public private(set) var totalBytes: UInt64 = 0
    @Published public private(set) var transferredBytes: UInt64 = 0
    @Published public private(set) var fractionCompleted: Double = 0.0
    @Published public private(set) var percentComplete: Double = 0.0
    @Published public private(set) var currentFileName: String = ""
    @Published public private(set) var currentSpeedBps: Double = 0.0
    @Published public private(set) var smoothedSpeedBps: Double = 0.0
    @Published public private(set) var etaSeconds: UInt64 = 0
    @Published public private(set) var statusMessage: String = "Ready to configure transfer."
    @Published public var errorMessage: String? = nil
    @Published public private(set) var quarantinedCount: Int = 0

    // MARK: - Weak References to Peer ViewModels for Direct Dispatch
    public weak var performanceViewModel: PerformanceViewModel?
    public weak var errorCenterViewModel: ErrorCenterViewModel?

    // MARK: - Dependencies & Throttler
    private var throttler: DisplayThrottler?
    private var activeJobController: VolterraJobController?
    private var activeJobHandle: OpaquePointer?
    private var eventConsumerTask: Task<Void, Never>?
    private var discoveredDisks: [VolterraDiskInfo] = []

    public init() {
        self.throttler = DisplayThrottler { [weak self] state in
            self?.applyThrottledUpdate(state)
        }
        refreshDisks()
    }

    deinit {
        throttler?.stop()
        eventConsumerTask?.cancel()
    }

    // MARK: - Computed Properties for UI Helpers
    public var completedFiles: UInt64 { processedFiles }
    public var isOverwriteApproved: Bool { allowOverwriteConfirmed }
    public var destinationHasExistingFiles: Bool { collidingFilesCount > 0 }

    public var canStartTransfer: Bool {
        state == .idle && sourceURL != nil && destinationURL != nil && preFlightValid && !hasInsufficientSpace
    }

    public var isActive: Bool {
        state == .scanning || state == .running || state == .paused
    }

    public var isPaused: Bool {
        state == .paused
    }

    public var canPauseOrResume: Bool {
        state == .running || state == .paused
    }

    public var isETAStable: Bool {
        etaSeconds > 0 && smoothedSpeedBps > 1024 * 100
    }

    public var statusColor: Color {
        switch state {
        case .idle: return .gray
        case .scanning: return .blue
        case .running: return .green
        case .paused: return .orange
        case .completed: return .teal
        case .failed: return .red
        case .cancelled: return .secondary
        }
    }

    public var formattedProcessedBytes: String {
        ByteCountFormatter.string(fromByteCount: Int64(transferredBytes), countStyle: .file)
    }

    public var formattedTotalBytes: String {
        ByteCountFormatter.string(fromByteCount: Int64(totalBytes), countStyle: .file)
    }

    public var formattedETA: String {
        if state == .completed { return "Complete" }
        if !isActive || etaSeconds == 0 { return "--:--" }
        let hours = etaSeconds / 3600
        let minutes = (etaSeconds % 3600) / 60
        let seconds = etaSeconds % 60
        if hours > 0 {
            return String(format: "%02dh %02dm %02ds", hours, minutes, seconds)
        } else {
            return String(format: "%02dm %02ds", minutes, seconds)
        }
    }

    // MARK: - Native macOS Open Panels
    public func chooseSourceFolder() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.allowsMultipleSelection = false
        panel.prompt = "Select Source Folder"
        if panel.runModal() == .OK, let url = panel.url {
            selectSource(url: url)
        }
    }

    public func chooseDestinationFolder() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.canCreateDirectories = true
        panel.allowsMultipleSelection = false
        panel.prompt = "Select Destination Folder"
        if panel.runModal() == .OK, let url = panel.url {
            selectDestination(url: url)
        }
    }

    // MARK: - Disk Discovery & Selection
    public func refreshDisks() {
        do {
            let disks = try VolterraBridge.discoverDisks()
            self.discoveredDisks = disks.map { d in
                VolterraDiskInfo(
                    deviceId: d.deviceId,
                    bsdName: d.bsdName,
                    mountPoint: d.mountPoint,
                    volumeName: d.volumeName,
                    fsType: d.fsType,
                    totalBytes: d.totalBytes,
                    freeBytes: d.freeBytes,
                    isRotational: d.isRotational,
                    isRemovable: d.isRemovable,
                    isApfsSynthesized: d.isApfsSynthesized,
                    parentBsdName: d.parentBsdName,
                    model: d.model,
                    vendor: d.vendor,
                    transport: d.transport
                )
            }
            if let src = sourceURL {
                self.sourceDisk = matchDisk(for: src)
            }
            if let dst = destinationURL {
                self.destinationDisk = matchDisk(for: dst)
            }
        } catch {
            self.errorMessage = "Failed to discover disks: \(error.localizedDescription)"
        }
    }

    public func selectSource(url: URL) {
        self.sourceURL = url
        self.sourceDisk = matchDisk(for: url)
    }

    public func selectDestination(url: URL) {
        self.destinationURL = url
        self.destinationDisk = matchDisk(for: url)
    }

    private func matchDisk(for url: URL) -> VolterraDiskInfo? {
        let canonicalPath = url.resolvingSymlinksInPath().path
        let standardPath = url.path
        return discoveredDisks
            .filter { canonicalPath.hasPrefix($0.mountPoint) || standardPath.hasPrefix($0.mountPoint) }
            .max(by: { $0.mountPoint.count < $1.mountPoint.count })
    }

    // MARK: - Pre-Flight Checks
    public func runPreFlightCheck() async {
        guard let src = sourceURL, let dst = destinationURL else {
            preFlightValid = false
            hasInsufficientSpace = false
            hasCrossFilesystemWarning = false
            capacityWarning = nil
            metadataWarning = nil
            preFlightWarning = nil
            return
        }

        isPreFlightChecking = true
        defer { isPreFlightChecking = false }

        let fm = FileManager.default
        guard fm.fileExists(atPath: src.path) else {
            preFlightValid = false
            preFlightWarning = "Source directory does not exist."
            return
        }
        guard fm.fileExists(atPath: dst.path) else {
            preFlightValid = false
            preFlightWarning = "Destination directory does not exist."
            return
        }

        // Capacity check
        let canonicalDstPath = dst.resolvingSymlinksInPath().path
        var freeBytes: UInt64 = 0
        var foundFsAttrs = false

        if let attrs = try? fm.attributesOfFileSystem(forPath: canonicalDstPath),
           let free = attrs[.systemFreeSize] as? NSNumber {
            freeBytes = free.uint64Value
            foundFsAttrs = true
        } else if let attrs = try? fm.attributesOfFileSystem(forPath: dst.path),
                  let free = attrs[.systemFreeSize] as? NSNumber {
            freeBytes = free.uint64Value
            foundFsAttrs = true
        }

        let srcDisk = matchDisk(for: src)
        let dstDisk = matchDisk(for: dst)
        self.sourceDisk = srcDisk
        self.destinationDisk = dstDisk

        if foundFsAttrs && freeBytes == 0 {
            // Preserve genuine 0 free bytes reported by filesystem attributes; do not allow fallback disk matching to overwrite it with root disk space
        } else if let dDisk = dstDisk, dDisk.freeBytes > 0 {
            if !foundFsAttrs {
                freeBytes = dDisk.freeBytes
            } else if dDisk.mountPoint != "/" && (canonicalDstPath.hasPrefix(dDisk.mountPoint) || dst.path.hasPrefix(dDisk.mountPoint)) {
                freeBytes = dDisk.freeBytes
            }
        }

        // Quick calculation of source size estimate
        var sourceBytes: UInt64 = 0
        if let enumerator = fm.enumerator(at: src, includingPropertiesForKeys: [.fileSizeKey, .isRegularFileKey]) {
            while let fileURL = enumerator.nextObject() as? URL {
                if let resourceValues = try? fileURL.resourceValues(forKeys: [.fileSizeKey, .isRegularFileKey]),
                   resourceValues.isRegularFile == true,
                   let fileSize = resourceValues.fileSize {
                    sourceBytes += UInt64(fileSize)
                }
            }
        }

        if sourceBytes > 0 && (freeBytes == 0 || sourceBytes > freeBytes) {
            hasInsufficientSpace = true
            capacityWarning = CapacityWarning(requiredBytes: sourceBytes, availableBytes: freeBytes)
        } else {
            hasInsufficientSpace = false
            capacityWarning = nil
        }

        // Cross-filesystem warning (APFS -> exFAT)
        if let srcFs = srcDisk?.fsType.lowercased(), let dstFs = dstDisk?.fsType.lowercased() {
            if srcFs.contains("apfs") && (dstFs.contains("exfat") || dstFs.contains("fat32") || dstFs.contains("msdos")) {
                hasCrossFilesystemWarning = true
                metadataWarning = MetadataWarning(
                    message: "Destination is formatted as exFAT. macOS Extended Attributes (xattr), Finder tags, creation dates, and resource forks cannot be preserved on this filesystem.",
                    sourceFs: srcDisk?.fsType ?? "APFS",
                    targetFs: dstDisk?.fsType ?? "exFAT"
                )
            } else {
                hasCrossFilesystemWarning = false
                metadataWarning = nil
            }
        }

        // Overwrite collision detection
        let collisionCandidates = detectCollisions(source: src, destination: dst)
        if !collisionCandidates.isEmpty {
            self.collidingFilesCount = collisionCandidates.count
            self.collidingSamplePaths = Array(collisionCandidates.prefix(5))
            self.destinationCollisionPath = collisionCandidates.first
        } else {
            self.collidingFilesCount = 0
            self.collidingSamplePaths = []
            self.destinationCollisionPath = nil
        }

        preFlightValid = true
        preFlightWarning = nil
    }

    private func detectCollisions(source: URL, destination: URL) -> [String] {
        let fm = FileManager.default
        guard let sourceItems = try? fm.contentsOfDirectory(atPath: source.path) else { return [] }
        var collisions: [String] = []
        for item in sourceItems {
            let targetPath = (destination.path as NSString).appendingPathComponent(item)
            if fm.fileExists(atPath: targetPath) {
                collisions.append(item)
            }
        }
        return collisions
    }

    // MARK: - Lifecycle Controls
    public func startTransfer() {
        guard state == .idle, let src = sourceURL, let dst = destinationURL else { return }

        if collidingFilesCount > 0 && !allowOverwriteConfirmed {
            showOverwriteDialog = true
            return
        }

        let bridgeMode: VolterraBridgeMode
        switch selectedMode {
        case .turbo: bridgeMode = .turbo
        case .smartHybrid: bridgeMode = .smartHybrid
        case .safeRecovery: bridgeMode = .safeRecovery
        }

        do {
            let controller = try VolterraBridge.createJob(
                sourcePath: src.path,
                destPath: dst.path,
                mode: bridgeMode,
                chunkSize: 8 * 1024 * 1024,
                directIO: directIO,
                verifyReadback: verifyReadback,
                memoryBudget: memoryBudgetMB * 1024 * 1024,
                allowOverwrite: allowOverwriteConfirmed,
                journalPath: nil
            )

            self.activeJobController = controller
            self.activeJobId = UUID().uuidString.prefix(8).uppercased() + "-JOB"
            self.state = .scanning
            self.statusMessage = "Scanning source directory manifest..."

            Task {
                if let rawHandle = await controller.getRawHandle() {
                    await MainActor.run {
                        self.activeJobHandle = rawHandle
                        self.errorCenterViewModel?.bindJob(handle: rawHandle)
                    }
                }
            }

            // Start 60Hz display throttler
            throttler?.start()

            // Subscribe to async events
            startEventSubscription(controller: controller)

            // Start job execution
            Task {
                do {
                    try await controller.start()
                } catch {
                    await MainActor.run {
                        self.errorMessage = error.localizedDescription
                        self.state = .failed
                        self.allowOverwriteConfirmed = false
                        self.cleanupJob()
                    }
                }
            }
        } catch {
            self.errorMessage = error.localizedDescription
            self.state = .failed
            self.allowOverwriteConfirmed = false
            self.cleanupJob()
        }
    }

    public func approveOverwriteAndStart() {
        self.allowOverwriteConfirmed = true
        self.showOverwriteDialog = false
        startTransfer()
    }

    public func confirmOverwriteAndStart() {
        approveOverwriteAndStart()
    }

    public func pauseTransfer() {
        guard state.canPause, let controller = activeJobController else { return }
        Task {
            do {
                try await controller.pause()
                await MainActor.run {
                    self.state = .paused
                    self.statusMessage = "Transfer paused by user."
                }
            } catch {
                await MainActor.run {
                    self.errorMessage = error.localizedDescription
                }
            }
        }
    }

    public func resumeTransfer() {
        guard state.canResume, let controller = activeJobController else { return }
        Task {
            do {
                try await controller.resume()
                await MainActor.run {
                    self.state = .running
                    self.statusMessage = "Transfer resumed."
                }
            } catch {
                await MainActor.run {
                    self.errorMessage = error.localizedDescription
                }
            }
        }
    }

    public func togglePauseResume() {
        if state == .running {
            pauseTransfer()
        } else if state == .paused {
            resumeTransfer()
        }
    }

    public func requestCancel() {
        cancelTransfer()
    }

    public func cancelTransfer() {
        self.allowOverwriteConfirmed = false
        self.showOverwriteDialog = false
        self.destinationCollisionPath = nil
        self.state = .cancelled
        self.statusMessage = "Transfer cancelled by user."
        self.errorCenterViewModel?.bindJob(handle: nil)
        self.activeJobHandle = nil

        guard let controller = activeJobController else {
            self.cleanupJob()
            return
        }

        Task {
            try? await controller.cancel()
            await MainActor.run {
                self.cleanupJob()
            }
        }
    }

    public func resetToIdle() {
        cleanupJob()
        self.state = .idle
        self.activeJobId = ""
        self.currentFileName = ""
        self.allowOverwriteConfirmed = false
        self.showOverwriteDialog = false
        self.collidingFilesCount = 0
        self.collidingSamplePaths = []
        self.destinationCollisionPath = nil
        self.transferredBytes = 0
        self.totalBytes = 0
        self.processedFiles = 0
        self.totalFiles = 0
        self.failedFiles = 0
        self.skippedFiles = 0
        self.fractionCompleted = 0.0
        self.percentComplete = 0.0
        self.currentSpeedBps = 0.0
        self.smoothedSpeedBps = 0.0
        self.etaSeconds = 0
        self.statusMessage = "Ready to configure transfer."
        self.errorMessage = nil
        self.quarantinedCount = 0
        self.performanceViewModel?.clearHistory()

        if sourceURL != nil && destinationURL != nil {
            Task {
                await self.runPreFlightCheck()
            }
        }
    }

    private func cleanupJob() {
        throttler?.stop()
        eventConsumerTask?.cancel()
        eventConsumerTask = nil
        self.errorCenterViewModel?.bindJob(handle: nil)
        self.activeJobHandle = nil
        self.allowOverwriteConfirmed = false
        self.showOverwriteDialog = false
        self.destinationCollisionPath = nil
        if let controller = activeJobController {
            Task {
                await controller.close()
            }
            self.activeJobController = nil
        }
    }

    // MARK: - Event Subscription & Throttled vs Immediate Dispatch
    private func startEventSubscription(controller: VolterraJobController) {
        eventConsumerTask = Task { [weak self] in
            let eventStream = await controller.events()
            for await event in eventStream {
                guard let self = self else { break }

                switch event.kind {
                case .stateChanged, .fileQuarantined, .diskWarning, .jobCompleted, .jobFailed:
                    // Immediate Lane
                    await MainActor.run {
                        self.handleDiscreteEvent(type: Int(event.kind.rawValue), message: event.message, fileId: event.fileId)
                    }
                case .scanningProgress:
                    self.throttler?.updateScanningProgress(files: event.fileId > 0 ? UInt64(event.fileId) : 0, bytes: event.totalBytes)
                case .chunkProgress:
                    self.throttler?.updateChunkProgress(fileId: event.fileId, bytesCopied: event.progressBytes, totalBytes: event.totalBytes)
                case .fileStarted:
                    self.throttler?.updateActiveFile(path: event.message)
                case .telemetryUpdated, .bottleneckDetected, .fileCompleted, .scanningComplete:
                    // High-frequency telemetry update
                    if let snap = await controller.getSnapshot(), let tel = await controller.getTelemetry() {
                        self.throttler?.updateTelemetry(
                            ewma1s: tel.ewma1sMbps,
                            ewma5s: tel.ewma5sMbps,
                            ewma30s: tel.ewma30sMbps,
                            instantMbps: tel.instantaneousMbps,
                            sourceReadMbps: tel.sourceReadMbps,
                            destWriteMbps: tel.destWriteMbps,
                            physicalMbps: tel.physicalThroughputMbps,
                            p50Read: tel.p50ReadLatencyMs,
                            p95Read: tel.p95ReadLatencyMs,
                            p50Write: tel.p50WriteLatencyMs,
                            p95Write: tel.p95WriteLatencyMs,
                            chunkSize: tel.activeChunkSizeBytes,
                            concurrency: tel.activeConcurrency,
                            bufferUtil: tel.bufferUtilizationPct,
                            bottleneckCode: Int(tel.bottleneckCode),
                            bottleneckMsg: tel.bottleneckMessage,
                            isCacheBurst: tel.isCacheBurst,
                            isStalled: tel.isStalled,
                            processedBytes: snap.processedBytes,
                            totalBytes: snap.totalBytes,
                            completedFiles: snap.processedFiles,
                            totalFiles: snap.totalFiles,
                            failedFiles: snap.failedFiles,
                            etaSeconds: snap.etaSeconds
                        )
                    }
                }
            }
        }
    }

    /// Handles immediate lane discrete events
    public func handleDiscreteEvent(type: Int, message: String, fileId: Int64) {
        switch type {
        case 1: // StateChanged
            let target = message.components(separatedBy: " to ").last ?? message
            let isCurrentTerminal = (self.state == .cancelled || self.state == .completed || self.state == .failed)
            let lower = target.lowercased()

            if lower.contains("cancelled") {
                self.state = .cancelled
                self.statusMessage = "Transfer cancelled by user."
                self.cleanupJob()
            } else if lower.contains("paused") {
                guard !isCurrentTerminal else { return }
                self.state = .paused
                self.statusMessage = "Transfer paused."
            } else if lower.contains("completed") {
                self.state = .completed
                self.statusMessage = "Transfer completed and verified with BLAKE3."
                self.cleanupJob()
            } else if lower.contains("failed") {
                self.state = .failed
                self.errorMessage = message
                self.cleanupJob()
            } else if lower.contains("running") {
                guard !isCurrentTerminal else { return }
                self.state = .running
                self.statusMessage = "Copying files..."
            } else if lower.contains("scanning") {
                guard !isCurrentTerminal else { return }
                self.state = .scanning
                self.statusMessage = "Scanning source directory manifest..."
            }
        case 7: // FileQuarantined
            self.failedFiles += 1
            self.quarantinedCount += 1
            self.statusMessage = "Sector read error. File quarantined."
            self.errorCenterViewModel?.refreshErrors()
        case 10: // DiskWarning
            self.errorMessage = "Disk Warning: \(message)"
        case 11: // JobCompleted
            if self.state == .cancelled { return }
            self.state = .completed
            self.statusMessage = "All files successfully verified with BLAKE3."
            self.cleanupJob()
        case 12: // JobFailed
            if self.state == .cancelled { return }
            self.state = .failed
            self.errorMessage = message
            self.cleanupJob()
        default:
            break
        }
    }

    /// Receives 60Hz throttled updates from DisplayThrottler
    private func applyThrottledUpdate(_ update: CoalescedTelemetryState) {
        self.transferredBytes = update.processedBytes
        self.totalBytes = update.totalBytes
        self.processedFiles = update.processedFiles
        self.totalFiles = update.totalFiles
        self.failedFiles = update.failedFiles
        self.fractionCompleted = update.totalBytes > 0 ? Double(update.processedBytes) / Double(update.totalBytes) : 0.0
        self.percentComplete = update.percentComplete
        self.currentSpeedBps = update.currentSpeedBps
        self.smoothedSpeedBps = update.rollingSpeedBps
        self.etaSeconds = update.etaSeconds
        if !update.currentFileName.isEmpty {
            self.currentFileName = update.currentFileName
        }
        self.performanceViewModel?.applyTelemetryUpdate(update)
    }
}
