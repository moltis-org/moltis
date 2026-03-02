import Combine
import Foundation

// swiftlint:disable:next type_body_length
final class ChatStore: ObservableObject {
    @Published private(set) var sessions: [ChatSession] = []
    @Published var selectedSessionKey: String?
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
    }

    var selectedSession: ChatSession? {
        guard let selectedSessionKey else { return nil }
        return sessions.first(where: { $0.key == selectedSessionKey })
    }

    var selectedMessageAnchorID: UUID? {
        selectedSession?.messages.last?.id
    }

    // MARK: - Session lifecycle

    func loadSessions() {
        logStore?.log(.info, target: "ChatStore", message: "Loading sessions from disk")
        do {
            let entries = try client.listSessions()
            let existingByKey = Dictionary(
                uniqueKeysWithValues: sessions.map { ($0.key, $0) }
            )
            sessions = entries.map { entry in
                var session = ChatSession(
                    key: entry.key,
                    title: entry.label ?? entry.key,
                    updatedAt: Date(timeIntervalSince1970: Double(entry.updatedAt) / 1000),
                    messageCount: Int(entry.messageCount)
                )
                // Preserve already-loaded messages so they don't flash away.
                if let existing = existingByKey[entry.key] {
                    session.messages = existing.messages
                }
                return session
            }
            logStore?.log(.info, target: "ChatStore", message: "Loaded \(sessions.count) sessions")

            // Auto-select first session or create one if empty.
            if sessions.isEmpty {
                createSession()
            } else if selectedSessionKey == nil {
                let firstKey = sessions.first?.key
                selectedSessionKey = firstKey
                if let firstKey {
                    loadSessionHistory(key: firstKey)
                }
            }
        } catch {
            logStore?.log(.error, target: "ChatStore", message: "Failed to load sessions: \(error)")
            statusText = "Failed to load sessions"
        }
    }

    func switchSession(key: String) {
        guard key != selectedSessionKey else { return }
        selectedSessionKey = key
        loadSessionHistory(key: key)
    }

    func createSession() {
        let nextNumber = sessions.count + 1
        do {
            let entry = try client.createSession(label: "Session \(nextNumber)")
            let session = ChatSession(
                key: entry.key,
                title: entry.label ?? entry.key,
                updatedAt: Date(timeIntervalSince1970: Double(entry.updatedAt) / 1000),
                messageCount: 0
            )
            sessions.insert(session, at: 0)
            selectedSessionKey = session.key
            logStore?.log(.info, target: "ChatStore", message: "Created session '\(entry.key)'")
        } catch {
            logStore?.log(.error, target: "ChatStore", message: "Failed to create session: \(error)")
            statusText = "Failed to create session"
        }
    }

    private func loadSessionHistory(key: String) {
        do {
            let history = try client.switchSession(key: key)
            guard let index = sessions.firstIndex(where: { $0.key == key }) else { return }

            sessions[index].messages = history.messages.compactMap { msg in
                mapPersistedMessage(msg)
            }
            sessions[index].messageCount = Int(history.entry.messageCount)
            sessions[index].updatedAt = Date(
                timeIntervalSince1970: Double(history.entry.updatedAt) / 1000
            )
            if let label = history.entry.label {
                sessions[index].title = label
            }
            statusText = "Loaded \(history.messages.count) messages"
        } catch {
            logStore?.log(
                .error, target: "ChatStore",
                message: "Failed to load history for '\(key)': \(error)"
            )
        }
    }

    private func mapPersistedMessage(_ msg: BridgePersistedMessage) -> ChatMessage? {
        let role: ChatMessageRole
        switch msg.role {
        case "user": role = .user
        case "assistant": role = .assistant
        case "system": role = .system
        case "notice": role = .system
        default: return nil // skip tool/tool_result messages
        }

        let createdAt: Date
        if let ts = msg.createdAt {
            createdAt = Date(timeIntervalSince1970: Double(ts) / 1000)
        } else {
            createdAt = Date()
        }

        return ChatMessage(
            role: role,
            text: msg.textContent,
            createdAt: createdAt,
            provider: msg.provider,
            model: msg.model,
            inputTokens: msg.inputTokens,
            outputTokens: msg.outputTokens,
            durationMs: msg.durationMs
        )
    }

    // MARK: - Session event handling (cross-UI sync)

    /// Called from the Rust session event callback when the gateway or another
    /// UI creates, deletes, or patches a session.
    func handleSessionEvent(_ event: BridgeSessionEventPayload) {
        switch event.kind {
        case "created", "patched":
            // Reload the full session list so the sidebar stays in sync.
            loadSessions()

        case "deleted":
            sessions.removeAll { $0.key == event.sessionKey }
            // If the deleted session was selected, switch to the first available.
            if selectedSessionKey == event.sessionKey {
                selectedSessionKey = sessions.first?.key
                if let key = selectedSessionKey {
                    loadSessionHistory(key: key)
                }
            }

        default:
            logStore?.log(
                .debug, target: "ChatStore",
                message: "Unknown session event kind: \(event.kind)"
            )
        }
    }

    // MARK: - Identity

    func loadIdentity() {
        logStore?.log(.info, target: "ChatStore", message: "Loading identity from config")
        do {
            let identity = try client.getIdentity()
            settings.identityName = identity.name
            settings.identityEmoji = identity.emoji ?? ""
            settings.identityTheme = identity.theme ?? ""
            settings.identityUserName = identity.userName ?? ""
            settings.identitySoul = identity.soul ?? ""
            logStore?.log(.info, target: "ChatStore", message: "Identity loaded: \(identity.name)")
        } catch {
            logStore?.log(.error, target: "ChatStore", message: "Failed to load identity: \(error)")
        }
    }

    // MARK: - Version

    func loadVersion() {
        logStore?.log(.info, target: "ChatStore", message: "Loading bridge version")
        do {
            let version = try client.version()
            bridgeSummary = "Bridge \(version.bridgeVersion) - Moltis \(version.moltisVersion)"
            settings.environmentConfigDir = version.configDir
            statusText = "Loaded version and config directory."
            logStore?.log(.info, target: "ChatStore", message: "Bridge loaded", fields: [
                "bridge": version.bridgeVersion,
                "moltis": version.moltisVersion,
                "configDir": version.configDir
            ])
        } catch {
            let text = error.localizedDescription
            statusText = text
            logStore?.log(.error, target: "ChatStore", message: "Failed to load version: \(text)")
        }
    }

    // MARK: - Send message (session-backed)

    func sendDraftMessage() {
        guard !isSending else { return }

        let trimmed = draftMessage.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }

        // Ensure we have a session.
        if selectedSessionKey == nil {
            createSession()
        }
        guard let sessionKey = selectedSessionKey else { return }

        // Add user message to local state immediately for responsiveness.
        appendMessage(role: .user, text: trimmed)
        updateSessionTitleIfNeeded(with: trimmed)
        draftMessage = ""

        // Create a streaming placeholder.
        let placeholderID = UUID()
        appendMessage(role: .assistant, text: "", id: placeholderID, isStreaming: true)
        streamingMessageID = placeholderID

        isSending = true
        statusText = "Thinking..."

        logStore?.log(.info, target: "ChatStore", message: "Sending message", fields: [
            "session": sessionKey,
            "model": providerStore.selectedModelID ?? "default",
            "length": "\(trimmed.count)"
        ])

        // Use session-backed streaming — persists user + assistant messages to JSONL.
        client.sessionChatStream(
            sessionKey: sessionKey,
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
        guard let key = selectedSessionKey,
              let index = sessions.firstIndex(where: { $0.key == key }),
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
            sessions[index].messageCount += 2 // user + assistant

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

    // MARK: - Abort / Peek

    func abortGeneration() {
        guard isSending, let sessionKey = selectedSessionKey else { return }
        do {
            let result = try client.abortSession(key: sessionKey)
            if result.aborted {
                // Remove the streaming placeholder if it has no content.
                if let key = selectedSessionKey,
                   let sIdx = sessions.firstIndex(where: { $0.key == key }),
                   let placeholderID = streamingMessageID,
                   let mIdx = sessions[sIdx].messages.firstIndex(where: { $0.id == placeholderID }) {
                    let msg = sessions[sIdx].messages[mIdx]
                    if msg.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                        sessions[sIdx].messages.remove(at: mIdx)
                    } else {
                        sessions[sIdx].messages[mIdx].isStreaming = false
                    }
                }
                streamingMessageID = nil
                isSending = false
                statusText = "Stopped."
            }
        } catch {
            logStore?.log(.error, target: "ChatStore", message: "Abort failed: \(error)")
        }
    }

    @Published var peekResult: BridgePeekResult?

    func peekCurrentSession() {
        guard let sessionKey = selectedSessionKey else { return }
        do {
            let result = try client.peekSession(key: sessionKey)
            peekResult = result
        } catch {
            logStore?.log(.error, target: "ChatStore", message: "Peek failed: \(error)")
            peekResult = nil
        }
    }

    // MARK: - Helpers

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
        guard let key = selectedSessionKey,
              let index = sessions.firstIndex(where: { $0.key == key })
        else { return }

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

    private func updateSessionTitleIfNeeded(with message: String) {
        guard let key = selectedSessionKey,
              let index = sessions.firstIndex(where: { $0.key == key })
        else { return }

        var session = sessions[index]
        guard session.title.hasPrefix("Session ") || session.title == "New Session" else {
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
