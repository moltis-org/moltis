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
            return NSLocalizedString("LLM", comment: "Onboarding step label")
        case .voice:
            return NSLocalizedString("Voice", comment: "Onboarding step label")
        case .channels:
            return NSLocalizedString("Channels", comment: "Onboarding step label")
        case .identity:
            return NSLocalizedString("Identity", comment: "Onboarding step label")
        case .summary:
            return NSLocalizedString("Summary", comment: "Onboarding step label")
        }
    }

    var title: String {
        switch self {
        case .llm:
            return NSLocalizedString("Language Model", comment: "Onboarding step title")
        case .voice:
            return NSLocalizedString("Voice", comment: "Onboarding step title")
        case .channels:
            return NSLocalizedString("Channels", comment: "Onboarding step title")
        case .identity:
            return NSLocalizedString("Assistant Identity", comment: "Onboarding step title")
        case .summary:
            return NSLocalizedString("Ready to Go", comment: "Onboarding step title")
        }
    }

    var subtitle: String {
        switch self {
        case .llm:
            return NSLocalizedString(
                "Choose your preferred model and provider.",
                comment: "Onboarding step subtitle"
            )
        case .voice:
            return NSLocalizedString(
                "Optionally enable voice interaction.",
                comment: "Onboarding step subtitle"
            )
        case .channels:
            return NSLocalizedString(
                "Configure channel routing and sender policies.",
                comment: "Onboarding step subtitle"
            )
        case .identity:
            return NSLocalizedString(
                "Give your assistant a name and personality.",
                comment: "Onboarding step subtitle"
            )
        case .summary:
            return NSLocalizedString(
                "Everything looks good. You're all set.",
                comment: "Onboarding step subtitle"
            )
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
