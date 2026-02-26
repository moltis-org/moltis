import SwiftUI

struct OnboardingView: View {
    @ObservedObject var settings: AppSettings
    @ObservedObject var providerStore: ProviderStore
    let onFinish: () -> Void

    @State private var currentStep = OnboardingStep.llm

    var body: some View {
        VStack(spacing: 0) {
            NavigationSplitView {
                sidebar
            } detail: {
                detailContent
            }

            Divider()

            footerBar
        }
        .frame(minWidth: 780, minHeight: 520)
        .onAppear {
            providerStore.loadAll()
        }
    }
}

// MARK: - Sidebar

private extension OnboardingView {
    var sidebar: some View {
        List(OnboardingStep.allCases, id: \.self, selection: $currentStep) { step in
            Label {
                Text(step.label)
            } icon: {
                Image(systemName: step.symbolName)
                    .foregroundStyle(iconColor(for: step))
            }
            .tag(step)
        }
        .navigationTitle("Setup")
        .navigationSplitViewColumnWidth(min: 180, ideal: 200)
    }

    func iconColor(for step: OnboardingStep) -> Color {
        if step.rawValue < currentStep.rawValue {
            return .green
        } else if step == currentStep {
            return .accentColor
        }
        return .secondary
    }
}

// MARK: - Detail

private extension OnboardingView {
    var detailContent: some View {
        Form {
            if currentStep == .summary {
                summarySection
            } else if currentStep == .llm {
                Section(currentStep.title) {
                    ProviderGridPane(providerStore: providerStore)
                }
            } else if currentStep == .voice {
                Section(currentStep.title) {
                    VoiceProviderGridPane(
                        providerStore: providerStore,
                        settings: settings
                    )
                }
            } else if let section = currentStep.settingsSection {
                Section(currentStep.title) {
                    SettingsSectionContent(
                        section: section,
                        settings: settings,
                        providerStore: providerStore
                    )
                }
            }
        }
        .formStyle(.grouped)
    }

    var summarySection: some View {
        Section("Ready to Go") {
            LabeledContent("Provider", value: settings.llmProvider.capitalized)
            LabeledContent("Model", value: settings.llmModel)
            LabeledContent("Voice", value: settings.voiceEnabled ? "Enabled" : "Disabled")
            LabeledContent(
                "Name",
                value: settings.identityName.isEmpty ? "Default" : settings.identityName
            )

            if !providerStore.detectedSources.isEmpty {
                VStack(alignment: .leading, spacing: 4) {
                    Text("Detected providers:")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    ForEach(providerStore.detectedSources, id: \.source) { source in
                        HStack(spacing: 4) {
                            Image(systemName: "checkmark.circle.fill")
                                .foregroundStyle(.green)
                                .font(.caption)
                            Text("\(source.provider)")
                                .font(.caption)
                        }
                    }
                }
            }
        }
    }
}

// MARK: - Footer

private extension OnboardingView {
    var footerBar: some View {
        HStack {
            Text("Step \(currentStep.stepNumber) of \(OnboardingStep.totalSteps)")
                .font(.subheadline)
                .foregroundStyle(.secondary)

            Spacer()

            if currentStep != OnboardingStep.allCases.first {
                Button("Back") {
                    goBack()
                }
                .keyboardShortcut(.cancelAction)
            }

            if currentStep == .summary {
                Button("Get Started") {
                    onFinish()
                }
                .keyboardShortcut(.defaultAction)
                .buttonStyle(.borderedProminent)
            } else {
                Button("Next") {
                    goForward()
                }
                .keyboardShortcut(.defaultAction)
                .buttonStyle(.borderedProminent)
            }
        }
        .padding(.horizontal, 20)
        .padding(.vertical, 14)
    }

    func goForward() {
        let cases = OnboardingStep.allCases
        guard let index = cases.firstIndex(of: currentStep),
              cases.index(after: index) < cases.endIndex else {
            return
        }
        withAnimation(.easeInOut(duration: 0.15)) {
            currentStep = cases[cases.index(after: index)]
        }
    }

    func goBack() {
        let cases = OnboardingStep.allCases
        guard let index = cases.firstIndex(of: currentStep), index > cases.startIndex else {
            return
        }
        withAnimation(.easeInOut(duration: 0.15)) {
            currentStep = cases[cases.index(before: index)]
        }
    }
}
