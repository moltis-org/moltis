import Combine
import Foundation

final class ChatStore: ObservableObject {
    @Published private(set) var sessions: [ChatSession]
    @Published var selectedSessionID: UUID?
    @Published var draftMessage = ""
    @Published var isSending = false
    @Published private(set) var streamingMessageID: UUID?
    @Published var statusText = "Ready"
    @Published var bridgeSummary = "Bridge metadata not loaded"

    private let client: MoltisClient
    let settings: AppSettings
    let providerStore: ProviderStore
    private let logStore: LogStore?

    init(
        client: MoltisClient = MoltisClient(),
        settings: AppSettings,
        providerStore: ProviderStore,
        logStore: LogStore? = nil
    ) {
        self.client = client
        self.settings = settings
        self.providerStore = providerStore
        self.logStore = logStore

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
        logStore?.log(.info, target: "ChatStore", message: "Loading bridge version")
        do {
            let version = try client.version()
            bridgeSummary = "Bridge \(version.bridgeVersion) - Moltis \(version.moltisVersion)"
            settings.environmentConfigDir = version.configDir
            statusText = "Loaded version and config directory."
            appendMessage(
                role: .system,
                text: "Using config dir: \(version.configDir)"
            )
            logStore?.log(.info, target: "ChatStore", message: "Bridge loaded", fields: [
                "bridge": version.bridgeVersion,
                "moltis": version.moltisVersion,
                "configDir": version.configDir
            ])
        } catch {
            let text = error.localizedDescription
            statusText = text
            appendMessage(role: .error, text: text)
            logStore?.log(.error, target: "ChatStore", message: "Failed to load version: \(text)")
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

        // Create a streaming placeholder message
        let placeholderID = UUID()
        appendMessage(role: .assistant, text: "", id: placeholderID, isStreaming: true)
        streamingMessageID = placeholderID

        isSending = true
        statusText = "Thinking..."

        logStore?.log(.info, target: "ChatStore", message: "Sending message", fields: [
            "model": providerStore.selectedModelID ?? "default",
            "length": "\(trimmed.count)"
        ])

        client.chatStream(
            message: trimmed,
            model: providerStore.selectedModelID
        ) { [weak self] event in
            DispatchQueue.main.async {
                guard let self else { return }
                self.handleStreamEvent(event, placeholderID: placeholderID)
            }
        }
    }

    private func handleStreamEvent(_ event: StreamEventType, placeholderID: UUID) {
        guard let index = selectedSessionIndex(),
              let msgIndex = sessions[index].messages.firstIndex(where: {
                  $0.id == placeholderID
              })
        else { return }

        switch event {
        case let .delta(text):
            sessions[index].messages[msgIndex].text += text

        case let .done(inputTokens, outputTokens, durationMs, model, provider):
            sessions[index].messages[msgIndex].isStreaming = false
            sessions[index].messages[msgIndex].inputTokens = inputTokens
            sessions[index].messages[msgIndex].outputTokens = outputTokens
            sessions[index].messages[msgIndex].durationMs = durationMs
            sessions[index].messages[msgIndex].model = model
            sessions[index].messages[msgIndex].provider = provider
            sessions[index].updatedAt = Date()

            if let model, let provider {
                settings.llmModel = model
                settings.llmProvider = provider
            }

            streamingMessageID = nil
            isSending = false
            statusText = "Received response via \(provider ?? "unknown")."

            logStore?.log(.info, target: "ChatStore", message: "Stream completed", fields: [
                "provider": provider ?? "?",
                "model": model ?? "?",
                "inputTokens": "\(inputTokens)",
                "outputTokens": "\(outputTokens)",
                "durationMs": "\(durationMs)"
            ])

        case let .error(message):
            sessions[index].messages[msgIndex].isStreaming = false
            sessions[index].messages[msgIndex].text = message
            sessions[index].messages[msgIndex].role = .error
            sessions[index].updatedAt = Date()

            streamingMessageID = nil
            isSending = false
            statusText = message

            logStore?.log(.error, target: "ChatStore", message: "Stream error: \(message)")
        }
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
        id: UUID = UUID(),
        isStreaming: Bool = false,
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
            id: id,
            role: role,
            text: text,
            isStreaming: isStreaming,
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
