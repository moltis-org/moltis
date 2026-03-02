import SwiftUI

@main
struct MoltisApp: App {
    @Environment(\.scenePhase) private var scenePhase

    @StateObject private var authManager = AuthManager()
    @StateObject private var connectionStore = ConnectionStore()
    @StateObject private var settingsStore = SettingsStore()
    @StateObject private var locationSharingStore = LocationSharingStore()

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
            .environmentObject(locationSharingStore)
            .task {
                authManager.loadSavedServers()
                locationSharingStore.configure(connectionStore: connectionStore)
                locationSharingStore.setAppIsActive(scenePhase == .active)
                if let server = authManager.activeServer {
                    await connectionStore.connect(to: server, authManager: authManager)
                }
                locationSharingStore.handleConnectionStateChange()
            }
            .onChange(of: scenePhase) { _, newPhase in
                locationSharingStore.setAppIsActive(newPhase == .active)
            }
            .onChange(of: connectionStore.state) { _, _ in
                locationSharingStore.handleConnectionStateChange()
            }
        }
    }
}
