import SwiftUI

@main
struct MoltisApp: App {
    @StateObject private var authManager = AuthManager()
    @StateObject private var connectionStore = ConnectionStore()
    @StateObject private var settingsStore = SettingsStore()

    var body: some Scene {
        WindowGroup {
            Group {
                if authManager.activeServer != nil {
                    ContentView()
                } else {
                    ConnectView()
                }
            }
            .environmentObject(authManager)
            .environmentObject(connectionStore)
            .environmentObject(settingsStore)
            .task {
                authManager.loadSavedServers()
                if let server = authManager.activeServer {
                    await connectionStore.connect(to: server, authManager: authManager)
                }
            }
        }
    }
}
