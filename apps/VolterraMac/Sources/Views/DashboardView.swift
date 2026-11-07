//
// DashboardView.swift
// VolterraMac
//

import SwiftUI
import AppKit

public struct DashboardView: View {
    @EnvironmentObject public var transferVM: TransferViewModel
    @ObservedObject public var l10n = LocalizationManager.shared
    @State private var showingOverwriteSheet = false

    public init() {}

    public var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                // Header
                VStack(alignment: .leading, spacing: 4) {
                    Text(l10n.t(.setupTitle))
                        .font(.largeTitle.bold())
                    Text(l10n.t(.setupSubtitle))
                        .foregroundColor(.secondary)
                }

                // Source & Destination Cards Grid
                HStack(alignment: .top, spacing: 16) {
                    DeviceSelectionCard(
                        title: l10n.t(.sourceLocation),
                        icon: "folder.badge.gearshape",
                        selectedURL: transferVM.sourceURL,
                        diskProfile: transferVM.sourceDisk,
                        isSource: true,
                        onSelect: { transferVM.chooseSourceFolder() }
                    )

                    Image(systemName: "arrow.right")
                        .font(.title2)
                        .foregroundColor(.secondary)
                        .padding(.top, 60)

                    DeviceSelectionCard(
                        title: l10n.t(.destinationLocation),
                        icon: "externaldrive.badge.plus",
                        selectedURL: transferVM.destinationURL,
                        diskProfile: transferVM.destinationDisk,
                        isSource: false,
                        onSelect: { transferVM.chooseDestinationFolder() }
                    )
                }

                // Mode Selector Card
                ModeSelectorSection(selectedMode: $transferVM.selectedMode)

                // Pre-flight Banners
                if let warning = transferVM.capacityWarning {
                    CapacityWarningBanner(warning: warning)
                }

                if let metaWarning = transferVM.metadataWarning {
                    MetadataWarningBanner(warning: metaWarning)
                }

                // Advanced Options Disclosure
                DisclosureGroup(l10n.t(.advancedOptions)) {
                    VStack(alignment: .leading, spacing: 12) {
                        Toggle(l10n.t(.directIOToggle), isOn: $transferVM.directIO)
                            .help("Reduces OS cache thrashing and memory pressure during large file copies.")

                        Toggle(l10n.t(.readbackVerifyToggle), isOn: $transferVM.verifyReadback)
                            .help("Reads written chunks back from destination disk to verify BLAKE3 checksums.")

                        HStack {
                            Text(l10n.t(.memoryBufferBudget))
                            Picker("", selection: $transferVM.memoryBudgetMB) {
                                Text("128 MiB").tag(128)
                                Text("256 MiB (Default)").tag(256)
                                Text("512 MiB").tag(512)
                                Text("1024 MiB (Turbo)").tag(1024)
                            }
                            .frame(width: 180)
                        }
                    }
                    .padding(.top, 8)
                }
                .padding()
                .background(RoundedRectangle(cornerRadius: 10).fill(Color(nsColor: .controlBackgroundColor)))

                // Start Transfer Action Area
                HStack {
                    if let err = transferVM.errorMessage {
                        Text(err)
                            .foregroundColor(.red)
                            .font(.caption)
                    }

                    Spacer()

                    Button(action: {
                        if transferVM.destinationHasExistingFiles && !transferVM.isOverwriteApproved {
                            showingOverwriteSheet = true
                        } else {
                            transferVM.startTransfer()
                        }
                    }) {
                        HStack {
                            Image(systemName: "bolt.fill")
                            Text(l10n.t(.startTransfer))
                        }
                        .font(.headline)
                        .padding(.horizontal, 24)
                        .padding(.vertical, 8)
                    }
                    .buttonStyle(.borderedProminent)
                    .controlSize(.large)
                    .disabled(!transferVM.canStartTransfer)
                }
            }
            .padding(24)
        }
        .sheet(isPresented: $showingOverwriteSheet) {
            OverwriteConfirmationSheet(
                destinationPath: transferVM.destinationURL?.path ?? "",
                onApprove: {
                    transferVM.approveOverwriteAndStart()
                    showingOverwriteSheet = false
                },
                onCancel: {
                    showingOverwriteSheet = false
                }
            )
        }
    }
}
