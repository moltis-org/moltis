import Foundation
import os

@MainActor
final class ModelStore: ObservableObject {
    @Published var models: [ModelInfo] = []
    @Published var selectedModelId: String? {
        didSet {
            if let id = selectedModelId {
                UserDefaults.standard.set(id, forKey: selectedModelKey)
            }
        }
    }
    @Published var isLoading = false

    private weak var connectionStore: ConnectionStore?
    private let logger = Logger(subsystem: "org.moltis.ios", category: "models")
    private let selectedModelKey = "selected_model_id"

    init(connectionStore: ConnectionStore) {
        self.connectionStore = connectionStore
        self.selectedModelId = UserDefaults.standard.string(forKey: selectedModelKey)
    }

    // MARK: - Load models

    func loadModels() async {
        guard let graphqlClient = connectionStore?.graphqlClient else { return }
        isLoading = true
        defer { isLoading = false }

        do {
            let gqlModels = try await graphqlClient.fetchModels()
            models = gqlModels.map { ModelInfo.from($0) }
        } catch {
            logger.error("Failed to load models: \(error.localizedDescription)")
        }
    }

    // MARK: - Set model

    func selectModel(id: String) async {
        selectedModelId = id
        guard let wsClient = connectionStore?.wsClient else { return }
        do {
            let params: [String: AnyCodable] = ["modelId": AnyCodable(id)]
            _ = try await wsClient.send(method: "models.set", params: params)
        } catch {
            logger.error("Failed to set model: \(error.localizedDescription)")
        }
    }

    /// Models grouped by provider.
    var modelsByProvider: [(provider: String, models: [ModelInfo])] {
        let grouped = Dictionary(grouping: models, by: \.provider)
        return grouped
            .sorted { $0.key < $1.key }
            .map { (provider: $0.key, models: $0.value) }
    }
}
