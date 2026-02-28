import SwiftUI

private struct SettingsMenuCommand: Commands {
    @Environment(\.openWindow) private var openWindow

    var body: some Commands {
        CommandGroup(replacing: .appSettings) {
            Button("Settings...") {
                openWindow(id: "settings")
            }
            .keyboardShortcut(",", modifiers: .command)
        }
    }
}

@main
struct MoltisApp: App {
    @StateObject private var settings: AppSettings
    @StateObject private var chatStore: ChatStore
    @StateObject private var onboardingState: OnboardingState
    @StateObject private var providerStore: ProviderStore
    @StateObject private var logStore: LogStore
    @StateObject private var networkAuditStore: NetworkAuditStore

    init() {
        let settings = AppSettings()
        let onboardingState = OnboardingState()
        let logStore = LogStore()
        let networkAuditStore = NetworkAuditStore()

        // Install Rust→Swift log bridge before any FFI calls
        MoltisClient.installLogCallback(logStore: logStore)

        let providerStore = ProviderStore(logStore: logStore)
        let chatStore = ChatStore(
            settings: settings, providerStore: providerStore, logStore: logStore
        )

        // Install Rust→Swift session event bridge (shares bus with gateway)
        MoltisClient.installSessionEventCallback(chatStore: chatStore)

        // Install Rust→Swift network audit bridge
        MoltisClient.installNetworkAuditCallback(store: networkAuditStore)

        _settings = StateObject(wrappedValue: settings)
        _chatStore = StateObject(wrappedValue: chatStore)
        _onboardingState = StateObject(wrappedValue: onboardingState)
        _providerStore = StateObject(wrappedValue: providerStore)
        _logStore = StateObject(wrappedValue: logStore)
        _networkAuditStore = StateObject(wrappedValue: networkAuditStore)
    }

    var body: some Scene {
        WindowGroup("Moltis") {
            Group {
                if onboardingState.isCompleted {
                    ContentView(chatStore: chatStore, settings: settings, providerStore: providerStore)
                        .onAppear {
                            chatStore.loadVersion()
                            chatStore.loadIdentity()
                            chatStore.loadSessions()
                        }
                } else {
                    OnboardingView(settings: settings, providerStore: providerStore) {
                        onboardingState.complete()
                        chatStore.loadVersion()
                        chatStore.loadIdentity()
                        chatStore.loadSessions()
                    }
                }
            }
        }
        .windowResizability(.contentSize)

        WindowGroup("Settings", id: "settings") {
            SettingsView(
                settings: settings,
                providerStore: providerStore,
                logStore: logStore,
                networkAuditStore: networkAuditStore
            )
        }
        .defaultSize(width: 960, height: 780)
        .windowResizability(.contentMinSize)
        .commands {
            SettingsMenuCommand()
        }
    }
}
