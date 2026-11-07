//
// RootSplitView.swift
// VolterraMac
//

import SwiftUI

public enum NavigationTab: String, CaseIterable, Identifiable {
    case dashboard = "Setup"
    case liveTransfer = "Live Transfer"
    case performance = "Performance"
    case errorCenter = "Error Center"
    case history = "History & Reports"

    public var id: String { rawValue }

    public var iconName: String {
        switch self {
        case .dashboard: return "slider.horizontal.3"
        case .liveTransfer: return "arrow.right.arrow.left.circle"
        case .performance: return "gauge.with.needle"
        case .errorCenter: return "exclamationmark.triangle"
        case .history: return "clock.arrow.circlepath"
        }
    }

    @MainActor
    public func localizedTitle(_ l10n: LocalizationManager) -> String {
        switch self {
        case .dashboard: return l10n.t(.tabSetup)
        case .liveTransfer: return l10n.t(.tabLiveTransfer)
        case .performance: return l10n.t(.tabPerformance)
        case .errorCenter: return l10n.t(.tabErrorCenter)
        case .history: return l10n.t(.tabHistory)
        }
    }
}

public struct RootSplitView: View {
    @EnvironmentObject public var transferVM: TransferViewModel
    @EnvironmentObject public var errorCenterVM: ErrorCenterViewModel
    @ObservedObject public var l10n = LocalizationManager.shared
    @State private var selectedTab: NavigationTab? = .dashboard

    public init() {}

    public var body: some View {
        NavigationSplitView {
            List(selection: $selectedTab) {
                Section(l10n.t(.sectionOperations)) {
                    NavigationLink(value: NavigationTab.dashboard) {
                        Label(NavigationTab.dashboard.localizedTitle(l10n), systemImage: NavigationTab.dashboard.iconName)
                    }

                    NavigationLink(value: NavigationTab.liveTransfer) {
                        HStack {
                            Label(NavigationTab.liveTransfer.localizedTitle(l10n), systemImage: NavigationTab.liveTransfer.iconName)
                            Spacer()
                            if transferVM.isActive {
                                Circle()
                                    .fill(transferVM.statusColor)
                                    .frame(width: 8, height: 8)
                            }
                        }
                    }
                }

                Section(l10n.t(.sectionDiagnostics)) {
                    NavigationLink(value: NavigationTab.performance) {
                        Label(NavigationTab.performance.localizedTitle(l10n), systemImage: NavigationTab.performance.iconName)
                    }

                    NavigationLink(value: NavigationTab.errorCenter) {
                        HStack {
                            Label(NavigationTab.errorCenter.localizedTitle(l10n), systemImage: NavigationTab.errorCenter.iconName)
                            Spacer()
                            if errorCenterVM.quarantinedCount > 0 {
                                Text("\(errorCenterVM.quarantinedCount)")
                                    .font(.caption2.bold())
                                    .padding(.horizontal, 6)
                                    .padding(.vertical, 2)
                                    .background(Capsule().fill(Color.red))
                                    .foregroundColor(.white)
                            }
                        }
                    }
                }

                Section(l10n.t(.sectionAudit)) {
                    NavigationLink(value: NavigationTab.history) {
                        Label(NavigationTab.history.localizedTitle(l10n), systemImage: NavigationTab.history.iconName)
                    }
                }
            }
            .navigationSplitViewColumnWidth(min: 200, ideal: 230, max: 290)
            .safeAreaInset(edge: .bottom) {
                VStack(alignment: .leading, spacing: 6) {
                    Divider()
                    HStack {
                        Menu {
                            ForEach(AppLanguage.allCases) { lang in
                                Button(action: { l10n.currentLanguage = lang }) {
                                    Text(l10n.currentLanguage == lang ? "✓ \(lang.displayLabel)" : "   \(lang.displayLabel)")
                                }
                            }
                        } label: {
                            HStack(spacing: 4) {
                                Image(systemName: "globe")
                                    .font(.caption)
                                Text(l10n.currentLanguage.displayLabel)
                                    .font(.caption.bold())
                            }
                        }
                        .menuStyle(.borderlessButton)
                        .fixedSize()

                        Spacer()

                        Button(action: { transferVM.refreshDisks() }) {
                            Image(systemName: "arrow.clockwise")
                                .font(.caption)
                        }
                        .buttonStyle(.plain)
                        .help(l10n.t(.refreshDisksHelp))
                    }
                    .padding(.horizontal, 16)
                    .padding(.vertical, 8)
                }
            }
        } detail: {
            Group {
                switch selectedTab ?? .dashboard {
                case .dashboard:
                    DashboardView()
                case .liveTransfer:
                    LiveTransferView()
                case .performance:
                    PerformanceDashboardView()
                case .errorCenter:
                    ErrorCenterView()
                case .history:
                    HistoryView()
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .toolbar {
                ToolbarItem(placement: .primaryAction) {
                    Menu {
                        ForEach(AppLanguage.allCases) { lang in
                            Button(action: { l10n.currentLanguage = lang }) {
                                Text(l10n.currentLanguage == lang ? "✓ \(lang.displayLabel)" : "   \(lang.displayLabel)")
                            }
                        }
                    } label: {
                        HStack(spacing: 4) {
                            Text(l10n.currentLanguage.flag)
                            Text(l10n.currentLanguage.nativeName)
                                .font(.subheadline)
                            Image(systemName: "chevron.down")
                                .font(.caption2)
                        }
                        .padding(.horizontal, 8)
                        .padding(.vertical, 4)
                    }
                    .help(l10n.t(.languageMenuTitle))
                }
            }
        }
        .environment(\.layoutDirection, l10n.currentLanguage.layoutDirection)
    }
}
