import ActivityKit
import Foundation
import os

@MainActor
final class LiveActivityManager {
    static let shared = LiveActivityManager()

    private let logger = Logger(subsystem: "org.moltis.ios", category: "live-activity")
    private var currentActivity: Activity<MoltisActivityAttributes>?
    private var lastUpdateDate = Date.distantPast
    private var pendingUpdate: MoltisActivityAttributes.ContentState?
    private var debounceTask: Task<Void, Never>?

    /// Minimum interval between Live Activity updates (Apple recommends >= 1s).
    private let debounceInterval: TimeInterval = 1.0

    private var stepHistory: [String] = []
    private var currentStepNumber = 0

    private init() {}

    // MARK: - Start

    func startActivity(agentName: String, userMessage: String) {
        guard ActivityAuthorizationInfo().areActivitiesEnabled else {
            logger.debug("Live Activities not enabled")
            return
        }

        // End any existing activity
        endExistingActivity()

        let attributes = MoltisActivityAttributes(
            agentName: agentName,
            userMessage: String(userMessage.prefix(100))
        )

        let initialState = MoltisActivityAttributes.ContentState(
            currentStep: "Starting...",
            currentStepIcon: "sparkles",
            stepStartDate: Date(),
            stepNumber: 0,
            tokensGenerated: 0
        )

        do {
            let activity = try Activity.request(
                attributes: attributes,
                content: .init(state: initialState, staleDate: nil),
                pushType: nil
            )
            currentActivity = activity
            stepHistory = []
            currentStepNumber = 0
            logger.info("Started Live Activity: \(activity.id)")
        } catch {
            logger.error("Failed to start Live Activity: \(error.localizedDescription)")
        }
    }

    // MARK: - Update step

    func updateStep(label: String, icon: String, stepNumber: Int) {
        guard currentActivity != nil else { return }

        currentStepNumber = stepNumber

        // Build step history for stacked cards
        let previousStep = stepHistory.last
        let secondPreviousStep = stepHistory.count >= 2
            ? stepHistory[stepHistory.count - 2] : nil

        if label != stepHistory.last {
            stepHistory.append(label)
        }

        let state = MoltisActivityAttributes.ContentState(
            currentStep: label,
            currentStepIcon: icon,
            previousStep: previousStep,
            secondPreviousStep: secondPreviousStep,
            stepStartDate: Date(),
            stepNumber: stepNumber,
            tokensGenerated: 0
        )

        debouncedUpdate(state)
    }

    // MARK: - End

    func endActivity(success: Bool) {
        guard let activity = currentActivity else { return }

        debounceTask?.cancel()
        debounceTask = nil

        let finalStep = success ? "Complete" : "Error"
        let finalIcon = success ? "checkmark.circle" : "exclamationmark.triangle"

        let finalState = MoltisActivityAttributes.ContentState(
            currentStep: finalStep,
            currentStepIcon: finalIcon,
            previousStep: stepHistory.last,
            secondPreviousStep: stepHistory.count >= 2
                ? stepHistory[stepHistory.count - 2] : nil,
            stepStartDate: Date(),
            stepEndDate: Date(),
            stepNumber: currentStepNumber + 1,
            tokensGenerated: 0
        )

        Task {
            await activity.end(
                .init(state: finalState, staleDate: nil),
                dismissalPolicy: .after(.now + 8)
            )
            logger.info("Ended Live Activity (success: \(success))")
        }

        currentActivity = nil
        stepHistory = []
        currentStepNumber = 0
    }

    // MARK: - Private

    private func debouncedUpdate(_ state: MoltisActivityAttributes.ContentState) {
        pendingUpdate = state

        let now = Date()
        let elapsed = now.timeIntervalSince(lastUpdateDate)

        if elapsed >= debounceInterval {
            performUpdate(state)
        } else {
            debounceTask?.cancel()
            debounceTask = Task {
                let remaining = debounceInterval - elapsed
                try? await Task.sleep(for: .seconds(remaining))
                guard !Task.isCancelled, let pending = pendingUpdate else { return }
                performUpdate(pending)
            }
        }
    }

    private func performUpdate(_ state: MoltisActivityAttributes.ContentState) {
        guard let activity = currentActivity else { return }
        lastUpdateDate = Date()
        pendingUpdate = nil

        Task {
            await activity.update(.init(state: state, staleDate: nil))
        }
    }

    private func endExistingActivity() {
        guard let activity = currentActivity else { return }
        Task {
            await activity.end(nil, dismissalPolicy: .immediate)
        }
        currentActivity = nil
    }
}
