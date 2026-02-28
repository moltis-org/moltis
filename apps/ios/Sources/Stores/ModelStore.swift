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

    /// Set by the presenting view before showing the picker so `selectModel` knows
    /// which session to patch.
    var currentSessionKey: String = "main"

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
            logger.info("Loading models via GraphQL")
            let gqlModels = try await graphqlClient.fetchModels()
            let resolvedModels = gqlModels.compactMap(ModelInfo.from)
            let droppedCount = gqlModels.count - resolvedModels.count
            let providerCount = Set(resolvedModels.map(\.provider)).count
            models = resolvedModels

            if resolvedModels.isEmpty {
                logger.warning(
                    "Model list is empty (raw=\(gqlModels.count), dropped=\(droppedCount))"
                )
            } else {
                logger.info(
                    "Loaded \(resolvedModels.count) models from \(providerCount) providers (raw=\(gqlModels.count), dropped=\(droppedCount))"
                )
            }
        } catch {
            logger.error("Failed to load models: \(error.localizedDescription)")
        }
    }

    // MARK: - Set model

    func selectModel(id: String) async {
        selectedModelId = id
        guard let wsClient = connectionStore?.wsClient else { return }
        do {
            let params: [String: AnyCodable] = [
                "key": AnyCodable(currentSessionKey),
                "model": AnyCodable(id),
            ]
            _ = try await wsClient.send(method: "sessions.patch", params: params)
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
