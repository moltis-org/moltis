import SwiftUI

struct AboutView: View {
    @EnvironmentObject var connectionStore: ConnectionStore

    private var appVersion: String {
        Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "1.0.0"
    }

    private var buildNumber: String {
        Bundle.main.infoDictionary?["CFBundleVersion"] as? String ?? "1"
    }

    var body: some View {
        List {
            Section("App") {
                LabeledContent("Version", value: appVersion)
                LabeledContent("Build", value: buildNumber)
            }

            if connectionStore.state.isConnected {
                Section("Server") {
                    if let host = connectionStore.serverHost {
                        LabeledContent("Host", value: host)
                    }
                    if let version = connectionStore.serverVersion {
                        LabeledContent("Version", value: version)
                    }
                    if let name = connectionStore.agentName {
                        LabeledContent("Agent", value: name)
                    }
                }
            }

            Section {
                Link(
                    "Documentation",
                    destination: URL(string: "https://docs.moltis.org")!
                )
                Link(
                    "Source Code",
                    destination: URL(string: "https://github.com/openclaw/openclaw")!
                )
            }
        }
        .navigationTitle("About")
    }
}
