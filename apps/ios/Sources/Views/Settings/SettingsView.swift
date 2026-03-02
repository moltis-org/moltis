import SwiftUI
import UIKit

struct SettingsView: View {
    @EnvironmentObject var connectionStore: ConnectionStore
    @EnvironmentObject var settingsStore: SettingsStore
    @EnvironmentObject var locationSharingStore: LocationSharingStore
    @EnvironmentObject var authManager: AuthManager

    var body: some View {
        NavigationStack {
            Form {
                // Connection info
                Section("Server") {
                    if let host = connectionStore.serverHost {
                        LabeledContent("Host", value: host)
                    }
                    if let version = connectionStore.serverVersion {
                        LabeledContent("Version", value: version)
                    }
                    if !connectionStore.state.isDisconnected {
                        Button("Disconnect", role: .destructive) {
                            authManager.disconnect()
                            Task { await connectionStore.disconnect() }
                        }
                    }
                }

                // Model selection
                Section("Model") {
                    NavigationLink {
                        ModelPickerView()
                            .environmentObject(connectionStore.modelStore)
                    } label: {
                        LabeledContent(
                            "Current Model",
                            value: connectionStore.modelStore.selectedModelId ?? "Default"
                        )
                    }
                }

                // Preferences
                Section("Display") {
                    Toggle("Show Tool Calls", isOn: $settingsStore.showToolCalls)
                    Toggle("Live Activities", isOn: $settingsStore.enableLiveActivities)
                }

                Section("Connection") {
                    Toggle("Auto-Reconnect", isOn: $settingsStore.autoReconnect)
                }

                Section("Location") {
                    Toggle("Share Location with Moltis", isOn: $locationSharingStore.isEnabled)

                    LabeledContent(
                        "Permission",
                        value: locationSharingStore.authorizationDescription
                    )

                    if let lastSentAt = locationSharingStore.lastSentAt {
                        LabeledContent(
                            "Last Sent",
                            value: lastSentAt.formatted(
                                .relative(
                                    presentation: .named,
                                    unitsStyle: .wide
                                )
                            )
                        )
                    } else {
                        LabeledContent("Last Sent", value: "Never")
                    }

                    if let lastError = locationSharingStore.lastError {
                        Text(lastError)
                            .font(.footnote)
                            .foregroundStyle(.orange)
                    }

                    if locationSharingStore.authorizationStatus == .denied
                        || locationSharingStore.authorizationStatus == .restricted {
                        Button("Open iOS Settings") {
                            guard
                                let url = URL(string: UIApplication.openSettingsURLString)
                            else { return }
                            UIApplication.shared.open(url)
                        }
                    }
                }

                // About
                Section {
                    NavigationLink("About") {
                        AboutView()
                    }
                }
            }
            .navigationTitle("Settings")
        }
    }
}
