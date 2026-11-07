//
// HistoryViewModel.swift
// VolterraMac
//

import Foundation
import SwiftUI
import SQLite3
import AppKit
import CVolterra

public struct HistoricalJobItem: Identifiable, Hashable, Sendable {
    public let id: String
    public let sourceRoot: String
    public let targetRoot: String
    public let mode: String
    public let state: String
    public let totalFiles: UInt64
    public let completedFiles: UInt64
    public let totalBytes: UInt64
    public let copiedBytes: UInt64
    public let createdAt: String
    public let completedAt: String?
    public let errorCount: Int

    public init(
        id: String,
        sourceRoot: String,
        targetRoot: String,
        mode: String,
        state: String,
        totalFiles: UInt64,
        completedFiles: UInt64 = 0,
        totalBytes: UInt64,
        copiedBytes: UInt64,
        createdAt: String,
        completedAt: String?,
        errorCount: Int
    ) {
        self.id = id
        self.sourceRoot = sourceRoot
        self.targetRoot = targetRoot
        self.mode = mode
        self.state = state
        self.totalFiles = totalFiles
        self.completedFiles = completedFiles == 0 ? totalFiles : completedFiles
        self.totalBytes = totalBytes
        self.copiedBytes = copiedBytes
        self.createdAt = createdAt
        self.completedAt = completedAt
        self.errorCount = errorCount
    }

    public var jobId: String { id }
    public var status: String { state }

    public var progressPercent: Double {
        totalBytes > 0 ? min(100.0, (Double(copiedBytes) / Double(totalBytes)) * 100.0) : 0.0
    }

    public var formattedSize: String {
        ByteCountFormatter.string(fromByteCount: Int64(copiedBytes), countStyle: .file)
    }

    public var formattedBytes: String {
        formattedSize
    }

    public var formattedDate: String {
        // e.g. "2026-11-01T10:15:30Z" -> "2026-11-01 10:15"
        if createdAt.count >= 16 {
            return String(createdAt.prefix(16)).replacingOccurrences(of: "T", with: " ")
        }
        return createdAt
    }

    public var statusColor: Color {
        switch state.lowercased() {
        case "completed": return .green
        case "running": return .blue
        case "paused": return .orange
        case "failed": return .red
        case "partial": return .yellow
        default: return .gray
        }
    }
}

@MainActor
public final class HistoryViewModel: ObservableObject {
    @Published public private(set) var historicalJobs: [HistoricalJobItem] = []
    @Published public var selectedJob: HistoricalJobItem? = nil
    @Published public private(set) var isLoading: Bool = false
    @Published public var errorMessage: String? = nil
    @Published public var exportMessage: String? = nil
    @Published public var maskUserPaths: Bool = true

    private var activeJobHandle: OpaquePointer?
    private var customDatabasePath: String?

    public init(databasePath: String? = nil) {
        self.customDatabasePath = databasePath
        loadHistory()
    }

    public var jobs: [HistoricalJobItem] { historicalJobs }

    public func bindJob(handle: OpaquePointer?) {
        self.activeJobHandle = handle
    }

    public func sanitizePath(_ path: String) -> String {
        guard maskUserPaths else { return path }
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        if path.hasPrefix(home) {
            return "~" + path.dropFirst(home.count)
        }
        return path
    }

    /// Resolves the SQLite database location.
    private func resolveDatabasePath() -> String {
        if let custom = customDatabasePath {
            return custom
        }
        let fm = FileManager.default
        let appSupport = fm.urls(for: .applicationSupportDirectory, in: .userDomainMask).first!
        let volterraDir = appSupport.appendingPathComponent("Volterra", isDirectory: true)
        try? fm.createDirectory(at: volterraDir, withIntermediateDirectories: true)
        return volterraDir.appendingPathComponent("volterra_journal.db").path
    }

    /// Queries SQLite WAL database for previous transfer jobs.
    public func loadHistory() {
        isLoading = true
        defer { isLoading = false }

        let dbPath = resolveDatabasePath()
        guard FileManager.default.fileExists(atPath: dbPath) else {
            self.historicalJobs = []
            return
        }

        var db: OpaquePointer?
        let openFlags = SQLITE_OPEN_READONLY | SQLITE_OPEN_URI
        guard sqlite3_open_v2(dbPath, &db, openFlags, nil) == SQLITE_OK else {
            errorMessage = "Failed to open historical journal database."
            return
        }
        defer { sqlite3_close(db) }

        let query = """
        SELECT j.id, j.source_root, j.target_root, j.mode, j.state, 
               j.total_files, j.total_bytes, j.copied_bytes, 
               j.created_at, j.completed_at,
               (SELECT COUNT(*) FROM errors e WHERE e.job_id = j.id) AS err_count
        FROM jobs j
        ORDER BY j.created_at DESC;
        """

        var stmt: OpaquePointer?
        guard sqlite3_prepare_v2(db, query, -1, &stmt, nil) == SQLITE_OK else {
            errorMessage = "Failed to prepare query on historical jobs."
            return
        }
        defer { sqlite3_finalize(stmt) }

        var results: [HistoricalJobItem] = []
        while sqlite3_step(stmt) == SQLITE_ROW {
            let id = String(cString: sqlite3_column_text(stmt, 0))
            let src = String(cString: sqlite3_column_text(stmt, 1))
            let dst = String(cString: sqlite3_column_text(stmt, 2))
            let mode = String(cString: sqlite3_column_text(stmt, 3))
            let state = String(cString: sqlite3_column_text(stmt, 4))
            let totalFiles = UInt64(sqlite3_column_int64(stmt, 5))
            let totalBytes = UInt64(sqlite3_column_int64(stmt, 6))
            let copiedBytes = UInt64(sqlite3_column_int64(stmt, 7))
            let createdAt = String(cString: sqlite3_column_text(stmt, 8))
            var completedAt: String? = nil
            if sqlite3_column_type(stmt, 9) != SQLITE_NULL {
                completedAt = String(cString: sqlite3_column_text(stmt, 9))
            }
            let errCount = Int(sqlite3_column_int(stmt, 10))

            results.append(HistoricalJobItem(
                id: id,
                sourceRoot: src,
                targetRoot: dst,
                mode: mode,
                state: state,
                totalFiles: totalFiles,
                totalBytes: totalBytes,
                copiedBytes: copiedBytes,
                createdAt: createdAt,
                completedAt: completedAt,
                errorCount: errCount
            ))
        }

        self.historicalJobs = results
        if let current = selectedJob {
            self.selectedJob = results.first(where: { $0.id == current.id })
        }
    }

    // MARK: - Export Report Dialog Integration
    public enum ExportFormat {
        case json
        case csv
    }

    /// Invokes `volterra_export_report` and displays native macOS NSSavePanel.
    public func exportReport(format: ExportFormat) {
        guard let job = activeJobHandle else {
            errorMessage = "No active job handle available for direct report export."
            return
        }

        let formatCode: Int32 = (format == .json) ? 0 : 1
        guard let reportPtr = volterra_export_report(job, formatCode) else {
            errorMessage = "Failed to generate audit report from engine."
            return
        }
        let reportContent = String(cString: reportPtr)
        volterra_string_free(reportPtr)

        saveToFile(content: reportContent, defaultName: format == .json ? "volterra_audit_report.json" : "volterra_audit_report.csv", isJSON: format == .json)
    }

    public func exportJSONReport(for job: HistoricalJobItem) {
        // If active handle matches, export from active engine; otherwise generate historical summary JSON
        if activeJobHandle != nil {
            exportReport(format: .json)
        } else {
            let json = """
            {
              "job_id": "\(job.id)",
              "source_root": "\(job.sourceRoot)",
              "target_root": "\(job.targetRoot)",
              "mode": "\(job.mode)",
              "state": "\(job.state)",
              "total_files": \(job.totalFiles),
              "total_bytes": \(job.totalBytes),
              "copied_bytes": \(job.copiedBytes),
              "created_at": "\(job.createdAt)",
              "completed_at": "\(job.completedAt ?? "")",
              "error_count": \(job.errorCount)
            }
            """
            saveToFile(content: json, defaultName: "job_\(job.id)_report.json", isJSON: true)
        }
    }

    public func exportCSVReport(for job: HistoricalJobItem) {
        if activeJobHandle != nil {
            exportReport(format: .csv)
        } else {
            let csv = """
            job_id,source_root,target_root,mode,state,total_files,total_bytes,copied_bytes,created_at,completed_at,error_count
            "\(job.id)","\(job.sourceRoot)","\(job.targetRoot)","\(job.mode)","\(job.state)",\(job.totalFiles),\(job.totalBytes),\(job.copiedBytes),"\(job.createdAt)","\(job.completedAt ?? "")",\(job.errorCount)
            """
            saveToFile(content: csv, defaultName: "job_\(job.id)_report.csv", isJSON: false)
        }
    }

    private func saveToFile(content: String, defaultName: String, isJSON: Bool) {
        let savePanel = NSSavePanel()
        savePanel.canCreateDirectories = true
        savePanel.showsTagField = false
        savePanel.nameFieldStringValue = defaultName
        if isJSON {
            savePanel.allowedContentTypes = [.json]
        } else {
            savePanel.allowedContentTypes = [.commaSeparatedText]
        }

        savePanel.begin { [weak self] response in
            guard response == .OK, let targetURL = savePanel.url else { return }
            do {
                try content.write(to: targetURL, atomically: true, encoding: .utf8)
                Task { @MainActor in
                    self?.exportMessage = "Report saved to \(targetURL.lastPathComponent)"
                }
            } catch {
                Task { @MainActor in
                    self?.errorMessage = "Failed to write report: \(error.localizedDescription)"
                }
            }
        }
    }
}
