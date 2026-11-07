//
// ErrorCenterView.swift
// VolterraMac
//

import SwiftUI
import AppKit

public enum ErrorFilter: String, CaseIterable, Identifiable {
    case all = "All Errors"
    case readIO = "Read I/O (EIO)"
    case permission = "Permission (EACCES)"
    case noSpace = "No Space (ENOSPC)"

    public var id: String { rawValue }
}

public struct ErrorCenterView: View {
    @EnvironmentObject public var errorCenterVM: ErrorCenterViewModel
    @ObservedObject public var l10n = LocalizationManager.shared
    @State private var searchText = ""
    @State private var selectedFilter: ErrorFilter = .all
    @State private var inspectingFile: QuarantinedFileItem?

    public init() {}

    public var filteredFiles: [QuarantinedFileItem] {
        errorCenterVM.quarantinedFiles.filter { item in
            let matchesSearch = searchText.isEmpty || item.relativePath.localizedCaseInsensitiveContains(searchText)
            let matchesFilter: Bool
            switch selectedFilter {
            case .all: matchesFilter = true
            case .readIO: matchesFilter = item.errorCategory.contains("EIO")
            case .permission: matchesFilter = item.errorCategory.contains("EACCES")
            case .noSpace: matchesFilter = item.errorCategory.contains("ENOSPC")
            }
            return matchesSearch && matchesFilter
        }
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            // Header & Bulk Action Controls
            HStack {
                VStack(alignment: .leading, spacing: 2) {
                    Text(l10n.t(.errorCenterTitle))
                        .font(.largeTitle.bold())
                    Text(l10n.t(.errorCenterSubtitle))
                        .foregroundColor(.secondary)
                }

                Spacer()

                HStack(spacing: 12) {
                    Button(action: { errorCenterVM.skipAllErrors() }) {
                        Label(l10n.t(.skipAllQuarantined), systemImage: "forward.fill")
                    }
                    .buttonStyle(.bordered)
                    .disabled(errorCenterVM.quarantinedFiles.isEmpty || !errorCenterVM.hasActiveJob)

                    Button(action: { errorCenterVM.retryAllFailed() }) {
                        Label(l10n.t(.retryAll), systemImage: "arrow.clockwise")
                    }
                    .buttonStyle(.borderedProminent)
                    .disabled(errorCenterVM.quarantinedFiles.isEmpty || !errorCenterVM.hasActiveJob)
                }
            }

            if let feedback = errorCenterVM.actionFeedbackMessage {
                HStack {
                    Image(systemName: "info.circle")
                    Text(feedback)
                        .font(.subheadline)
                    Spacer()
                    Button(action: { errorCenterVM.actionFeedbackMessage = nil }) {
                        Image(systemName: "xmark")
                    }
                    .buttonStyle(.plain)
                }
                .padding(8)
                .background(RoundedRectangle(cornerRadius: 6).fill(Color.orange.opacity(0.15)))
            }

            // Search and Filter Bar
            HStack(spacing: 12) {
                HStack {
                    Image(systemName: "magnifyingglass")
                        .foregroundColor(.secondary)
                    TextField(l10n.t(.searchFilePath), text: $searchText)
                        .textFieldStyle(.plain)
                }
                .padding(8)
                .background(RoundedRectangle(cornerRadius: 8).fill(Color(nsColor: .controlBackgroundColor)))

                Picker("Filter", selection: $selectedFilter) {
                    ForEach(ErrorFilter.allCases) { filter in
                        Text(filter.rawValue).tag(filter)
                    }
                }
                .frame(width: 180)
            }

            // Quarantined Files Table
            if filteredFiles.isEmpty {
                VStack(spacing: 12) {
                    Spacer()
                    Image(systemName: "checkmark.shield.fill")
                        .font(.system(size: 48))
                        .foregroundColor(.green)
                    Text(l10n.t(.noQuarantinedFiles))
                        .font(.headline)
                    Text(l10n.t(.healthyPipelineSubtitle))
                        .font(.caption)
                        .foregroundColor(.secondary)
                    Spacer()
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                Table(filteredFiles) {
                    TableColumn("File Path") { item in
                        VStack(alignment: .leading, spacing: 2) {
                            Text(item.fileName)
                                .font(.body.bold())
                            Text(item.relativePath)
                                .font(.caption)
                                .foregroundColor(.secondary)
                                .lineLimit(1)
                                .truncationMode(.middle)
                        }
                    }

                    TableColumn("Size") { item in
                        Text(item.formattedSize)
                    }
                    .width(min: 80, ideal: 90)

                    TableColumn("Error Category") { item in
                        BadgeView(text: item.errorCategory, color: .red)
                    }
                    .width(min: 140, ideal: 160)

                    TableColumn("Attempts") { item in
                        Text("\(item.attempts)")
                    }
                    .width(min: 60, ideal: 70)

                    TableColumn("Actions") { item in
                        HStack(spacing: 8) {
                            Button(l10n.t(.inspectAction)) {
                                inspectingFile = item
                            }
                            .buttonStyle(.bordered)
                            .controlSize(.small)

                            Button(l10n.t(.skipAction)) {
                                errorCenterVM.skipFile(fileId: item.fileId)
                            }
                            .buttonStyle(.bordered)
                            .controlSize(.small)
                            .disabled(!errorCenterVM.hasActiveJob)

                            Button(l10n.t(.retryAction)) {
                                errorCenterVM.retryFile(fileId: item.fileId)
                            }
                            .buttonStyle(.borderedProminent)
                            .controlSize(.small)
                            .disabled(!errorCenterVM.hasActiveJob)
                        }
                    }
                    .width(min: 200, ideal: 220)
                }
            }
        }
        .padding(24)
        .sheet(item: $inspectingFile) { file in
            ExtentInspectorSheet(
                file: file,
                onSkip: {
                    errorCenterVM.skipFile(fileId: file.fileId)
                    inspectingFile = nil
                },
                onRetry: {
                    errorCenterVM.retryFile(fileId: file.fileId)
                    inspectingFile = nil
                },
                onDefer: {
                    errorCenterVM.deferFile(fileId: file.fileId)
                    inspectingFile = nil
                },
                onClose: {
                    inspectingFile = nil
                }
            )
        }
    }
}
