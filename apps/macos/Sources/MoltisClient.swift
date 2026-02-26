// swiftlint:disable file_length
import Foundation

// MARK: - Rust Bridge Log Forwarding

/// Decoded payload from Rust `emit_log` JSON.
private struct BridgeLogPayload: Decodable {
    let level: String
    let target: String
    let message: String
    let fields: [String: String]?
}

/// Global reference to the `LogStore` used by the Rust log callback.
/// Set once during app startup via `MoltisClient.installLogCallback`.
private var globalLogStore: LogStore?
private let logDecoder = JSONDecoder()

/// C-callable callback that receives Rust log events as JSON strings.
private func rustLogCallbackHandler(logJson: UnsafePointer<CChar>?) {
    guard let logJson else { return }
    let jsonString = String(cString: logJson)
    let data = Data(jsonString.utf8)

    guard let payload = try? logDecoder.decode(
        BridgeLogPayload.self, from: data
    ) else { return }

    let level: LogLevel
    switch payload.level {
    case "TRACE": level = .trace
    case "DEBUG": level = .debug
    case "INFO": level = .info
    case "WARN": level = .warn
    case "ERROR": level = .error
    default: level = .debug
    }

    DispatchQueue.main.async {
        globalLogStore?.log(
            level,
            target: payload.target,
            message: payload.message,
            fields: payload.fields ?? [:]
        )
    }
}

// MARK: - Rust Bridge Session Event Forwarding

/// Decoded payload from Rust session events.
struct BridgeSessionEventPayload: Decodable {
    let kind: String
    let sessionKey: String
}

/// Global reference to the `ChatStore` used by the Rust session event callback.
/// Set once during app startup via `MoltisClient.installSessionEventCallback`.
private var globalChatStore: ChatStore?
private let sessionEventDecoder = JSONDecoder()

/// C-callable callback that receives Rust session events as JSON strings.
private func rustSessionEventCallbackHandler(eventJson: UnsafePointer<CChar>?) {
    guard let eventJson else { return }
    let jsonString = String(cString: eventJson)
    let data = Data(jsonString.utf8)

    guard let payload = try? sessionEventDecoder.decode(
        BridgeSessionEventPayload.self, from: data
    ) else { return }

    DispatchQueue.main.async {
        globalChatStore?.handleSessionEvent(payload)
    }
}

// MARK: - Rust Bridge Network Audit Forwarding

/// Decoded payload from Rust `emit_network_audit` JSON.
private struct BridgeNetworkAuditPayload: Decodable {
    let domain: String
    let port: UInt16
    let networkProtocol: String
    let action: String
    let source: String
    let method: String?
    let url: String?

    private enum CodingKeys: String, CodingKey {
        case domain, port
        case networkProtocol = "protocol"
        case action, source, method, url
    }
}

/// Global reference to the `NetworkAuditStore` used by the Rust network audit callback.
/// Set once during app startup via `MoltisClient.installNetworkAuditCallback`.
private var globalNetworkAuditStore: NetworkAuditStore?
private let networkAuditDecoder = JSONDecoder()

/// C-callable callback that receives Rust network audit events as JSON strings.
private func rustNetworkAuditCallbackHandler(eventJson: UnsafePointer<CChar>?) {
    guard let eventJson else { return }
    let jsonString = String(cString: eventJson)
    let data = Data(jsonString.utf8)

    guard let payload = try? networkAuditDecoder.decode(
        BridgeNetworkAuditPayload.self, from: data
    ) else { return }

    DispatchQueue.main.async {
        let entry = NetworkAuditEntry(
            id: UUID(),
            timestamp: Date(),
            domain: payload.domain,
            port: payload.port,
            networkProtocol: payload.networkProtocol,
            action: payload.action,
            source: payload.source,
            method: payload.method,
            url: payload.url
        )
        globalNetworkAuditStore?.push(entry)
    }
}

// MARK: - Client Errors

enum MoltisClientError: Error, LocalizedError {
    case nilResponsePointer
    case jsonEncodingFailed
    case bridgeError(code: String, message: String)

    var errorDescription: String? {
        switch self {
        case .nilResponsePointer:
            return "Rust bridge returned a null response pointer"
        case .jsonEncodingFailed:
            return "Failed to encode Swift request into JSON"
        case let .bridgeError(code, message):
            return "Rust bridge error [\(code)]: \(message)"
        }
    }
}

// MARK: - Version

struct BridgeVersionPayload: Decodable {
    let bridgeVersion: String
    let moltisVersion: String
    let configDir: String
}

// MARK: - Identity

struct BridgeIdentityPayload: Decodable {
    let name: String
    let emoji: String?
    let theme: String?
    let soul: String?
    let userName: String?
}

// MARK: - Validation

struct BridgeValidationPayload: Decodable {
    let errors: Int
    let warnings: Int
    let info: Int
    let hasErrors: Bool
}

// MARK: - Chat

struct BridgeChatPayload: Decodable {
    let reply: String
    let model: String?
    let provider: String?
    let configDir: String
    let defaultSoul: String
    let validation: BridgeValidationPayload?
    let inputTokens: UInt32?
    let outputTokens: UInt32?
    let durationMs: UInt64?
}

// MARK: - Provider types

struct BridgeKnownProvider: Decodable, Identifiable {
    let name: String
    let displayName: String
    let authType: String
    let envKey: String?
    let defaultBaseUrl: String?
    let requiresModel: Bool
    let keyOptional: Bool

    var id: String { name }
}

struct BridgeDetectedSource: Decodable {
    let provider: String
    let source: String
}

struct BridgeModelInfo: Decodable, Identifiable {
    let id: String
    let provider: String
    let displayName: String
    let createdAt: Int?
}

// MARK: - HTTPD status

struct BridgeHttpdStatus: Decodable {
    let running: Bool
    let addr: String?
}

// MARK: - Session types

struct BridgeSessionEntry: Decodable {
    let key: String
    let label: String?
    let messageCount: UInt32
    let createdAt: UInt64
    let updatedAt: UInt64
    let preview: String?
}

struct BridgeSessionHistory: Decodable {
    let entry: BridgeSessionEntry
    let messages: [BridgePersistedMessage]
}

/// Represents a persisted message from the JSONL session store.
/// Uses a tagged union on "role" to match the Rust PersistedMessage enum.
struct BridgePersistedMessage: Decodable {
    let role: String
    let content: BridgeMessageContent?
    let createdAt: UInt64?
    let model: String?
    let provider: String?
    let inputTokens: UInt32?
    let outputTokens: UInt32?
    let durationMs: UInt64?

    private enum CodingKeys: String, CodingKey {
        case role, content, model, provider
        case createdAt = "created_at"
        case inputTokens, outputTokens, durationMs
    }

    /// Extract plain text from the content field (handles string or multimodal array).
    var textContent: String {
        guard let content else { return "" }
        switch content {
        case let .text(str):
            return str
        case let .multimodal(blocks):
            return blocks
                .compactMap { block in
                    if case let .text(blockText) = block { return blockText }
                    return nil
                }
                .joined(separator: "\n")
        }
    }
}

/// Content can be a plain string or multimodal array.
enum BridgeMessageContent: Decodable {
    case text(String)
    case multimodal([BridgeContentBlock])

    init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if let str = try? container.decode(String.self) {
            self = .text(str)
        } else if let blocks = try? container.decode([BridgeContentBlock].self) {
            self = .multimodal(blocks)
        } else {
            self = .text("")
        }
    }
}

enum BridgeContentBlock: Decodable {
    case text(String)
    case other

    private enum CodingKeys: String, CodingKey {
        case blockType = "type"
        case text
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let blockType = try container.decode(String.self, forKey: .blockType)
        if blockType == "text",
           let text = try container.decodeIfPresent(String.self, forKey: .text) {
            self = .text(text)
        } else {
            self = .other
        }
    }
}

// MARK: - Ok response

private struct BridgeOkPayload: Decodable {
    let ok: Bool
}

// MARK: - Error envelope

private struct BridgeErrorEnvelope: Decodable {
    let error: BridgeErrorPayload
}

private struct BridgeErrorPayload: Decodable {
    let code: String
    let message: String
}

// MARK: - Stream event

enum StreamEventType {
    case delta(text: String)
    case done(
        inputTokens: UInt32, outputTokens: UInt32, durationMs: UInt64,
        model: String?, provider: String?
    )
    case error(message: String)
}

private struct BridgeStreamEventPayload: Decodable {
    let eventType: String
    let text: String?
    let message: String?
    let inputTokens: UInt32?
    let outputTokens: UInt32?
    let durationMs: UInt64?
    let model: String?
    let provider: String?

    private enum CodingKeys: String, CodingKey {
        case eventType = "type"
        case text
        case message
        case inputTokens = "input_tokens"
        case outputTokens = "output_tokens"
        case durationMs = "duration_ms"
        case model
        case provider
    }
}

/// Holds the callback closure retained for the lifetime of one streaming call.
/// Retained via `Unmanaged.passRetained` and released on terminal events.
private final class StreamContext {
    let onEvent: (StreamEventType) -> Void
    let decoder: JSONDecoder

    init(onEvent: @escaping (StreamEventType) -> Void) {
        self.onEvent = onEvent
        let decoder = JSONDecoder()
        self.decoder = decoder
    }
}

/// C-callable callback that bridges from the Rust FFI into Swift closures.
private func streamCallbackHandler(
    eventJson: UnsafePointer<CChar>?,
    userData: UnsafeMutableRawPointer?
) {
    guard let eventJson, let userData else { return }

    let context = Unmanaged<StreamContext>.fromOpaque(userData)
        .takeUnretainedValue()

    let jsonString = String(cString: eventJson)
    let data = Data(jsonString.utf8)

    guard let payload = try? context.decoder.decode(
        BridgeStreamEventPayload.self, from: data
    ) else {
        let event = StreamEventType.error(message: "Failed to decode stream event")
        context.onEvent(event)
        Unmanaged<StreamContext>.fromOpaque(userData).release()
        return
    }

    let event: StreamEventType
    var isTerminal = false

    switch payload.eventType {
    case "delta":
        event = .delta(text: payload.text ?? "")
    case "done":
        event = .done(
            inputTokens: payload.inputTokens ?? 0,
            outputTokens: payload.outputTokens ?? 0,
            durationMs: payload.durationMs ?? 0,
            model: payload.model,
            provider: payload.provider
        )
        isTerminal = true
    case "error":
        event = .error(message: payload.message ?? "Unknown error")
        isTerminal = true
    default:
        return
    }

    context.onEvent(event)

    if isTerminal {
        Unmanaged<StreamContext>.fromOpaque(userData).release()
    }
}

// MARK: - Client

struct MoltisClient {
    /// Install the Rust→Swift log bridge. Call once at app startup.
    static func installLogCallback(logStore: LogStore) {
        globalLogStore = logStore
        moltis_set_log_callback(rustLogCallbackHandler)
    }

    /// Install the Rust→Swift session event bridge. Call once at app startup.
    static func installSessionEventCallback(chatStore: ChatStore) {
        globalChatStore = chatStore
        moltis_set_session_event_callback(rustSessionEventCallbackHandler)
    }

    /// Install the Rust→Swift network audit bridge. Call once at app startup.
    static func installNetworkAuditCallback(store: NetworkAuditStore) {
        globalNetworkAuditStore = store
        moltis_set_network_audit_callback(rustNetworkAuditCallbackHandler)
    }

    private let decoder: JSONDecoder = {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return decoder
    }()

    private let encoder: JSONEncoder = {
        let encoder = JSONEncoder()
        encoder.keyEncodingStrategy = .convertToSnakeCase
        return encoder
    }()

    func version() throws -> BridgeVersionPayload {
        let payload = try consumeCStringPointer(moltis_version())
        return try decode(payload, as: BridgeVersionPayload.self)
    }

    func getIdentity() throws -> BridgeIdentityPayload {
        let payload = try consumeCStringPointer(moltis_get_identity())
        return try decode(payload, as: BridgeIdentityPayload.self)
    }

    func chat(
        message: String,
        model: String? = nil,
        provider: String? = nil,
        configToml: String? = nil
    ) throws -> BridgeChatPayload {
        try callBridge(
            ChatRequest(
                message: message,
                model: model,
                provider: provider,
                configToml: configToml
            ),
            via: moltis_chat_json
        )
    }

    func knownProviders() throws -> [BridgeKnownProvider] {
        let payload = try consumeCStringPointer(moltis_known_providers())
        return try decode(payload, as: [BridgeKnownProvider].self)
    }

    func detectProviders() throws -> [BridgeDetectedSource] {
        let payload = try consumeCStringPointer(moltis_detect_providers())
        return try decode(payload, as: [BridgeDetectedSource].self)
    }

    func saveProviderConfig(
        provider: String,
        apiKey: String?,
        baseUrl: String?,
        models: [String]?
    ) throws {
        let _: BridgeOkPayload = try callBridge(
            SaveProviderRequest(
                provider: provider,
                apiKey: apiKey,
                baseUrl: baseUrl,
                models: models
            ),
            via: moltis_save_provider_config
        )
    }

    func listModels() throws -> [BridgeModelInfo] {
        let payload = try consumeCStringPointer(moltis_list_models())
        return try decode(payload, as: [BridgeModelInfo].self)
    }

    func refreshRegistry() throws {
        let payload = try consumeCStringPointer(moltis_refresh_registry())
        _ = try decode(payload, as: BridgeOkPayload.self)
    }

    func startHttpd(host: String, port: UInt16) throws -> BridgeHttpdStatus {
        try callBridge(
            StartHttpdRequest(host: host, port: port),
            via: moltis_start_httpd
        )
    }

    func stopHttpd() throws -> BridgeHttpdStatus {
        let payload = try consumeCStringPointer(moltis_stop_httpd())
        return try decode(payload, as: BridgeHttpdStatus.self)
    }

    func httpdStatus() throws -> BridgeHttpdStatus {
        let payload = try consumeCStringPointer(moltis_httpd_status())
        return try decode(payload, as: BridgeHttpdStatus.self)
    }

    // MARK: - Session operations

    func listSessions() throws -> [BridgeSessionEntry] {
        let payload = try consumeCStringPointer(moltis_list_sessions())
        return try decode(payload, as: [BridgeSessionEntry].self)
    }

    func switchSession(key: String) throws -> BridgeSessionHistory {
        try callBridge(
            SwitchSessionRequest(key: key),
            via: moltis_switch_session
        )
    }

    func createSession(label: String?) throws -> BridgeSessionEntry {
        try callBridge(
            CreateSessionRequest(label: label),
            via: moltis_create_session
        )
    }

    func sessionChatStream(
        sessionKey: String,
        message: String,
        model: String? = nil,
        onEvent: @escaping (StreamEventType) -> Void
    ) {
        let request = SessionChatRequest(
            sessionKey: sessionKey,
            message: message,
            model: model
        )
        guard let data = try? encoder.encode(request),
              let json = String(data: data, encoding: .utf8)
        else {
            onEvent(.error(message: "Failed to encode session chat request"))
            return
        }

        let context = StreamContext(onEvent: onEvent)
        let retained = Unmanaged.passRetained(context).toOpaque()

        json.withCString { ptr in
            moltis_session_chat_stream(ptr, streamCallbackHandler, retained)
        }
    }

    func chatStream(
        message: String,
        model: String? = nil,
        onEvent: @escaping (StreamEventType) -> Void
    ) {
        let request = ChatRequest(
            message: message,
            model: model,
            provider: nil,
            configToml: nil
        )
        guard let data = try? encoder.encode(request),
              let json = String(data: data, encoding: .utf8)
        else {
            onEvent(.error(message: "Failed to encode chat request"))
            return
        }

        let context = StreamContext(onEvent: onEvent)
        let retained = Unmanaged.passRetained(context).toOpaque()

        json.withCString { ptr in
            moltis_chat_stream(ptr, streamCallbackHandler, retained)
        }
    }

    // MARK: - Private helpers

    private func callBridge<Request: Encodable, Response: Decodable>(
        _ request: Request,
        via ffiCall: (UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar>?
    ) throws -> Response {
        let data = try encoder.encode(request)
        guard let json = String(data: data, encoding: .utf8) else {
            throw MoltisClientError.jsonEncodingFailed
        }
        let payload = try json.withCString { ptr in
            try consumeCStringPointer(ffiCall(ptr))
        }
        return try decode(payload, as: Response.self)
    }

    private func decode<T: Decodable>(_ payload: String, as _: T.Type) throws -> T {
        let data = Data(payload.utf8)

        // Check for bridge error envelope first (distinct shape with required
        // "error.code" + "error.message"). If present, surface it immediately.
        if let bridgeError = try? decoder.decode(BridgeErrorEnvelope.self, from: data) {
            throw MoltisClientError.bridgeError(
                code: bridgeError.error.code,
                message: bridgeError.error.message
            )
        }

        // Decode the expected type — any DecodingError propagates with full
        // context (field name, type mismatch, etc.) instead of being swallowed.
        return try decoder.decode(T.self, from: data)
    }

    private func consumeCStringPointer(
        _ value: UnsafeMutablePointer<CChar>?
    ) throws -> String {
        guard let value else {
            throw MoltisClientError.nilResponsePointer
        }

        defer {
            moltis_free_string(value)
        }

        return String(cString: value)
    }
}

// MARK: - Request types

private struct ChatRequest: Encodable {
    let message: String
    let model: String?
    let provider: String?
    let configToml: String?
}

private struct SaveProviderRequest: Encodable {
    let provider: String
    let apiKey: String?
    let baseUrl: String?
    let models: [String]?
}

private struct StartHttpdRequest: Encodable {
    let host: String
    let port: UInt16
}

private struct SwitchSessionRequest: Encodable {
    let key: String
}

private struct CreateSessionRequest: Encodable {
    let label: String?
}

private struct SessionChatRequest: Encodable {
    let sessionKey: String
    let message: String
    let model: String?
}
