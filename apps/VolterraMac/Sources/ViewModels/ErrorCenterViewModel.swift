//
// ErrorCenterViewModel.swift
// VolterraMac
//

import Foundation
import SwiftUI
import Combine
import CVolterra
import VolterraBridge

@MainActor
public final class ErrorCenterViewModel: ObservableObject {
    @Published public private(set) var quarantinedErrors: [QuarantinedFileItem] = []
    @Published public var selectedItem: QuarantinedFileItem? = nil
    @Published public var searchQuery: String = ""
    @Published public var isPerformingAction: Bool = false
    @Published public var actionFeedbackMessage: String? = nil

    private var activeJobHandle: OpaquePointer?
    private var activeJobController: VolterraJobController?

    public init() {}

    public var hasActiveJob: Bool {
        activeJobHandle != nil
    }

    public var quarantinedFiles: [QuarantinedFileItem] { quarantinedErrors }
    public var quarantinedCount: Int { quarantinedErrors.count }

    public func bindJob(handle: OpaquePointer?) {
        self.activeJobHandle = handle
        if handle == nil {
            self.activeJobController = nil
        } else {
            refreshErrors()
        }
    }

    public func bindJobController(controller: VolterraJobController?) {
        self.activeJobController = controller
        if let controller = controller {
            Task {
                let raw = await controller.getRawHandle()
                await MainActor.run {
                    self.activeJobHandle = raw
                    self.refreshErrors()
                }
            }
        } else {
            self.activeJobHandle = nil
        }
    }

    public var filteredErrors: [QuarantinedFileItem] {
        if searchQuery.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            return quarantinedErrors
        }
        let query = searchQuery.lowercased()
        return quarantinedErrors.filter {
            $0.relativePath.lowercased().contains(query) ||
            $0.errorMessage.lowercased().contains(query)
        }
    }

    public var totalFailedBytes: UInt64 {
        quarantinedErrors.reduce(0) { $0 + $1.totalSizeBytes }
    }

    /// Fetches quarantined error list and failed extents from C ABI.
    public func refreshErrors() {
        guard let job = activeJobHandle else { return }
        guard let listPtr = volterra_get_errors(job) else { return }
        defer { volterra_error_list_free(listPtr) }

        var items: [QuarantinedFileItem] = []
        let count = Int(listPtr.pointee.count)

        for i in 0..<count {
            let errorItem = listPtr.pointee.items[i]
            let path = errorItem.path != nil ? String(cString: errorItem.path) : "Unknown file"
            let msg = errorItem.error_message != nil ? String(cString: errorItem.error_message) : "I/O error"

            var extents: [LogicalExtentItem] = []
            let extCount = Int(errorItem.failed_extents_count)
            if extCount > 0 && errorItem.extents != nil {
                for j in 0..<extCount {
                    let ext = errorItem.extents[j]
                    extents.append(LogicalExtentItem(
                        offset: ext.offset,
                        length: ext.length,
                        errorCode: ext.error_code
                    ))
                }
            }

            items.append(QuarantinedFileItem(
                id: errorItem.file_id,
                relativePath: path,
                errorCategory: "Read I/O Error (EIO)",
                errorMessage: msg,
                totalSizeBytes: errorItem.total_size_bytes,
                attempts: errorItem.attempts,
                state: "Quarantined",
                extents: extents
            ))
        }

        self.quarantinedErrors = items
        if let currentSelected = selectedItem {
            self.selectedItem = items.first(where: { $0.id == currentSelected.id })
        }
    }

    // MARK: - Per-File Actions
    public func skipFile(fileId: Int64) {
        guard let job = activeJobHandle else {
            actionFeedbackMessage = "No active job available"
            return
        }
        isPerformingAction = true
        let ret = volterra_skip_file(job, fileId)
        isPerformingAction = false
        if ret == 0 {
            actionFeedbackMessage = "File skipped successfully."
            refreshErrors()
        } else {
            actionFeedbackMessage = "Failed to skip file (code: \(ret))."
        }
    }

    public func retryFile(fileId: Int64) {
        guard let job = activeJobHandle else {
            actionFeedbackMessage = "No active job available"
            return
        }
        isPerformingAction = true
        let ret = volterra_retry_file(job, fileId)
        isPerformingAction = false
        if ret == 0 {
            actionFeedbackMessage = "File queued for retry."
            refreshErrors()
        } else {
            actionFeedbackMessage = "Failed to retry file (code: \(ret))."
        }
    }

    public func deferFile(fileId: Int64) {
        guard let job = activeJobHandle else {
            actionFeedbackMessage = "No active job available"
            return
        }
        isPerformingAction = true
        let ret = volterra_defer_file(job, fileId)
        isPerformingAction = false
        if ret == 0 {
            actionFeedbackMessage = "File deferred to second-pass recovery pool."
            refreshErrors()
        } else {
            actionFeedbackMessage = "Failed to defer file (code: \(ret))."
        }
    }

    // MARK: - Bulk Operations
    public func skipAllErrors() {
        guard let job = activeJobHandle else {
            actionFeedbackMessage = "No active job available"
            return
        }
        isPerformingAction = true
        let ret = volterra_skip_all_errors(job)
        isPerformingAction = false
        if ret == 0 {
            actionFeedbackMessage = "All quarantined files skipped."
            refreshErrors()
        } else {
            actionFeedbackMessage = "Bulk skip failed (code: \(ret))."
        }
    }

    public func retryAllFailed() {
        guard let job = activeJobHandle else {
            actionFeedbackMessage = "No active job available"
            return
        }
        isPerformingAction = true
        let ret = volterra_retry_all_failed(job)
        isPerformingAction = false
        if ret == 0 {
            actionFeedbackMessage = "All failed files queued for retry with reset budgets."
            refreshErrors()
        } else {
            actionFeedbackMessage = "Bulk retry failed (code: \(ret))."
        }
    }
}
