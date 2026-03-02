import AppKit
import SwiftUI

extension SettingsSectionContent {
    var environmentPane: some View {
        Group {
            Section {
                if settings.envVars.isEmpty {
                    Text("No environment variables set.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                } else {
                    ForEach(settings.envVars) { item in
                        if settings.updatingEnvVarId == item.id {
                            HStack(spacing: 8) {
                                Text(item.key)
                                    .font(.system(.caption, design: .monospaced))
                                environmentBadge(encrypted: item.encrypted)
                                SecureField("New value", text: $settings.updatingEnvValue)
                                    .textFieldStyle(.roundedBorder)
                                    .frame(minWidth: 220)
                                    .onSubmit {
                                        settings.confirmEnvironmentVariableUpdate(key: item.key)
                                    }
                                Button("Save") {
                                    settings.confirmEnvironmentVariableUpdate(key: item.key)
                                }
                                .disabled(settings.environmentBusy)
                                Button("Cancel") {
                                    settings.cancelEnvironmentVariableUpdate()
                                }
                                .disabled(settings.environmentBusy)
                            }
                        } else {
                            HStack(spacing: 8) {
                                VStack(alignment: .leading, spacing: 2) {
                                    HStack(spacing: 6) {
                                        Text(item.key)
                                            .font(.system(.caption, design: .monospaced))
                                        environmentBadge(encrypted: item.encrypted)
                                    }
                                    HStack(spacing: 8) {
                                        Text("********")
                                        Text(item.updatedAt)
                                    }
                                    .font(.caption2)
                                    .foregroundStyle(.secondary)
                                }
                                Spacer()
                                Button("Update") {
                                    settings.startEnvironmentVariableUpdate(id: item.id)
                                }
                                .disabled(settings.environmentBusy)
                                Button("Delete", role: .destructive) {
                                    settings.deleteEnvironmentVariable(id: item.id)
                                }
                                .disabled(settings.environmentBusy)
                            }
                        }
                    }
                }

                VStack(alignment: .leading, spacing: 8) {
                    LabeledContent("Key") {
                        TextField("", text: $settings.newEnvKey)
                            .font(.system(.body, design: .monospaced))
                            .textFieldStyle(.roundedBorder)
                            .frame(minWidth: 300)
                            .onSubmit {
                                settings.addEnvironmentVariable()
                            }
                            .accessibilityIdentifier("settings-env-add-key")
                    }
                    LabeledContent("Value") {
                        SecureField("", text: $settings.newEnvValue)
                            .textFieldStyle(.roundedBorder)
                            .frame(minWidth: 300)
                            .onSubmit {
                                settings.addEnvironmentVariable()
                            }
                            .accessibilityIdentifier("settings-env-add-value")
                    }
                    HStack {
                        Spacer()
                        Button(settings.environmentBusy ? "Saving\u{2026}" : "Add") {
                            settings.addEnvironmentVariable()
                        }
                        .disabled(settings.environmentBusy)
                        .accessibilityIdentifier("settings-env-add-button")
                    }
                    if let message = settings.envMessage {
                        Text(message)
                            .font(.caption)
                            .foregroundStyle(.green)
                            .accessibilityIdentifier("settings-env-message")
                    }
                    if let error = settings.envError {
                        Text(error)
                            .font(.caption)
                            .foregroundStyle(.red)
                            .accessibilityIdentifier("settings-env-error")
                    }
                }
            } header: {
                VStack(alignment: .leading, spacing: 4) {
                    Text("Environment Variables")
                        .textCase(nil)
                    Text(environmentOverviewText)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .textCase(nil)
                }
            }

            Section("Paths") {
                LabeledContent("Config directory") {
                    HStack(spacing: 8) {
                        Text(settings.environmentConfigDir)
                            .textSelection(.enabled)
                            .foregroundStyle(.secondary)
                        Button("Reveal in Finder") {
                            revealPathInFinder(settings.environmentConfigDir)
                        }
                    }
                }
                LabeledContent("Data directory") {
                    HStack(spacing: 8) {
                        Text(settings.environmentDataDir)
                            .textSelection(.enabled)
                            .foregroundStyle(.secondary)
                        Button("Reveal in Finder") {
                            revealPathInFinder(settings.environmentDataDir)
                        }
                    }
                }
            }
        }
    }
}

extension SettingsSectionContent {
    var environmentOverviewText: String {
        switch settings.environmentVaultStatus {
        case "unsealed":
            return NSLocalizedString(
                "env.overview.unsealed",
                tableName: "Localizable",
                bundle: .main,
                value:
                    "Environment variables are injected into sandbox command execution. "
                    + "Values are write-only and never displayed. "
                    + "Vault unlocked, your keys are stored encrypted.",
                comment: "Environment section overview with unlocked vault"
            )
        case "sealed":
            return NSLocalizedString(
                "env.overview.sealed",
                tableName: "Localizable",
                bundle: .main,
                value:
                    "Environment variables are injected into sandbox command execution. "
                    + "Values are write-only and never displayed. "
                    + "Vault locked, encrypted keys cannot be read until you unlock "
                    + "encryption in Security settings.",
                comment: "Environment section overview with locked vault"
            )
        case "uninitialized":
            return NSLocalizedString(
                "env.overview.uninitialized",
                tableName: "Localizable",
                bundle: .main,
                value:
                    "Environment variables are injected into sandbox command execution. "
                    + "Values are write-only and never displayed. "
                    + "Vault not set up, configure Security to encrypt stored keys.",
                comment: "Environment section overview with uninitialized vault"
            )
        default:
            return NSLocalizedString(
                "env.overview.default",
                tableName: "Localizable",
                bundle: .main,
                value:
                    "Environment variables are injected into sandbox command execution. "
                    + "Values are write-only and never displayed.",
                comment: "Environment section overview default"
            )
        }
    }

    func environmentBadge(encrypted: Bool) -> some View {
        Text(encrypted ? "Encrypted" : "Plaintext")
            .font(.caption2)
            .foregroundStyle(encrypted ? .green : .secondary)
            .padding(.horizontal, 6)
            .padding(.vertical, 2)
            .background(.quaternary)
            .clipShape(Capsule())
    }

    func revealPathInFinder(_ path: String) {
        let trimmedPath = path.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedPath.isEmpty else { return }

        let url = URL(fileURLWithPath: trimmedPath)
        if FileManager.default.fileExists(atPath: trimmedPath) {
            NSWorkspace.shared.activateFileViewerSelecting([url])
        } else {
            NSWorkspace.shared.open(url.deletingLastPathComponent())
        }
    }
}
