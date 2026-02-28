import Foundation

enum OnboardingStep: Int, CaseIterable {
    case llm
    case voice
    case channels
    case identity
    case summary

    var symbolName: String {
        switch self {
        case .llm:
            return "cpu.fill"
        case .voice:
            return "waveform.circle.fill"
        case .channels:
            return "point.3.connected.trianglepath.dotted"
        case .identity:
            return "person.crop.circle.fill"
        case .summary:
            return "checkmark.seal.fill"
        }
    }

    var label: String {
        switch self {
        case .llm:
            return "LLM"
        case .voice:
            return "Voice"
        case .channels:
            return "Channels"
        case .identity:
            return "Identity"
        case .summary:
            return "Summary"
        }
    }

    var title: String {
        switch self {
        case .llm:
            return "Language Model"
        case .voice:
            return "Voice"
        case .channels:
            return "Channels"
        case .identity:
            return "Assistant Identity"
        case .summary:
            return "Ready to Go"
        }
    }

    var subtitle: String {
        switch self {
        case .llm:
            return "Choose your preferred model and provider."
        case .voice:
            return "Optionally enable voice interaction."
        case .channels:
            return "Configure channel routing and sender policies."
        case .identity:
            return "Give your assistant a name and personality."
        case .summary:
            return "Everything looks good. You're all set."
        }
    }

    /// Maps to the SettingsSection that provides content for this step.
    /// Summary has no corresponding settings section.
    var settingsSection: SettingsSection? {
        switch self {
        case .llm:
            return .llms
        case .voice:
            return .voice
        case .channels:
            return .channels
        case .identity:
            return .identity
        case .summary:
            return nil
        }
    }

    var stepNumber: Int {
        rawValue + 1
    }

    static var totalSteps: Int {
        allCases.count
    }
}
