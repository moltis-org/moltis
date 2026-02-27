import SwiftUI

struct ContentView: View {
    @EnvironmentObject var connectionStore: ConnectionStore
    @EnvironmentObject var authManager: AuthManager

    var body: some View {
        TabView {
            Tab("Chat", systemImage: "bubble.left.and.bubble.right") {
                ChatView()
                    .environmentObject(connectionStore.chatStore)
            }

            Tab("Sessions", systemImage: "list.bullet") {
                SessionListView()
                    .environmentObject(connectionStore.sessionStore)
                    .environmentObject(connectionStore.chatStore)
            }

            Tab("Settings", systemImage: "gear") {
                SettingsView()
            }
        }
        .overlay(alignment: .top) {
            if !connectionStore.state.isConnected {
                connectionBanner
            }
        }
    }

    private var connectionBanner: some View {
        HStack(spacing: 8) {
            if case .connecting = connectionStore.state {
                ProgressView()
                    .controlSize(.small)
            } else {
                Image(systemName: "wifi.slash")
                    .font(.caption)
            }
            Text(connectionStore.state.statusText)
                .font(.caption)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
        .frame(maxWidth: .infinity)
        .background(.ultraThinMaterial)
    }
}
