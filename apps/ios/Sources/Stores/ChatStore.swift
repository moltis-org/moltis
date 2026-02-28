import Foundation
import os

@MainActor
final class ChatStore: ObservableObject {
    @Published var messages: [ChatMessage] = []
    @Published var isStreaming = false
    @Published var activeToolCalls: [ToolCallInfo] = []
    @Published var currentSessionKey: String = "main"
    @Published var draftMessage = ""
    @Published var currentThinkingText: String?
    @Published var peekResult: PeekResult?

    private weak var connectionStore: ConnectionStore?
    private let logger = Logger(subsystem: "org.moltis.ios", category: "chat")
    private let liveActivityManager = LiveActivityManager.shared
    private var currentRunId: String?
    private var streamingMessageId: UUID?
    private var userMessageText: String?

    init(connectionStore: ConnectionStore) {
        self.connectionStore = connectionStore
    }

    // MARK: - Send message

    func sendMessage() async {
        let text = draftMessage.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }

        draftMessage = ""
        userMessageText = text

        // Add user message locally
        let userMessage = ChatMessage(role: .user, text: text)
        messages.append(userMessage)

        // Start Live Activity
        let agentName = connectionStore?.agentName ?? "Moltis"
        liveActivityManager.startActivity(agentName: agentName, userMessage: text)

        // Send via WebSocket
        guard let wsClient = connectionStore?.wsClient else { return }

        do {
            let params: [String: AnyCodable] = [
                "message": AnyCodable(text),
                "sessionKey": AnyCodable(currentSessionKey)
            ]
            _ = try await wsClient.send(method: "chat.send", params: params)
        } catch {
            logger.error("Failed to send message: \(error.localizedDescription)")
            messages.append(ChatMessage(role: .error, text: error.localizedDescription))
            liveActivityManager.endActivity(success: false)
        }
    }

    func abortGeneration() async {
        guard let wsClient = connectionStore?.wsClient else { return }
        do {
            let params: [String: AnyCodable] = [
                "sessionKey": AnyCodable(currentSessionKey)
            ]
            let response = try await wsClient.send(method: "chat.abort", params: params)
            if let payload = response.payload,
               let dict = payload.value as? [String: Any],
               dict["aborted"] as? Bool == true {
                finalizeAbort()
            }
        } catch {
            logger.error("Failed to abort: \(error.localizedDescription)")
        }
    }

    private func finalizeAbort() {
        if let msgId = streamingMessageId,
           let idx = messages.firstIndex(where: { $0.id == msgId }) {
            messages[idx].isStreaming = false
            if messages[idx].text.isEmpty {
                messages.remove(at: idx)
            }
        }
        isStreaming = false
        streamingMessageId = nil
        currentRunId = nil
        currentThinkingText = nil
        activeToolCalls.removeAll()
        liveActivityManager.endActivity(success: false)
    }

    // MARK: - Load history

    func loadHistory(for sessionKey: String) async {
        guard let wsClient = connectionStore?.wsClient else { return }
        do {
            let params: [String: AnyCodable] = [
                "sessionKey": AnyCodable(sessionKey)
            ]
            let response = try await wsClient.send(method: "chat.history", params: params)
            if let payload = response.payload,
               let historyArray = payload.value as? [[String: Any]] {
                messages = historyArray.compactMap { parseHistoryMessage($0) }
            }
        } catch {
            logger.error("Failed to load history: \(error.localizedDescription)")
        }
    }

    // MARK: - Event handling

    func handleChatEvent(_ payload: ChatEventPayload) {
        guard let state = payload.state else { return }

        switch state {
        case .thinking:
            currentRunId = payload.runId
            isStreaming = true
            // Create placeholder assistant message
            let msg = ChatMessage(
                role: .assistant, text: "", isStreaming: true,
                provider: payload.provider, model: payload.model
            )
            streamingMessageId = msg.id
            messages.append(msg)
            liveActivityManager.updateStep(
                label: "Thinking...", icon: "brain.head.profile", stepNumber: 1
            )

        case .thinkingText:
            currentThinkingText = payload.text
            liveActivityManager.updateStep(
                label: "Reasoning...", icon: "brain", stepNumber: 1
            )

        case .thinkingDone:
            break

        case .toolCallStart:
            guard let toolCallId = payload.toolCallId,
                  let name = payload.name else { return }
            let toolCall = ToolCallInfo.from(
                toolCallId: toolCallId,
                name: name,
                input: payload.input?.value
            )
            activeToolCalls.append(toolCall)
            let stepNum = (activeToolCalls.count) + 1
            liveActivityManager.updateStep(
                label: toolCall.displayLabel, icon: toolCall.icon, stepNumber: stepNum
            )

        case .toolCallEnd:
            if let toolCallId = payload.toolCallId,
               let idx = activeToolCalls.firstIndex(where: { $0.id == toolCallId }) {
                activeToolCalls[idx].endedAt = Date()
            }
            // Remove completed tool calls
            activeToolCalls.removeAll { $0.endedAt != nil }

        case .delta:
            if let text = payload.text, let msgId = streamingMessageId,
               let idx = messages.firstIndex(where: { $0.id == msgId }) {
                messages[idx].text += text
                if messages[idx].text.count == text.count {
                    // First delta — update Live Activity
                    let stepNum = max(activeToolCalls.count + 2, 2)
                    liveActivityManager.updateStep(
                        label: "Writing response...", icon: "text.cursor",
                        stepNumber: stepNum
                    )
                }
            }

        case .final_:
            if let msgId = streamingMessageId,
               let idx = messages.firstIndex(where: { $0.id == msgId }) {
                messages[idx].isStreaming = false
                messages[idx].inputTokens = payload.inputTokens
                messages[idx].outputTokens = payload.outputTokens
                messages[idx].durationMs = payload.durationMs
                messages[idx].model = payload.model ?? messages[idx].model
                messages[idx].provider = payload.provider ?? messages[idx].provider
            }
            isStreaming = false
            streamingMessageId = nil
            currentRunId = nil
            currentThinkingText = nil
            activeToolCalls.removeAll()
            liveActivityManager.endActivity(success: true)

        case .error:
            let errorMsg = payload.error?.message ?? payload.message ?? "Unknown error"
            if let msgId = streamingMessageId,
               let idx = messages.firstIndex(where: { $0.id == msgId }) {
                messages[idx].isStreaming = false
                messages[idx].text = errorMsg
                messages[idx].role = .error
            } else {
                messages.append(ChatMessage(role: .error, text: errorMsg))
            }
            isStreaming = false
            streamingMessageId = nil
            currentRunId = nil
            currentThinkingText = nil
            activeToolCalls.removeAll()
            liveActivityManager.endActivity(success: false)

        case .notice:
            if let title = payload.title, let message = payload.message {
                messages.append(ChatMessage(role: .system, text: "\(title): \(message)"))
            }

        case .retrying:
            liveActivityManager.updateStep(
                label: "Retrying...", icon: "arrow.clockwise", stepNumber: 1
            )

        case .sessionCleared:
            messages.removeAll()

        case .aborted:
            finalizeAbort()

        case .autoCompact, .queueCleared, .voicePending, .channelUser:
            break
        }
    }

    // MARK: - Switch session

    func switchSession(key: String) async {
        currentSessionKey = key
        messages.removeAll()
        isStreaming = false
        activeToolCalls.removeAll()
        streamingMessageId = nil
        currentRunId = nil

        guard let wsClient = connectionStore?.wsClient else { return }
        do {
            let params: [String: AnyCodable] = ["key": AnyCodable(key)]
            _ = try await wsClient.send(method: "sessions.switch", params: params)
            await loadHistory(for: key)
        } catch {
            logger.error("Failed to switch session: \(error.localizedDescription)")
        }
    }

    // MARK: - Peek

    func peekSession() async {
        guard let wsClient = connectionStore?.wsClient else { return }
        do {
            let params: [String: AnyCodable] = [
                "sessionKey": AnyCodable(currentSessionKey)
            ]
            let response = try await wsClient.send(method: "chat.peek", params: params)
            if let payload = response.payload,
               let dict = payload.value as? [String: Any] {
                let active = dict["active"] as? Bool ?? false
                if active {
                    let thinking = dict["thinkingText"] as? String
                    var toolNames: [String] = []
                    if let tools = dict["toolCalls"] as? [[String: Any]] {
                        toolNames = tools.compactMap { $0["name"] as? String }
                    }
                    peekResult = PeekResult(
                        active: true,
                        thinkingText: thinking,
                        toolCallNames: toolNames
                    )
                } else {
                    peekResult = PeekResult(active: false)
                }
            }
        } catch {
            logger.error("Failed to peek: \(error.localizedDescription)")
            peekResult = nil
        }
    }

    // MARK: - Private

    private func parseHistoryMessage(_ dict: [String: Any]) -> ChatMessage? {
        guard let roleStr = dict["role"] as? String,
              let role = ChatMessageRole(rawValue: roleStr),
              let text = dict["content"] as? String else {
            return nil
        }
        return ChatMessage(
            role: role,
            text: text,
            provider: dict["provider"] as? String,
            model: dict["model"] as? String,
            inputTokens: dict["inputTokens"] as? Int,
            outputTokens: dict["outputTokens"] as? Int,
            durationMs: dict["durationMs"] as? Int
        )
    }
}

struct PeekResult {
    let active: Bool
    var thinkingText: String?
    var toolCallNames: [String] = []
}
