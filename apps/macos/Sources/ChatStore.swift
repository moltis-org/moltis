import Combine
import Foundation

final class ChatStore: ObservableObject {
    @Published private(set) var sessions: [ChatSession]
    @Published var selectedSessionID: UUID?
    @Published var draftMessage = ""
    @Published var isSending = false
    @Published var statusText = "Ready"
    @Published var bridgeSummary = "Bridge metadata not loaded"

    private let client: MoltisClient
    let settings: AppSettings
    let providerStore: ProviderStore

    init(
        client: MoltisClient = MoltisClient(),
        settings: AppSettings,
        providerStore: ProviderStore
    ) {
        self.client = client
        self.settings = settings
        self.providerStore = providerStore

        let initialSession = ChatSession(
            title: "Session 1",
            messages: [ChatMessage(role: .system, text: "Session started.")]
        )
        sessions = [initialSession]
        selectedSessionID = initialSession.id
    }

    var selectedSession: ChatSession? {
        guard let selectedSessionID else {
            return nil
        }
        return sessions.first(where: { $0.id == selectedSessionID })
    }

    var selectedMessageAnchorID: UUID? {
        selectedSession?.messages.last?.id
    }

    func createSession() {
        let nextNumber = sessions.count + 1
        let session = ChatSession(
            title: "Session \(nextNumber)",
            messages: [ChatMessage(role: .system, text: "Session started.")]
        )
        sessions.insert(session, at: 0)
        selectedSessionID = session.id
    }

    func loadVersion() {
        do {
            let version = try client.version()
            bridgeSummary = "Bridge \(version.bridgeVersion) - Moltis \(version.moltisVersion)"
            settings.environmentConfigDir = version.configDir
            statusText = "Loaded version and config directory."
            appendMessage(
                role: .system,
                text: "Using config dir: \(version.configDir)"
            )
        } catch {
            let text = error.localizedDescription
            statusText = text
            appendMessage(role: .error, text: text)
        }
    }

    func sendDraftMessage() {
        guard !isSending else {
            return
        }

        let trimmed = draftMessage.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            return
        }

        if selectedSessionID == nil {
            createSession()
        }

        appendMessage(role: .user, text: trimmed)
        updateSessionTitleIfNeeded(with: trimmed)
        draftMessage = ""

        let rawConfig = settings.configurationToml.trimmingCharacters(
            in: .whitespacesAndNewlines
        )
        let configToml: String? = rawConfig.isEmpty ? nil : rawConfig

        isSending = true
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self else { return }
            let result: Result<BridgeChatPayload, Error>
            do {
                let payload = try self.client.chat(
                    message: trimmed,
                    model: self.providerStore.selectedModelID,
                    configToml: configToml
                )
                result = .success(payload)
            } catch {
                result = .failure(error)
            }

            DispatchQueue.main.async {
                self.handleChatResult(result)
            }
        }
    }

    private func handleChatResult(_ result: Result<BridgeChatPayload, Error>) {
        switch result {
        case let .success(payload):
            settings.environmentConfigDir = payload.configDir
            settings.identitySoul = payload.defaultSoul

            if let model = payload.model, let provider = payload.provider {
                settings.llmModel = model
                settings.llmProvider = provider
            }

            appendMessage(
                role: .assistant,
                text: payload.reply,
                provider: payload.provider,
                model: payload.model,
                inputTokens: payload.inputTokens,
                outputTokens: payload.outputTokens,
                durationMs: payload.durationMs
            )
            appendValidationSummary(payload.validation)
            statusText = "Received response via \(payload.provider ?? "unknown")."

        case let .failure(error):
            let text = error.localizedDescription
            statusText = text
            appendMessage(role: .error, text: text)
        }
        isSending = false
    }

    private func appendValidationSummary(_ validation: BridgeValidationPayload?) {
        guard let validation else {
            return
        }

        let summary =
            "Validation: \(validation.errors) errors, \(validation.warnings) warnings, " +
            "\(validation.info) info."
        let role: ChatMessageRole = validation.hasErrors ? .error : .system
        appendMessage(role: role, text: summary)
    }

    private func appendMessage(
        role: ChatMessageRole,
        text: String,
        provider: String? = nil,
        model: String? = nil,
        inputTokens: UInt32? = nil,
        outputTokens: UInt32? = nil,
        durationMs: UInt64? = nil
    ) {
        guard let index = selectedSessionIndex() else {
            return
        }

        var session = sessions[index]
        session.messages.append(ChatMessage(
            role: role,
            text: text,
            provider: provider,
            model: model,
            inputTokens: inputTokens,
            outputTokens: outputTokens,
            durationMs: durationMs
        ))
        session.updatedAt = Date()
        sessions[index] = session
    }

    private func selectedSessionIndex() -> Int? {
        guard let selectedSessionID else {
            return nil
        }
        return sessions.firstIndex(where: { $0.id == selectedSessionID })
    }

    private func updateSessionTitleIfNeeded(with message: String) {
        guard let index = selectedSessionIndex() else {
            return
        }

        var session = sessions[index]
        guard session.title.hasPrefix("Session ") else {
            return
        }

        let compact = message
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .replacingOccurrences(of: "\n", with: " ")
        let shortTitle = String(compact.prefix(24))
        if !shortTitle.isEmpty {
            session.title = shortTitle
            sessions[index] = session
        }
    }
}
