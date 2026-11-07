//
// HistoryView.swift
// VolterraMac
//

import SwiftUI
import AppKit

public enum HistoryStatusFilter: String, CaseIterable, Identifiable {
    case all = "All Jobs"
    case completed = "Completed"
    case partial = "Partial"
    case failed = "Failed"

    public var id: String { rawValue }
}

public struct HistoryView: View {
    @EnvironmentObject public var historyVM: HistoryViewModel
    @ObservedObject public var l10n = LocalizationManager.shared
    @State private var searchQuery = ""
    @State private var statusFilter: HistoryStatusFilter = .all
    @State private var selectedJobId: String?

    public init() {}

    public var filteredJobs: [HistoricalJobItem] {
        historyVM.jobs.filter { job in
            let matchesSearch = searchQuery.isEmpty ||
                job.jobId.localizedCaseInsensitiveContains(searchQuery) ||
                job.sourceRoot.localizedCaseInsensitiveContains(searchQuery) ||
                job.targetRoot.localizedCaseInsensitiveContains(searchQuery)
            let matchesStatus: Bool
            switch statusFilter {
            case .all: matchesStatus = true
            case .completed: matchesStatus = job.status.lowercased() == "completed"
            case .partial: matchesStatus = job.status.lowercased() == "partial"
            case .failed: matchesStatus = job.status.lowercased() == "failed"
            }
            return matchesSearch && matchesStatus
        }
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            // Header
            HStack {
                VStack(alignment: .leading, spacing: 2) {
                    Text(l10n.t(.historyTitle))
                        .font(.largeTitle.bold())
                    Text(l10n.t(.historySubtitle))
                        .foregroundColor(.secondary)
                }
                Spacer()

                if let msg = historyVM.exportMessage {
                    Text(msg)
                        .font(.caption)
                        .foregroundColor(.green)
                }

                Button(action: { historyVM.loadHistory() }) {
                    Label(l10n.t(.refreshButton), systemImage: "arrow.clockwise")
                }
                .buttonStyle(.bordered)
            }

            // Search and Filter Bar
            HStack(spacing: 12) {
                HStack {
                    Image(systemName: "magnifyingglass")
                        .foregroundColor(.secondary)
                    TextField(l10n.t(.searchJobOrPath), text: $searchQuery)
                        .textFieldStyle(.plain)
                }
                .padding(8)
                .background(RoundedRectangle(cornerRadius: 8).fill(Color(nsColor: .controlBackgroundColor)))

                Picker("Status", selection: $statusFilter) {
                    ForEach(HistoryStatusFilter.allCases) { filter in
                        Text(filter.rawValue).tag(filter)
                    }
                }
                .frame(width: 160)

                Toggle(l10n.t(.maskPersonalPaths), isOn: $historyVM.maskUserPaths)
                    .help("Replaces user home directory paths with ~ for privacy in shared reports")
            }

            // Historical Jobs Table
            if filteredJobs.isEmpty {
                VStack(spacing: 12) {
                    Spacer()
                    Image(systemName: "tray")
                        .font(.system(size: 48))
                        .foregroundColor(.secondary)
                    Text(l10n.t(.noHistoryFound))
                        .font(.headline)
                    Text("Completed transfer runs will appear here automatically.")
                        .font(.caption)
                        .foregroundColor(.secondary)
                    Spacer()
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                Table(filteredJobs, selection: $selectedJobId) {
                    TableColumn("Date") { job in
                        Text(job.formattedDate)
                            .font(.subheadline)
                    }
                    .width(min: 130, ideal: 140)

                    TableColumn("Job ID") { job in
                        Text(job.jobId)
                            .font(.caption.monospaced())
                    }
                    .width(min: 120, ideal: 140)

                    TableColumn("Source → Target") { job in
                        VStack(alignment: .leading, spacing: 2) {
                            Text(historyVM.sanitizePath(job.sourceRoot))
                                .font(.caption)
                                .lineLimit(1)
                                .truncationMode(.middle)
                            Text("→ " + historyVM.sanitizePath(job.targetRoot))
                                .font(.caption.bold())
                                .lineLimit(1)
                                .truncationMode(.middle)
                        }
                    }

                    TableColumn("Mode") { job in
                        Text(job.mode)
                    }
                    .width(min: 90, ideal: 100)

                    TableColumn("Status") { job in
                        BadgeView(text: job.status, color: job.statusColor)
                    }
                    .width(min: 90, ideal: 100)

                    TableColumn("Files") { job in
                        Text("\(job.completedFiles) / \(job.totalFiles)")
                    }
                    .width(min: 80, ideal: 90)

                    TableColumn("Bytes") { job in
                        Text(job.formattedBytes)
                    }
                    .width(min: 80, ideal: 90)

                    TableColumn("Errors") { job in
                        Text("\(job.errorCount)")
                            .foregroundColor(job.errorCount > 0 ? .red : .secondary)
                    }
                    .width(min: 50, ideal: 60)

                    TableColumn("Export") { job in
                        HStack(spacing: 6) {
                            Button("JSON") {
                                historyVM.exportJSONReport(for: job)
                            }
                            .buttonStyle(.bordered)
                            .controlSize(.small)

                            Button("CSV") {
                                historyVM.exportCSVReport(for: job)
                            }
                            .buttonStyle(.bordered)
                            .controlSize(.small)
                        }
                    }
                    .width(min: 120, ideal: 130)
                }
            }
        }
        .padding(24)
        .onAppear {
            historyVM.loadHistory()
        }
    }
}
