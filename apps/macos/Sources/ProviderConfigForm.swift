import SwiftUI

struct ProviderConfigForm: View {
    @ObservedObject var providerStore: ProviderStore

    private var provider: BridgeKnownProvider? {
        providerStore.selectedKnownProvider
    }

    var body: some View {
        if let provider {
            formContent(for: provider)
        } else {
            Text("Select a provider to configure")
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, alignment: .center)
                .padding()
        }
    }

    @ViewBuilder
    private func formContent(for provider: BridgeKnownProvider) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(provider.displayName)
                .font(.title3.weight(.semibold))

            if !provider.keyOptional {
                SecureField("API Key", text: $providerStore.apiKeyDraft)
                    .textFieldStyle(.roundedBorder)
            }

            if provider.defaultBaseUrl != nil {
                TextField(
                    "Base URL",
                    text: $providerStore.baseUrlDraft,
                    prompt: Text(provider.defaultBaseUrl ?? "")
                )
                .textFieldStyle(.roundedBorder)
            }

            modelPicker(for: provider)

            HStack {
                Button("Save") {
                    do {
                        try providerStore.saveCurrentProvider()
                    } catch {
                        // Error is visible as unchanged state
                    }
                }
                .buttonStyle(.borderedProminent)
                .disabled(
                    !provider.keyOptional
                        && providerStore.apiKeyDraft
                            .trimmingCharacters(in: .whitespacesAndNewlines)
                            .isEmpty
                )

                if providerStore.isLoadingModels {
                    ProgressView()
                        .controlSize(.small)
                        .padding(.leading, 8)
                    Text("Loading models...")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
        }
        .padding()
    }

    @ViewBuilder
    private func modelPicker(for provider: BridgeKnownProvider) -> some View {
        let providerModels = providerStore.modelsForProvider(provider.name)

        if !providerModels.isEmpty {
            Picker("Model", selection: $providerStore.selectedModelID) {
                Text("Default").tag(nil as String?)
                ForEach(providerModels) { model in
                    modelLabel(model)
                        .tag(Optional(model.id))
                }
            }
        } else if providerStore.isLoadingModels {
            HStack(spacing: 6) {
                Text("Model")
                    .foregroundStyle(.secondary)
                ProgressView()
                    .controlSize(.mini)
            }
        }
    }

    private func modelLabel(_ model: BridgeModelInfo) -> some View {
        HStack {
            Text(model.displayName)
            if let dateText = Self.formatModelDate(model.createdAt) {
                Spacer()
                Text(dateText)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }

    private static let monthYearFormatter: DateFormatter = {
        let formatter = DateFormatter()
        formatter.dateFormat = "MMM yyyy"
        return formatter
    }()

    private static func formatModelDate(_ epoch: Int?) -> String? {
        guard let epoch, epoch > 0 else { return nil }
        let date = Date(timeIntervalSince1970: TimeInterval(epoch))
        return monthYearFormatter.string(from: date)
    }
}
