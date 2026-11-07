//
// VolterraMacApp.swift
// VolterraMac
//

import SwiftUI
import AppKit

@main
struct VolterraMacApp: App {
    @StateObject private var transferVM = TransferViewModel()
    @StateObject private var performanceVM = PerformanceViewModel()
    @StateObject private var errorCenterVM = ErrorCenterViewModel()
    @StateObject private var historyVM = HistoryViewModel()
    @ObservedObject private var l10n = LocalizationManager.shared

    init() {
        // Interconnect view models for dual-lane telemetry and error synchronization
    }

    var body: some Scene {
        WindowGroup {
            RootSplitView()
                .environmentObject(transferVM)
                .environmentObject(performanceVM)
                .environmentObject(errorCenterVM)
                .environmentObject(historyVM)
                .environment(\.layoutDirection, l10n.currentLanguage.layoutDirection)
                .frame(minWidth: 960, minHeight: 640)
                .onAppear {
                    transferVM.performanceViewModel = performanceVM
                    transferVM.errorCenterViewModel = errorCenterVM
                }
        }
        .windowStyle(.titleBar)
        .windowToolbarStyle(.unified(showsTitle: true))
        .commands {
            SidebarCommands()
            CommandMenu("Transfer") {
                Button(transferVM.isPaused ? l10n.t(.resumeTransfer) : l10n.t(.pauseTransfer)) {
                    transferVM.togglePauseResume()
                }
                .keyboardShortcut(.space, modifiers: [])
                .disabled(!transferVM.canPauseOrResume)

                Button(l10n.t(.cancelTransfer)) {
                    transferVM.requestCancel()
                }
                .keyboardShortcut(".", modifiers: [.command])
                .disabled(!transferVM.isActive)

                Divider()

                Button(l10n.t(.skipAllQuarantined)) {
                    errorCenterVM.skipAllErrors()
                }
                .keyboardShortcut("s", modifiers: [.command, .shift])
                .disabled(errorCenterVM.quarantinedFiles.isEmpty)
            }
            CommandMenu(l10n.t(.languageMenuTitle)) {
                ForEach(AppLanguage.allCases) { lang in
                    Button(action: { l10n.currentLanguage = lang }) {
                        if l10n.currentLanguage == lang {
                            Text("✓ \(lang.displayLabel)")
                        } else {
                            Text("   \(lang.displayLabel)")
                        }
                    }
                }
            }
        }
    }
}
