import SwiftUI

@main
struct MoltisApp: App {
    @StateObject private var settings: AppSettings
    @StateObject private var chatStore: ChatStore
    @StateObject private var onboardingState: OnboardingState
    @StateObject private var providerStore: ProviderStore

    init() {
        let settings = AppSettings()
        let onboardingState = OnboardingState()
        let providerStore = ProviderStore()
        _settings = StateObject(wrappedValue: settings)
        _chatStore = StateObject(wrappedValue: ChatStore(settings: settings, providerStore: providerStore))
        _onboardingState = StateObject(wrappedValue: onboardingState)
        _providerStore = StateObject(wrappedValue: providerStore)
    }

    var body: some Scene {
        WindowGroup("Moltis") {
            Group {
                if onboardingState.isCompleted {
                    ContentView(chatStore: chatStore, settings: settings, providerStore: providerStore)
                } else {
                    OnboardingView(settings: settings, providerStore: providerStore) {
                        onboardingState.complete()
                        chatStore.loadVersion()
                    }
                }
            }
        }
        .windowResizability(.contentSize)

        Settings {
            SettingsView(settings: settings, providerStore: providerStore)
        }
        .windowResizability(.contentMinSize)
    }
}
