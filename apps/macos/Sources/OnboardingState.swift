import Combine
import Foundation

final class OnboardingState: ObservableObject {
    @Published private(set) var isCompleted: Bool

    private let defaults: UserDefaults
    private let completionKey: String

    init(
        defaults: UserDefaults = .standard,
        completionKey: String = "swift_poc_onboarding_completed_v1"
    ) {
        self.defaults = defaults
        self.completionKey = completionKey
        isCompleted = defaults.bool(forKey: completionKey)
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
