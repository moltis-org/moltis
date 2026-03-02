import Combine
import Foundation

final class OnboardingState: ObservableObject {
    @Published private(set) var isCompleted: Bool
    private static let skipOnboardingEnv = "MOLTIS_UI_TEST_SKIP_ONBOARDING"

    private let defaults: UserDefaults
    private let completionKey: String

    init(
        defaults: UserDefaults = .standard,
        completionKey: String = "swift_poc_onboarding_completed_v1"
    ) {
        self.defaults = defaults
        self.completionKey = completionKey
        if Self.shouldSkipOnboarding {
            isCompleted = true
        } else {
            isCompleted = defaults.bool(forKey: completionKey)
        }
    }

    func complete() {
        defaults.set(true, forKey: completionKey)
        isCompleted = true
    }

    func reset() {
        defaults.set(false, forKey: completionKey)
        isCompleted = false
    }
}

private extension OnboardingState {
    static var shouldSkipOnboarding: Bool {
        let rawValue = ProcessInfo.processInfo.environment[skipOnboardingEnv]
        guard
            let value = rawValue?
                .trimmingCharacters(in: .whitespacesAndNewlines)
                .lowercased()
        else {
            return false
        }

        return value == "1" || value == "true" || value == "yes"
    }
}
