//
// OverwriteConfirmationSheet.swift
// VolterraMac
//

import SwiftUI

public struct OverwriteConfirmationSheet: View {
    public let destinationPath: String
    public let onApprove: () -> Void
    public let onCancel: () -> Void

    public init(destinationPath: String, onApprove: @escaping () -> Void, onCancel: @escaping () -> Void) {
        self.destinationPath = destinationPath
        self.onApprove = onApprove
        self.onCancel = onCancel
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack(spacing: 12) {
                Image(systemName: "exclamationmark.shield.fill")
                    .font(.system(size: 36))
                    .foregroundColor(.orange)
                VStack(alignment: .leading, spacing: 4) {
                    Text("Overwrite Confirmation Required")
                        .font(.headline)
                    Text("Existing destination files were detected in:")
                        .font(.caption)
                        .foregroundColor(.secondary)
                    Text(destinationPath)
                        .font(.caption.bold())
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
            }

            Text("In compliance with VOLTERRA Data Safety Constraint R2, files will never be overwritten silently. Choosing 'Allow Overwrite' will atomically replace existing files only upon verified completion.")
                .font(.subheadline)
                .foregroundColor(.secondary)

            HStack {
                Spacer()
                Button("Cancel", role: .cancel) {
                    onCancel()
                }
                .keyboardShortcut(.cancelAction)

                Button("Explicitly Allow Overwrite", role: .destructive) {
                    onApprove()
                }
                .buttonStyle(.borderedProminent)
            }
        }
        .padding(24)
        .frame(width: 520)
    }
}
