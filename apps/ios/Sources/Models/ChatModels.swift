import Foundation

// MARK: - Message role

enum ChatMessageRole: String, Codable {
    case user
    case assistant
    case system
    case error

    var title: String {
        switch self {
        case .user: return "You"
        case .assistant: return "Assistant"
        case .system: return "System"
        case .error: return "Error"
        }
    }
}

// MARK: - Chat message

struct ChatMessage: Identifiable, Equatable {
    let id: UUID
    var role: ChatMessageRole
    var text: String
    let createdAt: Date
    var isStreaming: Bool
    var provider: String?
    var model: String?
    var inputTokens: Int?
    var outputTokens: Int?
    var durationMs: Int?

    init(
        id: UUID = UUID(),
        role: ChatMessageRole,
        text: String,
        createdAt: Date = Date(),
        isStreaming: Bool = false,
        provider: String? = nil,
        model: String? = nil,
        inputTokens: Int? = nil,
        outputTokens: Int? = nil,
        durationMs: Int? = nil
    ) {
        self.id = id
        self.role = role
        self.text = text
        self.createdAt = createdAt
        self.isStreaming = isStreaming
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
    let key: String
    var title: String
    var preview: String?
    var updatedAt: Date
    var messageCount: Int
    var model: String?
    var archived: Bool

    init(
        id: UUID = UUID(),
        key: String = "main",
        title: String,
        preview: String? = nil,
        updatedAt: Date = Date(),
        messageCount: Int = 0,
        model: String? = nil,
        archived: Bool = false
    ) {
        self.id = id
        self.key = key
        self.title = title
        self.preview = preview
        self.updatedAt = updatedAt
        self.messageCount = messageCount
        self.model = model
        self.archived = archived
    }

    private static let dateFormatter: ISO8601DateFormatter = {
        let fmt = ISO8601DateFormatter()
        fmt.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return fmt
    }()

    /// Create from a GraphQL session response.
    static func from(_ gql: GQLSession) -> ChatSession {
        return ChatSession(
            key: gql.key,
            title: gql.label ?? gql.key,
            preview: gql.preview,
            updatedAt: gql.updatedAt.flatMap { dateFormatter.date(from: $0) } ?? Date(),
            messageCount: gql.messageCount ?? 0,
            model: gql.model,
            archived: gql.archived ?? false
        )
    }
}

// MARK: - Model info

struct ModelInfo: Identifiable, Equatable {
    let id: String
    let name: String
    let provider: String
    let tier: String?

    static func from(_ gql: GQLModel) -> ModelInfo? {
        guard let id = gql.id, !id.isEmpty,
              let name = gql.name, !name.isEmpty else {
            return nil
        }
        return ModelInfo(
            id: id,
            name: name,
            provider: gql.provider ?? "unknown",
            tier: gql.tier
        )
    }
}
