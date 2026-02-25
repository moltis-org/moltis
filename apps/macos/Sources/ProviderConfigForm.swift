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
                    Text(model.displayName)
                        .tag(Optional(model.id))
                }
            }

            // Alternative searchable pickers kept for reference:
            //
            // let options = providerModels.map { (id: $0.id, display: $0.displayName) }
            //
            // SearchablePopoverPicker(
            //     label: "Model",
            //     selection: $providerStore.selectedModelID,
            //     options: options
            // )
            //
            // SearchableComboBoxPicker(
            //     label: "Model",
            //     selection: $providerStore.selectedModelID,
            //     options: options
            // )
            // .frame(height: 24)
        } else if providerStore.isLoadingModels {
            HStack(spacing: 6) {
                Text("Model")
                    .foregroundStyle(.secondary)
                ProgressView()
                    .controlSize(.mini)
            }
        }
    }
}
