import Combine
import Foundation

// MARK: - Voice Provider

struct VoiceProvider: Identifiable {
    let name: String
    let displayName: String
    let requiresApiKey: Bool

    var id: String { name }

    static let all: [VoiceProvider] = [
        VoiceProvider(name: "openai", displayName: "OpenAI TTS", requiresApiKey: true),
        VoiceProvider(name: "elevenlabs", displayName: "ElevenLabs", requiresApiKey: true),
        VoiceProvider(name: "google", displayName: "Google Cloud TTS", requiresApiKey: true),
        VoiceProvider(name: "piper", displayName: "Piper (Local)", requiresApiKey: false),
        VoiceProvider(name: "coqui", displayName: "Coqui (Local)", requiresApiKey: false)
    ]
}

// MARK: - Provider Store

final class ProviderStore: ObservableObject {
    @Published private(set) var knownProviders: [BridgeKnownProvider] = []
    @Published private(set) var detectedSources: [BridgeDetectedSource] = []
    @Published private(set) var models: [BridgeModelInfo] = []
    @Published private(set) var isLoadingModels = false

    @Published var selectedProviderName: String?
    @Published var selectedModelID: String?
    @Published var apiKeyDraft = ""
    @Published var baseUrlDraft = ""

    // Voice provider state
    @Published var selectedVoiceProviderName: String?
    @Published var voiceApiKeyDraft = ""

    private let client: MoltisClient
    private let logStore: LogStore?

    init(client: MoltisClient = MoltisClient(), logStore: LogStore? = nil) {
        self.client = client
        self.logStore = logStore
        loadAll()
    }

    // MARK: - Data loading

    func loadKnownProviders() {
        logStore?.log(.debug, target: "ProviderStore", message: "Loading known providers")
        do {
            knownProviders = try client.knownProviders()
            logStore?.log(.info, target: "ProviderStore", message: "Loaded known providers", fields: [
                "count": "\(knownProviders.count)"
            ])
        } catch {
            knownProviders = []
            let msg = "Failed to load providers: \(error.localizedDescription)"
            logStore?.log(.error, target: "ProviderStore", message: msg)
        }
    }

    func loadDetectedSources() {
        logStore?.log(.debug, target: "ProviderStore", message: "Detecting sources")
        do {
            detectedSources = try client.detectProviders()
            let providers = detectedSources.map(\.provider).joined(separator: ", ")
            logStore?.log(.info, target: "ProviderStore", message: "Detected sources", fields: [
                "count": "\(detectedSources.count)",
                "providers": providers
            ])
        } catch {
            detectedSources = []
            let msg = "Failed to detect sources: \(error.localizedDescription)"
            logStore?.log(.error, target: "ProviderStore", message: msg)
        }
    }

    func loadModels() {
        isLoadingModels = true
        logStore?.log(.debug, target: "ProviderStore", message: "Loading models")
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self else { return }
            let result: [BridgeModelInfo]
            do {
                result = try self.client.listModels()
            } catch {
                result = []
                DispatchQueue.main.async {
                    let msg = "Failed to load models: \(error.localizedDescription)"
                    self.logStore?.log(.error, target: "ProviderStore", message: msg)
                }
            }
            DispatchQueue.main.async {
                self.models = result.sorted { firstModel, secondModel in
                    let firstTime = firstModel.createdAt ?? 0
                    let secondTime = secondModel.createdAt ?? 0
                    if firstTime != secondTime { return firstTime > secondTime }
                    return firstModel.displayName.localizedCompare(secondModel.displayName)
                        == .orderedAscending
                }
                self.isLoadingModels = false
                self.logStore?.log(.info, target: "ProviderStore", message: "Loaded models", fields: [
                    "count": "\(result.count)"
                ])
            }
        }
    }

    func loadAll() {
        loadKnownProviders()
        loadDetectedSources()
        loadModels()
    }

    // MARK: - Save

    func saveCurrentProvider() throws {
        guard let name = selectedProviderName else { return }

        let key = apiKeyDraft.trimmingCharacters(in: .whitespacesAndNewlines)
        let url = baseUrlDraft.trimmingCharacters(in: .whitespacesAndNewlines)
        let modelList: [String]? = selectedModelID.map { [$0] }

        logStore?.log(.info, target: "ProviderStore", message: "Saving provider config", fields: [
            "provider": name,
            "hasKey": key.isEmpty ? "false" : "true",
            "hasUrl": url.isEmpty ? "false" : "true"
        ])

        try client.saveProviderConfig(
            provider: name,
            apiKey: key.isEmpty ? nil : key,
            baseUrl: url.isEmpty ? nil : url,
            models: modelList
        )

        try client.refreshRegistry()
        logStore?.log(.info, target: "ProviderStore", message: "Registry refreshed after save")
        loadDetectedSources()
        loadModels()
    }

    // MARK: - Queries

    func isConfigured(_ providerName: String) -> Bool {
        detectedSources.contains { $0.provider == providerName }
    }

    func modelsForProvider(_ providerName: String) -> [BridgeModelInfo] {
        models.filter { $0.provider == providerName }
    }

    /// The currently selected known provider, if any.
    var selectedKnownProvider: BridgeKnownProvider? {
        guard let name = selectedProviderName else { return nil }
        return knownProviders.first { $0.name == name }
    }

    /// Summary of the selected model for display in chat.
    var selectedModelSummary: String? {
        guard let modelID = selectedModelID else { return nil }
        if let info = models.first(where: { $0.id == modelID }) {
            return "\(info.displayName) (\(info.provider))"
        }
        return modelID
    }
}
