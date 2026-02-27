import SwiftUI

struct ModelPickerView: View {
    @EnvironmentObject var modelStore: ModelStore
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        List {
            if modelStore.isLoading {
                ProgressView("Loading models...")
            }

            ForEach(modelStore.modelsByProvider, id: \.provider) { group in
                Section(group.provider) {
                    ForEach(group.models) { model in
                        Button {
                            Task {
                                await modelStore.selectModel(id: model.id)
                                dismiss()
                            }
                        } label: {
                            HStack {
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(model.name)
                                        .font(.body)
                                        .foregroundStyle(.primary)
                                    if let tier = model.tier {
                                        Text(tier)
                                            .font(.caption)
                                            .foregroundStyle(.secondary)
                                    }
                                }

                                Spacer()

                                if model.id == modelStore.selectedModelId {
                                    Image(systemName: "checkmark")
                                        .foregroundStyle(.blue)
                                }
                            }
                        }
                    }
                }
            }
        }
        .navigationTitle("Models")
        .refreshable {
            await modelStore.loadModels()
        }
    }
}
