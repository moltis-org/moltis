import ActivityKit
import Foundation

struct MoltisActivityAttributes: ActivityAttributes {
    var agentName: String
    var userMessage: String

    struct ContentState: Codable, Hashable {
        var currentStep: String
        var currentStepIcon: String?
        var previousStep: String?
        var secondPreviousStep: String?
        var stepStartDate: Date
        var stepEndDate: Date?
        var stepNumber: Int
        var model: String?
        var provider: String?
        var tokensGenerated: Int
        var isFinished: Bool { stepEndDate != nil }
    }
}
