import SwiftUI

struct SettingsView: View {
    @EnvironmentObject var connectionStore: ConnectionStore
    @EnvironmentObject var settingsStore: SettingsStore
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
                    if connectionStore.state.isConnected {
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
