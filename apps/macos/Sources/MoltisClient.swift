import Foundation

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

// MARK: - Client

struct MoltisClient {
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
