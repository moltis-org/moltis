import ActivityKit
import SwiftUI
import WidgetKit

struct MoltisLiveActivity: Widget {
    var body: some WidgetConfiguration {
        ActivityConfiguration(for: MoltisActivityAttributes.self) { context in
            // MARK: - Lock Screen Banner
            lockScreenView(context: context)
                .activityBackgroundTint(.black.opacity(0.85))

        } dynamicIsland: { context in
            DynamicIsland {
                // MARK: - Expanded regions
                DynamicIslandExpandedRegion(.leading) {
                    Circle()
                        .fill(context.state.isFinished ? .green : .blue)
                        .frame(width: 12, height: 12)
                        .padding(.leading, 4)
                }

                DynamicIslandExpandedRegion(.center) {
                    VStack(spacing: 2) {
                        Text(context.attributes.agentName)
                            .font(.headline)
                            .lineLimit(1)
                        Text(context.state.currentStep)
                            .font(.subheadline)
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                }

                DynamicIslandExpandedRegion(.trailing) {
                    Text("Step \(context.state.stepNumber)")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }

                DynamicIslandExpandedRegion(.bottom) {
                    if let previousStep = context.state.previousStep {
                        HStack(spacing: 6) {
                            Image(systemName: "checkmark.circle.fill")
                                .font(.caption2)
                                .foregroundStyle(.green)
                            Text(previousStep)
                                .font(.caption2)
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                            Spacer()
                        }
                        .padding(.horizontal, 4)
                    }
                }

            } compactLeading: {
                Circle()
                    .fill(context.state.isFinished ? .green : .blue)
                    .frame(width: 10, height: 10)

            } compactTrailing: {
                if context.state.isFinished {
                    Text("Done")
                        .font(.caption2)
                        .foregroundStyle(.green)
                } else {
                    Text(context.state.currentStep)
                        .font(.caption2)
                        .lineLimit(1)
                        .frame(maxWidth: 60)
                }

            } minimal: {
                Circle()
                    .fill(context.state.isFinished ? .green : .blue)
                    .frame(width: 8, height: 8)
            }
        }
    }

    // MARK: - Lock Screen layout

    @ViewBuilder
    private func lockScreenView(
        context: ActivityViewContext<MoltisActivityAttributes>
    ) -> some View {
        VStack(spacing: 12) {
            // Header: agent name + model + tokens
            headerRow(context: context)

            // Center: step cards or waiting state
            centerContent(context: context)
                .frame(height: 70)

            // Footer: current step with timer
            footerRow(context: context)
        }
        .padding(16)
    }

    @ViewBuilder
    private func headerRow(
        context: ActivityViewContext<MoltisActivityAttributes>
    ) -> some View {
        HStack {
            // Agent icon
            Image(systemName: "sparkles")
                .font(.caption)
                .foregroundStyle(.blue)

            Text(context.attributes.agentName)
                .font(.subheadline.bold())
                .foregroundStyle(.white)

            Spacer()

            if let model = context.state.model {
                Text(model)
                    .font(.caption2)
                    .padding(.horizontal, 8)
                    .padding(.vertical, 3)
                    .background(.ultraThinMaterial)
                    .clipShape(Capsule())
            }

            if context.state.tokensGenerated > 0 {
                Text("\(context.state.tokensGenerated) tok")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
            }
        }
    }

    @ViewBuilder
    private func centerContent(
        context: ActivityViewContext<MoltisActivityAttributes>
    ) -> some View {
        if context.state.isFinished {
            // Finished state
            HStack(spacing: 8) {
                Image(systemName: "checkmark.circle.fill")
                    .font(.title2)
                    .foregroundStyle(.green)
                Text("Complete")
                    .font(.body.bold())
                    .foregroundStyle(.white)
            }
        } else if let previousStep = context.state.previousStep {
            // Stacked completed step cards
            ZStack {
                if let secondPrev = context.state.secondPreviousStep {
                    IntentCardView(stepText: secondPrev, isTopCard: false)
                }
                IntentCardView(stepText: previousStep, isTopCard: true)
            }
        } else {
            // Waiting state: show user message
            HStack(spacing: 8) {
                Text(context.attributes.userMessage)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(3)
                    .padding(10)
                    .background(.ultraThinMaterial)
                    .clipShape(RoundedRectangle(cornerRadius: 10))
            }
        }
    }

    @ViewBuilder
    private func footerRow(
        context: ActivityViewContext<MoltisActivityAttributes>
    ) -> some View {
        HStack(spacing: 8) {
            if let icon = context.state.currentStepIcon {
                Image(systemName: icon)
                    .font(.caption)
                    .foregroundStyle(.blue)
            }

            Text(context.state.currentStep)
                .font(.caption)
                .foregroundStyle(.white)
                .lineLimit(1)

            Spacer()

            if !context.state.isFinished {
                Text(context.state.stepStartDate, style: .timer)
                    .font(.caption.monospacedDigit())
                    .foregroundStyle(.secondary)
            }
        }
    }
}
