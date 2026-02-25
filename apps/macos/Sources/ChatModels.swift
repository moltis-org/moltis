import AppKit
import Foundation

enum ChatMessageRole: String {
    case user
    case assistant
    case system
    case error

    var title: String {
        switch self {
        case .user:
            return "You"
        case .assistant:
            return "Assistant"
        case .system:
            return "System"
        case .error:
            return "Error"
        }
    }
}

// MARK: - Attachments

enum ChatAttachmentKind: Equatable {
    case image(NSImage)
    case file(url: URL)

    static func == (lhs: ChatAttachmentKind, rhs: ChatAttachmentKind) -> Bool {
        switch (lhs, rhs) {
        case let (.image(a), .image(b)):
            return a === b
        case let (.file(a), .file(b)):
            return a == b
        default:
            return false
        }
    }
}

struct ChatAttachment: Identifiable, Equatable {
    let id: UUID
    let name: String
    let kind: ChatAttachmentKind

    init(id: UUID = UUID(), name: String, kind: ChatAttachmentKind) {
        self.id = id
        self.name = name
        self.kind = kind
    }
}

// MARK: - Chat message

struct ChatMessage: Identifiable, Equatable {
    let id: UUID
    var role: ChatMessageRole
    var text: String
    let createdAt: Date
    var isStreaming: Bool
    var attachments: [ChatAttachment]
    var provider: String?
    var model: String?
    var inputTokens: UInt32?
    var outputTokens: UInt32?
    var durationMs: UInt64?

    init(
        id: UUID = UUID(),
        role: ChatMessageRole,
        text: String,
        createdAt: Date = Date(),
        isStreaming: Bool = false,
        attachments: [ChatAttachment] = [],
        provider: String? = nil,
        model: String? = nil,
        inputTokens: UInt32? = nil,
        outputTokens: UInt32? = nil,
        durationMs: UInt64? = nil
    ) {
        self.id = id
        self.role = role
        self.text = text
        self.createdAt = createdAt
        self.isStreaming = isStreaming
        self.attachments = attachments
        self.provider = provider
        self.model = model
        self.inputTokens = inputTokens
        self.outputTokens = outputTokens
        self.durationMs = durationMs
    }
}

// MARK: - Chat session

struct ChatSession: Identifiable, Equatable {
    let id: UUID
    var title: String
    var messages: [ChatMessage]
    var updatedAt: Date

    init(
        id: UUID = UUID(),
        title: String,
        messages: [ChatMessage] = [],
        updatedAt: Date = Date()
    ) {
        self.id = id
        self.title = title
        self.messages = messages
        self.updatedAt = updatedAt
    }

    var previewText: String {
        guard let lastMessage = messages.last else {
            return "No messages yet"
        }
        return lastMessage.text.replacingOccurrences(of: "\n", with: " ")
    }
}
