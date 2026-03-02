import Foundation

// MARK: - Chat event states

enum ChatEventState: String, Decodable {
    case thinking
    case thinkingText = "thinking_text"
    case thinkingDone = "thinking_done"
    case toolCallStart = "tool_call_start"
    case toolCallEnd = "tool_call_end"
    case delta
    case final_ = "final"
    case error
    case notice
    case retrying
    case autoCompact = "auto_compact"
    case sessionCleared = "session_cleared"
    case queueCleared = "queue_cleared"
    case aborted
    case voicePending = "voice_pending"
    case channelUser = "channel_user"
}

// MARK: - Chat event payload

struct ChatEventPayload: Decodable {
    let state: ChatEventState?
    let sessionKey: String?
    let runId: String?
    let text: String?
    let name: String?
    let input: AnyCodable?
    let output: AnyCodable?
    let toolCallId: String?
    let message: String?
    let title: String?
    let error: ChatEventError?
    // Token stats in final events
    let inputTokens: Int?
    let outputTokens: Int?
    let durationMs: Int?
    let model: String?
    let provider: String?

    struct ChatEventError: Decodable {
        let title: String?
        let message: String?
    }
}

extension ChatEventPayload {
    static let empty = ChatEventPayload(
        state: nil,
        sessionKey: nil,
        runId: nil,
        text: nil,
        name: nil,
        input: nil,
        output: nil,
        toolCallId: nil,
        message: nil,
        title: nil,
        error: nil,
        inputTokens: nil,
        outputTokens: nil,
        durationMs: nil,
        model: nil,
        provider: nil
    )
}

// MARK: - Tool call info

struct ToolCallInfo: Identifiable, Equatable {
    let id: String
    let name: String
    let displayLabel: String
    let icon: String
    let startedAt: Date
    var endedAt: Date?

    var isActive: Bool { endedAt == nil }

    static func == (lhs: ToolCallInfo, rhs: ToolCallInfo) -> Bool {
        lhs.id == rhs.id && lhs.endedAt == rhs.endedAt
    }

    /// Create a display label and icon from a tool call name and input.
    static func from(
        toolCallId: String,
        name: String,
        input: Any?
    ) -> ToolCallInfo {
        let inputDict = input as? [String: Any]
        let (label, icon) = displayInfo(name: name, input: inputDict)
        return ToolCallInfo(
            id: toolCallId,
            name: name,
            displayLabel: label,
            icon: icon,
            startedAt: Date()
        )
    }

    private static func displayInfo(
        name: String,
        input: [String: Any]?
    ) -> (String, String) {
        switch name.lowercased() {
        case "bash", "execute", "exec":
            let cmd = (input?["command"] as? String)?
                .components(separatedBy: "\n").first ?? ""
            let truncated = cmd.count > 40 ? String(cmd.prefix(40)) + "..." : cmd
            return ("Running: \(truncated)", "terminal")
        case "read", "read_file":
            let file = (input?["file_path"] as? String)?
                .components(separatedBy: "/").last ?? "file"
            return ("Reading: \(file)", "doc.text")
        case "write", "write_file", "edit":
            let file = (input?["file_path"] as? String)?
                .components(separatedBy: "/").last ?? "file"
            return ("Writing: \(file)", "doc.text")
        case "web_fetch", "browser", "browse":
            return ("Browsing...", "globe")
        case "web_search", "search":
            let query = (input?["query"] as? String) ?? ""
            let truncated = query.count > 30 ? String(query.prefix(30)) + "..." : query
            return ("Searching: \(truncated)", "magnifyingglass")
        case "glob", "grep":
            let pattern = (input?["pattern"] as? String) ?? ""
            return ("Searching: \(pattern)", "magnifyingglass")
        default:
            return (name, "gearshape")
        }
    }
}
