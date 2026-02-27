import Foundation

@MainActor
final class SettingsStore: ObservableObject {
    @Published var showToolCalls: Bool {
        didSet { UserDefaults.standard.set(showToolCalls, forKey: "showToolCalls") }
    }
    @Published var enableLiveActivities: Bool {
        didSet { UserDefaults.standard.set(enableLiveActivities, forKey: "enableLiveActivities") }
    }
    @Published var autoReconnect: Bool {
        didSet { UserDefaults.standard.set(autoReconnect, forKey: "autoReconnect") }
    }

    init() {
        let defaults = UserDefaults.standard
        // Default to true for these settings
        if defaults.object(forKey: "showToolCalls") == nil {
            defaults.set(true, forKey: "showToolCalls")
        }
        if defaults.object(forKey: "enableLiveActivities") == nil {
            defaults.set(true, forKey: "enableLiveActivities")
        }
        if defaults.object(forKey: "autoReconnect") == nil {
            defaults.set(true, forKey: "autoReconnect")
        }

        self.showToolCalls = defaults.bool(forKey: "showToolCalls")
        self.enableLiveActivities = defaults.bool(forKey: "enableLiveActivities")
        self.autoReconnect = defaults.bool(forKey: "autoReconnect")
    }
}
