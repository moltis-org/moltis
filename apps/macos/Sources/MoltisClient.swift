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

// MARK: - Abort / Peek types

struct BridgeAbortResult: Decodable {
    let aborted: Bool
    let runId: String?
}

struct BridgePeekResult: Decodable {
    let active: Bool
    let sessionKey: String?
    let thinkingText: String?
    let toolCalls: [BridgePeekToolCall]?
}

struct BridgePeekToolCall: Decodable, Identifiable {
    let id: String
    let name: String
    let startedAt: UInt64?
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

// MARK: - Config response (raw dictionary for round-tripping)

struct BridgeGetConfigResult {
    let config: [String: Any]
    let configDir: String
    let dataDir: String
}

struct BridgeGetConfigPayload: Decodable {
    let config: AnyCodable
    let configDir: String
    let dataDir: String
}

/// Wrapper to decode arbitrary JSON into `Any` (used for config round-trip).
struct AnyCodable: Decodable {
    let value: Any

    init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if let dict = try? container.decode([String: AnyCodable].self) {
            value = dict.mapValues { $0.value }
        } else if let array = try? container.decode([AnyCodable].self) {
            value = array.map { $0.value }
        } else if let string = try? container.decode(String.self) {
            value = string
        } else if let bool = try? container.decode(Bool.self) {
            value = bool
        } else if let int = try? container.decode(Int.self) {
            value = int
        } else if let double = try? container.decode(Double.self) {
            value = double
        } else if container.decodeNil() {
            value = NSNull()
        } else {
            value = NSNull()
        }
    }
}

// MARK: - Soul response

struct BridgeGetSoulPayload: Decodable {
    let soul: String?
}

// MARK: - Memory

struct BridgeMemoryStatusPayload: Decodable {
    let available: Bool
    let totalFiles: Int
    let totalChunks: Int
    let dbSize: UInt64
    let dbSizeDisplay: String
    let embeddingModel: String
    let hasEmbeddings: Bool
    let error: String?
}

struct BridgeMemoryConfigPayload: Decodable {
    let backend: String
    let citations: String
    let disableRag: Bool
    let llmReranking: Bool
    let sessionExport: Bool
    let qmdFeatureEnabled: Bool
}

struct BridgeMemoryQmdStatusPayload: Decodable {
    let featureEnabled: Bool
    let available: Bool
    let version: String?
    let error: String?
}

// MARK: - Auth

struct BridgeAuthStatusPayload: Decodable {
    let authDisabled: Bool
    let hasPassword: Bool
    let hasPasskeys: Bool
    let setupComplete: Bool
}

struct BridgeAuthPasswordChangePayload: Decodable {
    let ok: Bool
    let recoveryKey: String?
}

struct BridgeAuthPasskeyEntry: Decodable, Identifiable {
    let id: Int64
    let name: String
    let createdAt: String
}

private struct BridgeAuthPasskeysPayload: Decodable {
    let passkeys: [BridgeAuthPasskeyEntry]
}

// MARK: - Sandboxes

struct BridgeSandboxStatusPayload: Decodable {
    let backend: String
    let os: String
    let defaultImage: String
}

struct BridgeSandboxImageEntry: Decodable, Identifiable, Equatable {
    let tag: String
    let size: String
    let created: String
    let kind: String

    var id: String { tag }
}

private struct BridgeSandboxImagesPayload: Decodable {
    let images: [BridgeSandboxImageEntry]
}

private struct BridgeSandboxPrunePayload: Decodable {
    let pruned: Int
}

private struct BridgeSandboxCheckPackagesPayload: Decodable {
    let found: [String: Bool]
}

private struct BridgeSandboxBuildImagePayload: Decodable {
    let tag: String
}

private struct BridgeSandboxDefaultImagePayload: Decodable {
    let image: String
}

struct BridgeSandboxSharedHomePayload: Decodable {
    let enabled: Bool
    let mode: String
    let path: String
    let configuredPath: String?
}

private struct BridgeSandboxSharedHomeSavePayload: Decodable {
    let ok: Bool
    let restartRequired: Bool
    let configPath: String
    let config: BridgeSandboxSharedHomePayload
}

struct BridgeSandboxContainerEntry: Decodable, Identifiable, Equatable {
    let name: String
    let image: String
    let state: String
    let backend: String
    let cpus: UInt32?
    let memoryMb: UInt64?
    let started: String?
    let addr: String?

    var id: String { name }
}

private struct BridgeSandboxContainersPayload: Decodable {
    let containers: [BridgeSandboxContainerEntry]
}

private struct BridgeSandboxCleanContainersPayload: Decodable {
    let ok: Bool
    let removed: Int
}

struct BridgeSandboxDiskUsagePayload: Decodable, Equatable {
    let containersTotal: UInt64
    let containersActive: UInt64
    let containersSizeBytes: UInt64
    let containersReclaimableBytes: UInt64
    let imagesTotal: UInt64
    let imagesActive: UInt64
    let imagesSizeBytes: UInt64
}

private struct BridgeSandboxDiskUsageEnvelope: Decodable {
    let usage: BridgeSandboxDiskUsagePayload
}

// MARK: - Environment variables

struct BridgeEnvVarEntry: Decodable, Identifiable {
    let id: Int64
    let key: String
    let createdAt: String
    let updatedAt: String
    let encrypted: Bool
}

struct BridgeListEnvVarsPayload: Decodable {
    let envVars: [BridgeEnvVarEntry]
    let vaultStatus: String
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

    // MARK: - Abort / Peek

    func abortSession(key: String) throws -> BridgeAbortResult {
        let payload = try key.withCString { ptr in
            try consumeCStringPointer(moltis_abort_session(ptr))
        }
        return try decode(payload, as: BridgeAbortResult.self)
    }

    func peekSession(key: String) throws -> BridgePeekResult {
        let payload = try key.withCString { ptr in
            try consumeCStringPointer(moltis_peek_session(ptr))
        }
        return try decode(payload, as: BridgePeekResult.self)
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

// MARK: - Config / Identity / Soul

extension MoltisClient {
    /// Loads the full config as a raw dictionary, plus config_dir and data_dir.
    func getConfig() throws -> BridgeGetConfigResult {
        let payload = try consumeCStringPointer(moltis_get_config())
        let parsed = try decode(payload, as: BridgeGetConfigPayload.self)
        guard let dict = parsed.config.value as? [String: Any] else {
            throw MoltisClientError.bridgeError(
                code: "decode_error",
                message: "Config is not a JSON object"
            )
        }
        return BridgeGetConfigResult(
            config: dict,
            configDir: parsed.configDir,
            dataDir: parsed.dataDir
        )
    }

    /// Saves the full config from a raw dictionary. The Rust side deserializes
    /// to `MoltisConfig` and writes TOML preserving comments.
    func saveConfig(_ config: [String: Any]) throws {
        let data = try JSONSerialization.data(withJSONObject: config)
        guard let json = String(data: data, encoding: .utf8) else {
            throw MoltisClientError.jsonEncodingFailed
        }
        let payload = try json.withCString { ptr in
            try consumeCStringPointer(moltis_save_config(ptr))
        }
        _ = try decode(payload, as: BridgeOkPayload.self)
    }

    func memoryStatus() throws -> BridgeMemoryStatusPayload {
        let payload = try consumeCStringPointer(moltis_memory_status())
        return try decode(payload, as: BridgeMemoryStatusPayload.self)
    }

    func memoryConfigGet() throws -> BridgeMemoryConfigPayload {
        let payload = try consumeCStringPointer(moltis_memory_config_get())
        return try decode(payload, as: BridgeMemoryConfigPayload.self)
    }

    func memoryConfigUpdate(
        backend: String,
        citations: String,
        llmReranking: Bool,
        disableRag: Bool,
        sessionExport: Bool
    ) throws -> BridgeMemoryConfigPayload {
        let request = MemoryConfigUpdateRequest(
            backend: backend,
            citations: citations,
            llmReranking: llmReranking,
            disableRag: disableRag,
            sessionExport: sessionExport
        )
        return try callBridge(request, via: moltis_memory_config_update)
    }

    func memoryQmdStatus() throws -> BridgeMemoryQmdStatusPayload {
        let payload = try consumeCStringPointer(moltis_memory_qmd_status())
        return try decode(payload, as: BridgeMemoryQmdStatusPayload.self)
    }

    func authStatus() throws -> BridgeAuthStatusPayload {
        let payload = try consumeCStringPointer(moltis_auth_status())
        return try decode(payload, as: BridgeAuthStatusPayload.self)
    }

    func authPasswordChange(
        currentPassword: String?,
        newPassword: String
    ) throws -> BridgeAuthPasswordChangePayload {
        let request = AuthPasswordChangeRequest(
            currentPassword: currentPassword,
            newPassword: newPassword
        )
        return try callBridge(request, via: moltis_auth_password_change)
    }

    func authReset() throws {
        let payload = try consumeCStringPointer(moltis_auth_reset())
        _ = try decode(payload, as: BridgeOkPayload.self)
    }

    func authListPasskeys() throws -> [BridgeAuthPasskeyEntry] {
        let payload = try consumeCStringPointer(moltis_auth_list_passkeys())
        let parsed = try decode(payload, as: BridgeAuthPasskeysPayload.self)
        return parsed.passkeys
    }

    func authRemovePasskey(id: Int64) throws {
        let request = AuthPasskeyIdRequest(id: id)
        let _: BridgeOkPayload = try callBridge(request, via: moltis_auth_remove_passkey)
    }

    func authRenamePasskey(id: Int64, name: String) throws {
        let request = AuthPasskeyRenameRequest(id: id, name: name)
        let _: BridgeOkPayload = try callBridge(request, via: moltis_auth_rename_passkey)
    }

    func sandboxStatus() throws -> BridgeSandboxStatusPayload {
        let payload = try consumeCStringPointer(moltis_sandbox_status())
        return try decode(payload, as: BridgeSandboxStatusPayload.self)
    }

    func sandboxListImages() throws -> [BridgeSandboxImageEntry] {
        let payload = try consumeCStringPointer(moltis_sandbox_list_images())
        let parsed = try decode(payload, as: BridgeSandboxImagesPayload.self)
        return parsed.images
    }

    func sandboxDeleteImage(tag: String) throws {
        let request = SandboxDeleteImageRequest(tag: tag)
        let _: BridgeOkPayload = try callBridge(request, via: moltis_sandbox_delete_image)
    }

    func sandboxPruneImages() throws -> Int {
        let payload = try consumeCStringPointer(moltis_sandbox_prune_images())
        let parsed = try decode(payload, as: BridgeSandboxPrunePayload.self)
        return parsed.pruned
    }

    func sandboxCheckPackages(base: String, packages: [String]) throws -> [String: Bool] {
        let request = SandboxCheckPackagesRequest(base: base, packages: packages)
        let parsed: BridgeSandboxCheckPackagesPayload = try callBridge(
            request,
            via: moltis_sandbox_check_packages
        )
        return parsed.found
    }

    func sandboxBuildImage(name: String, base: String, packages: [String]) throws -> String {
        let request = SandboxBuildImageRequest(name: name, base: base, packages: packages)
        let parsed: BridgeSandboxBuildImagePayload = try callBridge(
            request,
            via: moltis_sandbox_build_image
        )
        return parsed.tag
    }

    func sandboxGetDefaultImage() throws -> String {
        let payload = try consumeCStringPointer(moltis_sandbox_get_default_image())
        let parsed = try decode(payload, as: BridgeSandboxDefaultImagePayload.self)
        return parsed.image
    }

    func sandboxSetDefaultImage(image: String?) throws -> String {
        let request = SandboxSetDefaultImageRequest(image: image)
        let parsed: BridgeSandboxDefaultImagePayload = try callBridge(
            request,
            via: moltis_sandbox_set_default_image
        )
        return parsed.image
    }

    func sandboxGetSharedHome() throws -> BridgeSandboxSharedHomePayload {
        let payload = try consumeCStringPointer(moltis_sandbox_get_shared_home())
        return try decode(payload, as: BridgeSandboxSharedHomePayload.self)
    }

    func sandboxSetSharedHome(enabled: Bool, path: String?) throws -> BridgeSandboxSharedHomePayload {
        let request = SandboxSetSharedHomeRequest(enabled: enabled, path: path)
        let parsed: BridgeSandboxSharedHomeSavePayload = try callBridge(
            request,
            via: moltis_sandbox_set_shared_home
        )
        _ = parsed.ok
        _ = parsed.restartRequired
        _ = parsed.configPath
        return parsed.config
    }

    func sandboxListContainers() throws -> [BridgeSandboxContainerEntry] {
        let payload = try consumeCStringPointer(moltis_sandbox_list_containers())
        let parsed = try decode(payload, as: BridgeSandboxContainersPayload.self)
        return parsed.containers
    }

    func sandboxStopContainer(name: String) throws {
        let request = SandboxContainerNameRequest(name: name)
        let _: BridgeOkPayload = try callBridge(request, via: moltis_sandbox_stop_container)
    }

    func sandboxRemoveContainer(name: String) throws {
        let request = SandboxContainerNameRequest(name: name)
        let _: BridgeOkPayload = try callBridge(request, via: moltis_sandbox_remove_container)
    }

    func sandboxCleanContainers() throws -> Int {
        let payload = try consumeCStringPointer(moltis_sandbox_clean_containers())
        let parsed = try decode(payload, as: BridgeSandboxCleanContainersPayload.self)
        _ = parsed.ok
        return parsed.removed
    }

    func sandboxDiskUsage() throws -> BridgeSandboxDiskUsagePayload {
        let payload = try consumeCStringPointer(moltis_sandbox_disk_usage())
        let parsed = try decode(payload, as: BridgeSandboxDiskUsageEnvelope.self)
        return parsed.usage
    }

    func sandboxRestartDaemon() throws {
        let payload = try consumeCStringPointer(moltis_sandbox_restart_daemon())
        _ = try decode(payload, as: BridgeOkPayload.self)
    }

    /// Returns the soul text from SOUL.md, or nil if empty/missing.
    func getSoul() throws -> String? {
        let payload = try consumeCStringPointer(moltis_get_soul())
        let parsed = try decode(payload, as: BridgeGetSoulPayload.self)
        return parsed.soul
    }

    /// Saves soul text to SOUL.md. Pass nil to clear.
    func saveSoul(_ text: String?) throws {
        let request: [String: Any?] = ["soul": text]
        let data = try JSONSerialization.data(
            withJSONObject: request.compactMapValues { $0 ?? NSNull() }
        )
        guard let json = String(data: data, encoding: .utf8) else {
            throw MoltisClientError.jsonEncodingFailed
        }
        let payload = try json.withCString { ptr in
            try consumeCStringPointer(moltis_save_soul(ptr))
        }
        _ = try decode(payload, as: BridgeOkPayload.self)
    }

    /// Saves identity (name, emoji, theme) to IDENTITY.md.
    func saveIdentity(name: String?, emoji: String?, theme: String?) throws {
        let request: [String: String?] = [
            "name": name,
            "emoji": emoji,
            "theme": theme
        ]
        let dict = request.compactMapValues { $0 }
        let data = try JSONSerialization.data(withJSONObject: dict)
        guard let json = String(data: data, encoding: .utf8) else {
            throw MoltisClientError.jsonEncodingFailed
        }
        let payload = try json.withCString { ptr in
            try consumeCStringPointer(moltis_save_identity(ptr))
        }
        _ = try decode(payload, as: BridgeOkPayload.self)
    }

    /// Saves user profile (name) to USER.md.
    func saveUserProfile(name: String?) throws {
        let request: [String: String?] = ["name": name]
        let dict = request.compactMapValues { $0 }
        let data = try JSONSerialization.data(withJSONObject: dict)
        guard let json = String(data: data, encoding: .utf8) else {
            throw MoltisClientError.jsonEncodingFailed
        }
        let payload = try json.withCString { ptr in
            try consumeCStringPointer(moltis_save_user_profile(ptr))
        }
        _ = try decode(payload, as: BridgeOkPayload.self)
    }
}

// MARK: - Environment Variables

extension MoltisClient {
    func listEnvVars() throws -> BridgeListEnvVarsPayload {
        let payload = try consumeCStringPointer(moltis_list_env_vars())
        return try decode(payload, as: BridgeListEnvVarsPayload.self)
    }

    func setEnvVar(key: String, value: String) throws {
        let request = SetEnvVarRequest(key: key, value: value)
        let _: BridgeOkPayload = try callBridge(request, via: moltis_set_env_var)
    }

    func deleteEnvVar(id: Int64) throws {
        let request = DeleteEnvVarRequest(id: id)
        let _: BridgeOkPayload = try callBridge(request, via: moltis_delete_env_var)
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

private struct SetEnvVarRequest: Encodable {
    let key: String
    let value: String
}

private struct MemoryConfigUpdateRequest: Encodable {
    let backend: String
    let citations: String
    let llmReranking: Bool
    let disableRag: Bool
    let sessionExport: Bool
}

private struct DeleteEnvVarRequest: Encodable {
    let id: Int64
}

private struct SandboxDeleteImageRequest: Encodable {
    let tag: String
}

private struct SandboxCheckPackagesRequest: Encodable {
    let base: String
    let packages: [String]
}

private struct SandboxBuildImageRequest: Encodable {
    let name: String
    let base: String
    let packages: [String]
}

private struct SandboxSetDefaultImageRequest: Encodable {
    let image: String?
}

private struct SandboxSetSharedHomeRequest: Encodable {
    let enabled: Bool
    let path: String?
}

private struct SandboxContainerNameRequest: Encodable {
    let name: String
}

private struct AuthPasswordChangeRequest: Encodable {
    let currentPassword: String?
    let newPassword: String
}

private struct AuthPasskeyIdRequest: Encodable {
    let id: Int64
}

private struct AuthPasskeyRenameRequest: Encodable {
    let id: Int64
    let name: String
}
